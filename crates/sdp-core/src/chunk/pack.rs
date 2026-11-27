//------------------------------------------------------------------
// 블록 문서 청킹 본체 (SimpleRAG chunk.rs 의 split_blocks · pack_units · peel_heading_only · split_unit · enforce_hard_max)
//=> 절(제목 경로로 묶은 블록들)을 토큰 예산 안에서 묶고, 넘치는 절은 단위(제목 · 블록 · 표 행)로 자른다.
//   표는 행 경계에서 자르고 조각마다 머리글 행을 다시 붙인다. 로직은 SimpleRAG 와 같다.
//------------------------------------------------------------------

use fancy_regex::Regex as FRegex;
use once_cell::sync::Lazy;

use super::assemble::{build_sections, make_prefix, section_units, Section, Unit};
use super::fixed::{chunk_ids, split_oversized, split_structured};
use super::pagefind::PageFinder;
use super::tok::Measure;
use super::{ChunkMeta, ChunkOpts};
use crate::docmodel::blocks::Document;
use crate::pyutil;

// 문장 끝(. ! ? 。) 뒤 공백 — sentence_split 에서 넘치는 줄을 문장으로 나눌 때
static SENT_END: Lazy<FRegex> = Lazy::new(|| FRegex::new(r"(?<=[.!?。])\s+").unwrap());
// 한국어 문장 끝까지(0.4.0~ sentence_endings = "ko") — 마침표 없는 "~다 · ~요 · ~음 · ~함" 뒤 공백에서도 나눈다
//   (DecideRAG 청커와 같은 경계. "다음 날" 처럼 낱말 속 "음" 뒤에서도 나뉠 수 있어 기본은 basic)
static SENT_END_KO: Lazy<FRegex> = Lazy::new(|| FRegex::new(r"(?<=[.!?。다요음함])\s+").unwrap());

//------------------------------------------------------------------
// 블록 문서 청킹 (핵심)
//=> 폴백 평문 문서는 split_structured 그대로(SimpleRAG legacy 와 같은 청크).
//    1) 절마다 단위 목록을 만들고, 접두 + 단위 전부가 예산 이하면 앞 절들과 이어 묶는다
//    2) 넘치면 모아 둔 것을 내보내되, 끝의 '들어오는 절의 상위 제목뿐인 절'은 떼어 다음 묶음으로 넘긴다
//    3) 혼자 예산을 넘는 절은 pack_units 로 자르고 조각마다 접두를 붙인다
//    4) 청크마다 PageFinder 로 실제 쪽을 찾고, hard_max 를 넘는 청크는 창으로 다시 자른다
//    5) min_tokens 미만 청크는 버린다
//
// -in: doc   = 추출 결과(절 조립이 블록 path 를 채우므로 &mut — 사본을 넘긴다)
// -in: m     = 길이 재기
// -in: o     = 옵션
// -in: title = 접두의 문서 제목(None 이면 doc.title)
//
// -out: Vec<ChunkMeta>
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn split_blocks(doc: &mut Document, m: &dyn Measure, o: &ChunkOpts, title: Option<&str>) -> Vec<ChunkMeta> {
    let title: String = title.map(|t| t.to_string()).unwrap_or_else(|| doc.title.clone());
    let size = o.tokens;
    let overlap = o.overlap;
    // 블록이 없는 폴백 평문 — 평문 구조 청킹
    if !doc.is_blocks() {
        let raw = doc.raw_text.clone().unwrap_or_default();
        let out: Vec<ChunkMeta> = split_structured(&raw, m, o, Some(&title), Some(size), Some(overlap)).into_iter()
            .map(|t| ChunkMeta::plain(t, "legacy")).collect();
        return finish_short(out, m, o);
    }
    let mut out: Vec<ChunkMeta> = vec![];
    let mut pf = PageFinder::new(o.prefix);
    let mut buf: Vec<String> = vec![];
    let mut buf_n = 0;
    let mut buf_secs: Vec<Section> = vec![];
    let title_opt = if title.is_empty() { None } else { Some(title.as_str()) };
    // 작은 표 통째(table_whole) — 표 글 길이가 기준 이하면 행으로 나누지 않는다
    let whole_max = o.table_whole;
    let keep = |t: &str| m.tlen(t) <= whole_max;
    let keep_whole: Option<&dyn Fn(&str) -> bool> = if whole_max != 0 { Some(&keep) } else { None };
    for sec in build_sections(doc) {
        let units = section_units(&sec, keep_whole);
        if units.is_empty() {
            continue;
        }
        // 접두를 끈 옵션이면 빈 글(SimpleRAG 는 늘 붙임)
        let prefix = if o.prefix { make_prefix(title_opt, &sec.path, o.prefix_max_chars) } else { String::new() };
        let whole = pyutil::strip(&format!("{}{}", prefix, units.iter().map(|u| u.0.as_str()).collect::<Vec<_>>().join("\n"))).to_string();
        let n = m.tlen(&whole);
        if n <= size {
            if buf_n + n > size {
                // 모아 둔 것 끝의 '제목만 든 절'은 앞 청크에 매달지 않고 이 절과 함께 새 묶음으로 넘긴다
                let tail = peel_heading_only(&mut buf, &mut buf_secs, &sec);
                if !buf.is_empty() {
                    out.push(chunk_meta(pyutil::strip(&buf.join("\n\n")).to_string(), &buf_secs, &mut pf));
                    buf.clear();
                    buf_secs.clear();
                }
                buf_n = 0;
                for (w, s) in tail {
                    buf_n += m.tlen(&w);
                    buf.push(w);
                    buf_secs.push(s);
                }
            }
            buf.push(whole);
            buf_secs.push(sec);
            buf_n += n;
            continue;
        }
        // 모아 둔 것 끝의 '제목만 든 절'은 홀로 내보내지 않고 이 절 첫 조각 앞에 제목 줄로 붙인다
        let tail = peel_heading_only(&mut buf, &mut buf_secs, &sec);
        if !buf.is_empty() {
            out.push(chunk_meta(pyutil::strip(&buf.join("\n\n")).to_string(), &buf_secs, &mut pf));
            buf.clear();
            buf_secs.clear();
        }
        buf_n = 0;
        let lead_lines: Vec<String> = tail.iter().map(|(_, s)| section_units(s, None)[0].0.clone()).collect();
        let lead = if lead_lines.is_empty() { None } else { Some(lead_lines.join("\n")) };
        // 접두 길이만큼 예산을 줄인다(최소 min_tokens)
        let budget = std::cmp::max(o.min_tokens, size.saturating_sub(m.tlen(&prefix)));
        for piece in pack_units(&units, m, o, budget, overlap, has_heading(&sec), lead) {
            if !pyutil::strip(&piece).is_empty() {
                out.push(chunk_meta(pyutil::strip(&format!("{}{}", prefix, piece)).to_string(), std::slice::from_ref(&sec), &mut pf));
            }
        }
    }
    if !buf.is_empty() {
        out.push(chunk_meta(pyutil::strip(&buf.join("\n\n")).to_string(), &buf_secs, &mut pf));
    }
    let out = enforce_hard_max(out, m, o, overlap);
    finish_short(out, m, o)
}

//------------------------------------------------------------------
// min_tokens 미만 청크 처리 — 버리기(기본) 또는 합치기(0.4.0~ short = "merge", 설계서 3.2)
//=> "제5조 경조금은 50만원으로 한다." 처럼 짧아도 답이 되는 조항이 버려지지 않게 한다.
//    1) 짧은 청크는 바로 앞 청크 뒤에 붙인다 — 합쳐도 hard_max 이하일 때
//    2) 앞에 못 붙이면 다음 청크 앞에 붙인다 — 역시 hard_max 이하일 때
//    3) 어느 쪽에도 못 붙이면 짧은 채로 남긴다(버리지 않는다)
//   붙일 때 같은 접두 줄("[문서 > 절]")이 두 번 들어가지 않게 뒤 청크의 같은 접두 줄은 뺀다.
//
// -in: chunks = 청크 목록(순서대로)
// -in: m      = 길이 재기
// -in: o      = 옵션(short · min_tokens · hard_max)
//
// -out: 처리한 청크 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn finish_short(chunks: Vec<ChunkMeta>, m: &dyn Measure, o: &ChunkOpts) -> Vec<ChunkMeta> {
    if o.short != "merge" {
        return chunks.into_iter().filter(|c| m.tlen(&c.text) >= o.min_tokens).collect();
    }
    let mut out: Vec<ChunkMeta> = vec![];
    // 앞에 못 붙여 다음 청크를 기다리는 짧은 청크
    let mut pending: Option<ChunkMeta> = None;
    for c in chunks {
        if pyutil::strip(&c.text).is_empty() {
            continue;
        }
        let short = m.tlen(&c.text) < o.min_tokens;
        if let Some(p) = pending.take() {
            // 기다리던 짧은 청크를 이 청크 앞에
            let text = join_chunks(&p.text, &c.text);
            if m.tlen(&text) <= o.hard_max {
                let mut x = p;
                x.text = text;
                x.page_to = max_opt(x.page_to, c.page_to);
                x.page_from = min_opt(x.page_from, c.page_from);
                // 합친 결과도 짧으면 다시 기다린다
                if m.tlen(&x.text) < o.min_tokens {
                    pending = Some(x);
                } else {
                    out.push(x);
                }
                continue;
            }
            out.push(p);
        }
        if !short {
            out.push(c);
            continue;
        }
        // 1) 앞 청크 뒤에
        if let Some(prev) = out.last_mut() {
            let text = join_chunks(&prev.text, &c.text);
            if m.tlen(&text) <= o.hard_max {
                prev.text = text;
                prev.page_to = max_opt(prev.page_to, c.page_to);
                prev.page_from = min_opt(prev.page_from, c.page_from);
                continue;
            }
        }
        // 2) 다음 청크를 기다린다
        pending = Some(c);
    }
    // 3) 끝까지 못 붙인 것은 짧은 채로 남긴다
    if let Some(p) = pending {
        out.push(p);
    }
    out
}

// 두 청크 글 잇기 — 뒤 청크 첫 줄이 앞 청크 첫 줄(접두)과 같으면 그 줄은 뺀다
fn join_chunks(a: &str, b: &str) -> String {
    let a_first = a.split('\n').next().unwrap_or("");
    let (b_first, b_rest) = match b.split_once('\n') {
        Some((f, r)) => (f, r),
        None => (b, ""),
    };
    let is_prefix = a_first.starts_with('[') && a_first.ends_with(']');
    if is_prefix && b_first == a_first && !pyutil::strip(b_rest).is_empty() {
        return format!("{}\n{}", a, b_rest);
    }
    format!("{}\n\n{}", a, b)
}

// 둘 중 큰 쪽 번호(없으면 있는 쪽)
fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

// 둘 중 작은 쪽 번호(없으면 있는 쪽)
fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

// 절 제목이 단위로 들어가는 절인가 — section_units 와 같은 조건(합성 아님 · 글자 있음)
fn has_heading(sec: &Section) -> bool {
    sec.heading.as_ref().map(|h| !h.synthetic && !pyutil::strip(&h.text).is_empty()).unwrap_or(false)
}

// 제목만 든 절인가 — 제목 바로 뒤에 하위 제목이 와서 본문 단위가 하나도 없는 절
fn heading_only(sec: &Section) -> bool {
    has_heading(sec) && section_units(sec, None).len() == 1
}

// 상위 절인가 — 위 절 경로가 아래 절 경로의 앞부분이고 아래 절이 더 깊다
fn is_ancestor(upper: &Section, lower: &Section) -> bool {
    let k = upper.path.len();
    lower.path.len() > k && lower.path[..k] == upper.path[..]
}

// 모아 둔 절 끝에서 '들어오는 절(nxt)의 상위 제목뿐인 절'들을 떼어 낸다 — (whole, 절) 목록, 순서 유지
//   상위 제목인지 꼭 본다 — 번호 조항 문서는 제목 줄이 곧 내용인 절이 줄줄이 이어진다(Python peel 과 같다)
fn peel_heading_only(buf: &mut Vec<String>, secs: &mut Vec<Section>, nxt: &Section) -> Vec<(String, Section)> {
    let mut k = secs.len();
    while k > 0 && heading_only(&secs[k - 1]) && is_ancestor(&secs[k - 1], nxt) {
        k -= 1;
    }
    buf.split_off(k).into_iter().zip(secs.split_off(k)).collect()
}

// 청크 출처 정보 — 첫 절 기준. 쪽은 청크가 절 글의 어디에 있는지로 찾는다(탐색기 모드 설계서 §13)
fn chunk_meta(text: String, secs: &[Section], pf: &mut PageFinder) -> ChunkMeta {
    let first = &secs[0];
    let pages: Vec<i64> = secs.iter().flat_map(|s| [s.page_from, s.page_to]).flatten().collect();
    let src = first.heading.as_ref().map(|h| h.source).unwrap_or("parser");
    let (page, from, to) = match pf.locate(&text, secs) {
        // 청크 자리를 찾았으면 쪽 범위를 그 자리로 좁힌다
        Some((p, f, t)) => (Some(p), Some(f), Some(t)),
        // 못 찾으면 예전처럼 절 범위(넓지만 틀리지는 않음)
        None => (None, pages.iter().min().copied(), pages.iter().max().copied()),
    };
    ChunkMeta {
        text,
        section_id: Some(first.id),
        path: first.path.clone(),
        source: src.to_string(),
        page_from: from,
        page_to: to,
        page,
    }
}

//------------------------------------------------------------------
// 절 단위들을 예산 안에서 묶기
//=> 표 머리글 단위가 조각 끝에 홀로 남지 않게 미리 끊고, 값 행 조각이 새로 시작하면 머리글을 붙인다.
//   예산을 넘는 단위는 약한 초과(α) 안이면 통째로, 아니면 줄·토큰으로 나눈다.
//   제목 줄만 든 조각은 끊지 않는다 — 지금 조각에 절 제목(과 넘겨받은 상위 제목, 그 뒤의 표 머리글)만
//   들었으면 '머리 상태'로 보고 다음 단위가 예산을 넘겨도 함께 담는다. 다음 단위가 혼자 예산을 넘어
//   쪼개질 때는 머리 줄을 그 첫 조각 맨 앞에 붙인다(Python chunk._pack_units 와 같다).
//
// -in: units   = 절 단위(글, 머리글, 통째 표인가)
// -in: m       = 길이 재기
// -in: o       = 옵션
// -in: budget  = 조각 예산(접두 길이를 뺀 값)
// -in: overlap = 창 겹침
// -in: head    = units[0] 이 절 제목인가
// -in: lead    = 첫 조각 맨 앞에 둘 제목 줄(앞의 '제목만 든 상위 절')
//
// -out: 조각 글 목록(접두 없음)
// -out: error = 예외 없음
//------------------------------------------------------------------
fn pack_units(units: &[Unit], m: &dyn Measure, o: &ChunkOpts, budget: usize, overlap: usize, head: bool, lead: Option<String>) -> Vec<String> {
    let mut pieces: Vec<String> = vec![];
    let mut cur: Vec<String> = vec![];
    let mut cur_n = 0usize;
    // 지금 조각이 제목(·표 머리글) 줄뿐인가
    let mut head_only = false;
    if let Some(l) = lead {
        cur_n = m.tlen(&l);
        cur.push(l);
        head_only = true;
    }
    let soft = budget as f64 * o.overflow_ratio.max(1.0);
    // 0.4.0~ 직전 짧은 줄 겹치기(carry_short_line_chars, 설계서 3.3) — 크기 때문에 끊을 때 끊긴 조각의
    //   마지막 줄이 짧으면("(단위 : 원)" · 조항 머리 문장) 다음 조각 앞에도 넣는다. 0 이면 끔
    let carry_max = o.carry_short_line_chars;
    // 다음 조각 앞에 넣을 수 있는 마지막 줄(표 행 · 표 머리글 · 통째 표는 아님)
    let mut carry: Option<String> = None;
    // 지금 조각이 넘겨받은 줄 하나뿐인가 — 그 줄만으로 조각을 내면 앞 조각과 겹치기만 하므로 내지 않는다
    let mut carried_only = false;
    // 지금 조각을 끊어 낸다
    fn emit(cur: &mut Vec<String>, pieces: &mut Vec<String>, cur_n: &mut usize, head_only: &mut bool) {
        if !cur.is_empty() {
            pieces.push(cur.join("\n"));
            cur.clear();
        }
        *cur_n = 0;
        *head_only = false;
    }
    for (i, (text, header, whole)) in units.iter().enumerate() {
        let n = m.tlen(text);
        let hdr = header.as_ref().filter(|h| m.tlen(h) <= o.table_header_max_tokens).cloned();
        let nxt = units.get(i + 1);
        let is_hdr_unit = nxt.map(|(_, nh, _)| nh.as_deref() == Some(text.as_str())).unwrap_or(false);
        // 표 머리글이 첫 행과 함께 들어갈 자리가 없으면 먼저 끊는다 — 단 지금 조각이 제목뿐이면 끊지 않는다
        if !cur.is_empty() && !head_only && is_hdr_unit {
            if let Some((nt, _, _)) = nxt {
                if cur_n + n + m.tlen(nt) > budget {
                    if carried_only {
                        // 넘겨받은 줄뿐이면 내지 않고 버린다(앞 조각에 이미 있다)
                        cur.clear();
                        cur_n = 0;
                        carried_only = false;
                    } else {
                        let last = take_carry(&mut carry, &cur);
                        emit(&mut cur, &mut pieces, &mut cur_n, &mut head_only);
                        // 머리글 + 첫 행과 함께 들어갈 자리가 있을 때만 넘긴다
                        if let Some(l) = last.filter(|l| m.tlen(l) + n + m.tlen(nt) <= budget) {
                            cur_n = m.tlen(&l);
                            cur.push(l);
                            carried_only = true;
                        }
                    }
                }
            }
        }
        if n > budget {
            // 제목 줄뿐인 조각은 따로 내지 않고 이 단위 첫 조각 앞에 붙인다
            let mut pre: Vec<String> = vec![];
            // 넘겨받은 줄뿐인 조각은 내지 않는다(큰 단위는 따로 잘리므로 넘길 자리가 없다)
            if carried_only {
                cur.clear();
                cur_n = 0;
                carried_only = false;
            }
            carry = None;
            if head_only {
                pre = std::mem::take(&mut cur);
                // 끝이 이 행의 표 머리글이면 아래에서 다시 붙으므로 뺀다(두 번 들어가지 않게)
                if let (Some(h), Some(last)) = (&hdr, pre.last()) {
                    if last == h {
                        pre.pop();
                    }
                }
                cur_n = 0;
                head_only = false;
            } else {
                emit(&mut cur, &mut pieces, &mut cur_n, &mut head_only);
            }
            // 통째 표는 약한 초과(α)를 넘어도 hard_max 안이면 쪼개지 않는다
            let mut subs = if (n as f64 <= soft || *whole) && n <= o.hard_max {
                vec![match &hdr { Some(h) => format!("{}\n{}", h, text), None => text.clone() }]
            } else {
                let mut subs = split_unit(text, m, o, budget, overlap);
                if let (Some(h), Some(first)) = (&hdr, subs.first_mut()) {
                    *first = format!("{}\n{}", h, first);
                }
                subs
            };
            if !pre.is_empty() {
                match subs.first_mut() {
                    Some(first) => *first = format!("{}\n{}", pre.join("\n"), first),
                    None => pieces.push(pre.join("\n")),
                }
            }
            pieces.extend(subs);
            continue;
        }
        if cur_n + n > budget && !head_only {
            if carried_only {
                // 넘겨받은 줄과 이 단위가 함께 들어가지 않으면 넘겨받은 줄은 버린다
                cur.clear();
                cur_n = 0;
                carried_only = false;
            } else {
                let last = take_carry(&mut carry, &cur);
                emit(&mut cur, &mut pieces, &mut cur_n, &mut head_only);
                // 머리글(값 행이면) + 이 단위와 함께 들어갈 자리가 있을 때만 넘긴다
                let need = n + hdr.as_ref().map(|h| m.tlen(h)).unwrap_or(0);
                if let Some(l) = last.filter(|l| m.tlen(l) + need <= budget) {
                    cur_n = m.tlen(&l);
                    cur.push(l);
                    carried_only = true;
                }
            }
        }
        let was_head_only = head_only && !cur.is_empty();
        // 새 조각이 값 행으로 시작하면 그 표의 머리글을 먼저 넣는다(넘겨받은 줄 뒤에)
        if cur.is_empty() || carried_only {
            if let Some(h) = &hdr {
                cur_n += if carried_only { m.tlen(h) } else { 0 };
                cur.push(h.clone());
            }
        }
        cur.push(text.clone());
        cur_n += n;
        carried_only = false;
        // 이 줄이 짧은 보통 줄이면 다음에 끊을 때 넘길 후보
        carry = if carry_max > 0 && header.is_none() && !*whole && !is_hdr_unit && pyutil::clen(text) < carry_max {
            Some(text.clone())
        } else {
            None
        };
        // 머리 상태가 이어지는 경우: 절 제목(첫 단위)이거나, 머리 상태에 표 머리글 단위가 더해질 때
        head_only = (head && i == 0) || (was_head_only && is_hdr_unit);
    }
    // 끝에 넘겨받은 줄만 남았으면 내지 않는다
    if carried_only {
        cur.clear();
    }
    emit(&mut cur, &mut pieces, &mut cur_n, &mut head_only);
    pieces
}

//------------------------------------------------------------------
// 끊을 때 다음 조각으로 넘길 줄 꺼내기 (carry_short_line_chars)
//=> 후보가 지금 조각의 마지막 줄이고 조각에 줄이 둘 이상일 때만 넘긴다 — 한 줄짜리 조각을 넘기면
//   그 조각 전체가 다음 조각에 또 들어간다.
//
// -in: carry = 후보(꺼내면 비운다)
// -in: cur   = 끊기 직전의 지금 조각
//
// -out: Some(넘길 줄) / None
// -out: error = 예외 없음
//------------------------------------------------------------------
fn take_carry(carry: &mut Option<String>, cur: &[String]) -> Option<String> {
    let c = carry.take()?;
    if cur.len() >= 2 && cur.last() == Some(&c) {
        Some(c)
    } else {
        None
    }
}

//------------------------------------------------------------------
// 큰 단위 하나 나누기 — sentence_split 이면 넘치는 줄을 문장 경계로도 나눈다
//
// -in: text = 단위 글 / m = 길이 재기 / o = 옵션 / budget = 예산 / overlap = 창 겹침
//
// -out: 조각 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
fn split_unit(text: &str, m: &dyn Measure, o: &ChunkOpts, budget: usize, overlap: usize) -> Vec<String> {
    if !o.sentence_split {
        return split_oversized(text, m, o, budget, overlap);
    }
    let mut lines: Vec<String> = vec![];
    for ln in text.split('\n') {
        // 예산을 넘는 줄만 문장으로 쪼갠다
        if !pyutil::strip(ln).is_empty() && m.tlen(ln) > budget {
            let mut last = 0;
            // 문장 끝 규칙 — ko 면 마침표 없는 한국어 종결 어미 뒤에서도
            let re: &FRegex = if o.sentence_endings == "ko" { &SENT_END_KO } else { &SENT_END };
            for mt in re.find_iter(ln).filter_map(|x| x.ok()) {
                let s = &ln[last..mt.start()];
                if !pyutil::strip(s).is_empty() {
                    lines.push(s.to_string());
                }
                last = mt.end();
            }
            let s = &ln[last..];
            if !pyutil::strip(s).is_empty() {
                lines.push(s.to_string());
            }
        } else {
            lines.push(ln.to_string());
        }
    }
    split_oversized(&lines.join("\n"), m, o, budget, overlap)
}

//------------------------------------------------------------------
// 절대 상한(e5 입력 한계)을 넘는 청크는 창으로 다시 자른다 — 출처 정보는 그대로 물려준다
//
// -in: chunks = 청크 목록 / m = 길이 재기 / o = 옵션(hard_max · min_tokens) / overlap = 창 겹침
//
// -out: 상한 이하 청크 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
fn enforce_hard_max(chunks: Vec<ChunkMeta>, m: &dyn Measure, o: &ChunkOpts, overlap: usize) -> Vec<ChunkMeta> {
    let hard = o.hard_max;
    let mut out = vec![];
    for c in chunks {
        if m.tlen(&c.text) <= hard {
            out.push(c);
            continue;
        }
        let ids = m.ids(&c.text);
        for piece in chunk_ids(&ids, hard, overlap, o.min_tokens) {
            let mut x = c.clone();
            x.text = m.decode(&piece);
            out.push(x);
        }
    }
    out
}
