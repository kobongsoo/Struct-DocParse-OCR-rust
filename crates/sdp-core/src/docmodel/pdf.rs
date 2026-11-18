//------------------------------------------------------------------
// PDF(텍스트층) → 블록 (Python docmodel/pdf.py)
//    · 쪽 경계 — 블록마다 page
//    · 머리말·꼬리말 — 쪽 맨 위·아래 2줄 가운데 3쪽 이상에서 되풀이되는 줄(숫자는 '#')
//    · 쪽번호 — "12" · "- 12 -" · "3 / 30" · "Page 3" 꼴
//    · 제목(P-2, 설정으로 켤 때만) — 본문보다 확연히 큰 짧은 줄
//    · 표(extract.pdf_tables, 기본 켬) — 테두리 선으로 표를 찾아(pdftable) TABLE 블록으로 두고
//      그 글자는 본문 줄에서 뺀다. 표 블록은 표가 시작된 줄 자리에 들어간다
//    · 본문 밖 글(extra_text, 기본 켬) — 글상자 주석 · 서식 칸 값을 쪽의 글자층 줄 뒤 · OCR 줄 앞에
//      source "extra" 문단으로 둔다. OCR 하는 쪽에서는 그림에 그려진 보이는 주석을 OCR 이 읽으므로
//      숨긴 주석만 붙인다(본문밖글읽기 설계서 6-3). 머리말 판정 · 제목 추론에는 넣지 않는다
//   한 줄이 블록 하나다. 텍스트층이 사실상 없으면 오류 → 기존 추출로 폴백.
//------------------------------------------------------------------

use std::collections::{BTreeMap, HashMap};

use once_cell::sync::Lazy;
use regex::Regex;

use super::blocks::{self as B, Block, Document};
use crate::pyutil;

const MAX_PAGES: usize = 3000;
const MAX_SECONDS: u64 = 120;
const EDGE_LINES: usize = 2;
const MIN_REPEAT_PAGES: usize = 3;
const MAX_HEADER_CHARS: usize = 60;
const MIN_HEADER_LETTERS: usize = 4;
const MIN_TEXT_CHARS: usize = 20;
const HEAD_RATIO: f64 = 1.12;
const HEAD_MAX_CHARS: usize = 60;
const MIN_INFERRED: usize = 3;
const HEAD_MAX_SHARE: f64 = 0.15;
const MAX_LEVEL: i64 = 3;

static PAGE_NO: Lazy<Regex> = Lazy::new(|| {
    Regex::new(concat!(r"(?i)^\s*(?:[-–—]\s*)?\d{1,4}(?:\s*[-–—])?\s*$",
                       r"|^\s*\d{1,4}\s*/\s*\d{1,4}\s*$",
                       r"|^\s*(?:page|p\.?)\s*\d{1,4}(?:\s*(?:of|/)\s*\d{1,4})?\s*$")).unwrap()
});
static TOC_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:\.{3,}|·{3,}|…{2,}|-{4,})\s*\d{1,4}\s*$").unwrap());
static WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
static DIGITS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d+").unwrap());
// 파이썬 [#\W_]+ — '#', 글자·숫자가 아닌 것, 밑줄
static NONWORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"[#\W_]+").unwrap());
static HANGUL_OR_LATIN: Lazy<Regex> = Lazy::new(|| Regex::new(r"[가-힣A-Za-z]").unwrap());

// \r\n | \r | \n 으로 줄 나누기(_LINE_SPLIT)
fn split_lines(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\r' {
            if it.peek() == Some(&'\n') {
                it.next();
            }
            out.push(std::mem::take(&mut cur));
        } else if c == '\n' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

//------------------------------------------------------------------
// PDF 추출 (진입점)
// -out: Ok(Document) / Err("PDF 열기 실패" · "PDF 텍스트층 없음(스캔 PDF)")
//------------------------------------------------------------------
pub fn extract(path: &str, title: &str) -> Result<Document, String> {
    extract_keep(path, title, None)
}

//------------------------------------------------------------------
// PDF 추출 — 고른 쪽만 (keep 이 None 이면 extract 와 같다)
//=> 비슷한 문서 찾기에서 쪽이 아주 많은 바깥 PDF 를 고른 쪽만 읽는다(pdfium::read_pages_keep).
//   읽지 않은 쪽은 빈 쪽이라 블록이 없고, 블록의 쪽 번호는 원래 쪽 번호 그대로다.
//
// -in: path · title = extract 와 같음 / keep = 쪽 번호(0부터), 전체 쪽 수 → 읽을지
//
// -out: Ok(Document) / Err("PDF 열기 실패" · "PDF 텍스트층 없음(스캔 PDF)")
//------------------------------------------------------------------
pub fn extract_keep(path: &str, title: &str, keep: Option<&dyn Fn(usize, usize) -> bool>) -> Result<Document, String> {
    let mut doc = Document::new("pdf", title);
    let t0 = std::time::Instant::now();
    let infer = crate::config::cfg().pdf_infer_headings;
    let want_tables = crate::config::cfg().pdf_tables;
    let mut stop_note: Option<(usize, usize)> = None;
    let (raw_pages, _n) = crate::pdfium::read_pages_keep(path, infer, want_tables, keep, |i, n| {
        if i >= MAX_PAGES || t0.elapsed().as_secs() > MAX_SECONDS {
            stop_note = Some((i, n));
            return false;
        }
        true
    }).map_err(|e| if e.contains("적재") { e } else { "PDF 열기 실패: PdfiumError".to_string() })?;
    if let Some((i, n)) = stop_note {
        doc.warnings.push(format!("PDF 부분 추출 {}/{}쪽 (상한)", i, n));
    }
    // OCR — 스캔 쪽은 캐시에 글이 있으면 그 줄을 쪽 끝에 붙이고, 없으면 대기열로 보낼 쪽만 적는다
    //   (OCR 설계서 §4 · §6). 고른 쪽만 읽을 때(비슷한 문서 찾기)는 OCR 하지 않는다
    let mut ocr_rows: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    // 구조 분석을 켰을 때 쪽(0부터)마다 OCR 블록 — 줄 글(ocr_rows)은 머리말 판정 · 글자 수 세기에 그대로 쓴다
    let layout_on = crate::config::cfg().ocr_layout;
    let mut layout_in: Vec<crate::ocr::layout::PageIn> = vec![];
    let mut info = crate::ocr::DocOcr::default();
    // OCR 할 쪽(완료 · 대기 모두, 0부터) — 이 쪽들은 보이는 주석을 OCR 이 읽는다
    let mut ocr_set: std::collections::HashSet<usize> = std::collections::HashSet::new();
    if keep.is_none() && crate::ocr::enabled() {
        let (want, dropped) = crate::ocr::pdf_ocr_pages(&raw_pages);
        ocr_set.extend(want.iter().copied());
        if !want.is_empty() {
            let cache = crate::hash::content_hash(path).ok().and_then(|h| crate::ocr::load_cache(&h));
            for &i in &want {
                match cache.as_ref().and_then(|c| c.pages.get(&(i + 1))) {
                    Some(lines) => {
                        let boxes: Vec<(String, [f32; 4])> = lines.iter().map(|l| (l.t.clone(), l.b)).collect();
                        ocr_rows.insert(i, crate::ocr::group_rows(&boxes).into_iter().map(|r| r.0).collect());
                        if layout_on {
                            // 쪽 크기: pt → 200dpi 픽셀(캐시 좌표와 같은 단위)
                            let k = crate::ocr::DPI / 72.0;
                            let pg = &raw_pages[i];
                            // OCR 그림은 쪽 회전 표시(/Rotate)를 적용한 "보이는 모습"으로 그린다 — page_w · page_h 는 회전 전이므로
                            // 90 · 270° 표시가 붙은 쪽은 가로 · 세로를 바꿔 보이는 크기로 되돌린다(안 하면 머리말 · 꼬리말 범위가 틀어진다)
                            let (vw, vh) = if pg.page_rot % 2 == 1 { (pg.page_h, pg.page_w) } else { (pg.page_w, pg.page_h) };
                            // 누운 쪽을 90 · 270° 돌려 OCR 했으면 캐시 좌표는 바로 선 쪽 기준 — 쪽 크기도 가로 · 세로를 바꾼다
                            let rot = cache.as_ref().map_or(0, |c| c.page_rot(i + 1));
                            let size = if crate::ocr::swaps_sides(rot) { [vh * k, vw * k] } else { [vw * k, vh * k] };
                            layout_in.push(crate::ocr::layout::PageIn { no: i + 1, lines: boxes, size: Some(size) });
                        }
                        info.done.push(i + 1);
                    }
                    None => info.pending.push(i + 1),
                }
            }
            if dropped > 0 {
                doc.warnings.push(format!("OCR 부분 {}/{}쪽 (상한)", want.len(), want.len() + dropped));
            }
        }
    }
    if keep.is_none() {
        crate::ocr::note(path, info.clone());
    }
    let mut pages: Vec<Vec<String>> = vec![];
    let mut sizes: Vec<Vec<Option<f64>>> = vec![];
    let mut anchors: Vec<BTreeMap<usize, Vec<Vec<Vec<String>>>>> = vec![];
    // 쪽마다 OCR 줄이 시작하는 줄 번호 — 이 번호부터는 OCR 출처 블록이다
    let mut ocr_from: Vec<usize> = vec![];
    // 쪽마다 본문 밖 글 — 줄 목록(pages)에 넣지 않는다: 머리말 · 꼬리말이 쪽 끝 2줄로 판정되므로
    // 섞으면 그 쪽의 꼬리말 판정이 밀린다
    let extras: Vec<Vec<String>> = raw_pages.iter().enumerate().map(|(pi, p)| page_extras(p, ocr_set.contains(&pi))).collect();
    for (pi, p) in raw_pages.iter().enumerate() {
        // 표가 있으면 표 글자를 뺀 글로 줄을 만든다(줄 수는 원문과 같다)
        let (body, at) = if p.tables.is_empty() { (p.text.clone(), BTreeMap::new()) } else { strip_tables(&p.text, &p.tables) };
        let mut lines: Vec<String> = split_lines(&body).into_iter().map(|l| pyutil::rstrip(&l).to_string()).collect();
        // 글자 크기는 원문 글(p.text)로 잰다 — 줄바꿈을 그대로 두었으므로 줄 번호가 같다
        let mut sz = if infer { line_sizes(p, lines.len()) } else { vec![None; lines.len()] };
        ocr_from.push(lines.len());
        // 글자층 줄 뒤에 OCR 줄을 붙인다 — 글자층(쪽 번호·제목)이 먼저, 그림 속 본문이 뒤
        if let Some(rows) = ocr_rows.get(&pi) {
            lines.extend(rows.iter().cloned());
            sz.extend(std::iter::repeat(None).take(rows.len()));
        }
        pages.push(lines);
        sizes.push(sz);
        anchors.push(at);
    }
    doc.pages = Some(pages.len() as i64);
    let n_tables: usize = anchors.iter().map(|at| at.values().map(|v| v.len()).sum::<usize>()).sum();
    let count = |l: &String| l.chars().filter(|c| !pyutil::is_space(*c)).count();
    // 본문 밖 글자도 센다 — 글상자만 있는 스캔 PDF(OCR 끔)도 블록으로 돌려준다(설계서 정할 것 ③)
    let total: usize = pages.iter().chain(extras.iter()).map(|p| p.iter().map(count).sum::<usize>()).sum();
    // 글이 전부 표 안에 있는 쪽들도 텍스트층은 있는 것이다
    //   OCR 을 기다리는 쪽이 있으면 실패가 아니다 — 빈 문서로 돌려주고 색인이 대기열에 넣는다
    if total < MIN_TEXT_CHARS && n_tables == 0 && info.pending.is_empty() {
        return Err("PDF 텍스트층 없음(스캔 PDF)".into());
    }
    let edge = edge_kinds(&pages);
    // 구조 분석 결과를 쪽(1부터)별로 나눠 둔다 — 쪽의 글자층 줄 뒤에 넣는다(이전과 같은 자리)
    let mut layout_blocks: BTreeMap<i64, Vec<Block>> = BTreeMap::new();
    if layout_on && !layout_in.is_empty() {
        for b in crate::ocr::layout::analyze(&layout_in) {
            layout_blocks.entry(b.page.unwrap_or(0)).or_default().push(b);
        }
    }
    let heads = if infer { infer_headings(&pages, &sizes, &edge) } else { HashMap::new() };
    for (pi, (lines, at)) in pages.iter().zip(&anchors).enumerate() {
        let no = pi as i64 + 1;
        // 본문 밖 글은 글자층 줄 다음 · OCR 줄 앞에 한 번 넣는다
        let mut extra_done = false;
        for (j, ln) in lines.iter().enumerate() {
            if j == ocr_from[pi] && !extra_done {
                push_extras(&mut doc.blocks, &extras[pi], no);
                extra_done = true;
            }
            // 이 줄에서 시작한 표를 먼저 — 표 앞 글(표 제목·단위)은 앞 줄에 이미 나왔다
            if let Some(tabs) = at.get(&j) {
                for rows in tabs {
                    doc.blocks.push(Block::table(rows.clone(), 0).pg(Some(no)));
                }
            }
            let t = pyutil::strip(ln);
            if t.is_empty() {
                continue;
            }
            // 구조 분석을 켰으면 OCR 줄은 아래에서 블록으로 한꺼번에 넣는다
            if layout_on && j >= ocr_from[pi] {
                continue;
            }
            // OCR 줄 — 제목 추론·머리말 판정 없이 본문 문단으로, 출처는 ocr
            if j >= ocr_from[pi] {
                doc.blocks.push(Block::new(B::PARAGRAPH, t).pg(Some(no)).src(B::SRC_OCR));
                continue;
            }
            if let Some(lv) = heads.get(&(no, j)) {
                doc.blocks.push(Block::new(B::HEADING, t).lv(*lv).pg(Some(no)).src(B::SRC_INFERRED));
                continue;
            }
            let kind = edge.get(&(no, j)).copied().unwrap_or(B::PARAGRAPH);
            doc.blocks.push(Block::new(kind, t).pg(Some(no)));
        }
        // OCR 줄이 없는 쪽(또는 구조 분석 블록을 쓰는 쪽)은 여기서 — OCR 구조 블록보다 앞
        if !extra_done {
            push_extras(&mut doc.blocks, &extras[pi], no);
        }
        // 이 쪽의 OCR 구조 블록 — 글자층 줄 뒤(이전 OCR 줄 자리)
        if let Some(bs) = layout_blocks.remove(&no) {
            doc.blocks.extend(bs);
        }
    }
    Ok(doc)
}

//------------------------------------------------------------------
// 쪽 하나에서 블록으로 넣을 본문 밖 글 고르기 (본문밖글읽기 설계서 6-3 A 안)
//=> OCR 하지 않는 쪽은 모두 넣는다. OCR 하는 쪽(완료 · 대기)은 보이는 주석이 그림에 그려져 OCR 이
//   읽으므로 숨긴 주석만 넣는다 — 같은 글이 두 번 들어가지 않게.
//
// -in: p     = 쪽 읽은 결과(extra 는 extra_text 켬일 때만 차 있다)
// -in: ocred = 이 쪽을 OCR 하나
//
// -out: 넣을 글 목록(쪽 안 위 → 아래 순서)
// -out: error = 예외 없음
//------------------------------------------------------------------
fn page_extras(p: &crate::pdfium::PageText, ocred: bool) -> Vec<String> {
    p.extra.iter().filter(|e| !ocred || e.hidden).map(|e| e.text.clone()).collect()
}

// 본문 밖 글 → 문단 블록(source "extra"). 글상자 하나가 블록 하나 — 안쪽 줄바꿈은 그대로 둔다
fn push_extras(out: &mut Vec<Block>, texts: &[String], no: i64) {
    for t in texts {
        out.push(Block::new(B::PARAGRAPH, t.clone()).pg(Some(no)).src(B::SRC_EXTRA));
    }
}

//------------------------------------------------------------------
// 쪽 글에서 표 글자 빼기 (Python pdf._strip_tables)
//=> 표 셀에 들어간 글자를 쪽 글에서 지운다(같은 글이 본문 줄과 표에 두 번 들어가지 않게).
//   줄바꿈은 그대로 두어 줄 번호가 원문과 같게 유지한다 — 글자 크기(line_sizes)와 맞추기 위해.
//   표마다 '첫 글자가 있던 줄'을 기록해 그 자리에 표 블록을 끼운다.
// -in: txt = 쪽 글 / tables = pdftable 결과
// -out: (표 글자를 뺀 글, {줄 번호: [표 행렬, …]})
//------------------------------------------------------------------
fn strip_tables(txt: &str, tables: &[super::pdftable::Table]) -> (String, BTreeMap<usize, Vec<Vec<Vec<String>>>>) {
    // 첫 글자 위치 → 표 (같은 위치면 뒤의 표가 이긴다 — 파이썬 dict 와 같다)
    let mut first: HashMap<usize, &Vec<Vec<String>>> = HashMap::new();
    let mut drop: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for t in tables {
        if let Some(&m) = t.used.iter().next() {
            first.insert(m, &t.rows);
        }
        drop.extend(t.used.iter().copied());
    }
    let chars: Vec<char> = txt.chars().collect();
    let mut out = String::with_capacity(txt.len());
    let mut at: BTreeMap<usize, Vec<Vec<Vec<String>>>> = BTreeMap::new();
    let (mut line, mut i) = (0usize, 0usize);
    while i < chars.len() {
        let ch = chars[i];
        if let Some(rows) = first.get(&i) {
            at.entry(line).or_default().push((*rows).clone());
        }
        if ch == '\r' || ch == '\n' {
            out.push(ch);
            // \r\n 은 줄바꿈 하나 — split_lines 와 같은 규칙
            if ch == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' {
                out.push('\n');
                i += 1;
            }
            line += 1;
        } else if !drop.contains(&i) {
            out.push(ch);
        }
        i += 1;
    }
    (out, at)
}

// 줄마다 글꼴 크기 중앙값(P-2). 글자 수와 글 길이가 다르면 그 쪽은 모름
fn line_sizes(p: &crate::pdfium::PageText, n_lines: usize) -> Vec<Option<f64>> {
    let sizes = match &p.sizes {
        Some(s) => s,
        None => return vec![None; n_lines],
    };
    let chars: Vec<char> = p.text.chars().collect();
    if p.n_chars as usize != chars.len() {
        return vec![None; n_lines];
    }
    let median = |v: &mut Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let m = v.len();
        Some(if m % 2 == 1 { v[m / 2] } else { (v[m / 2 - 1] + v[m / 2]) / 2.0 })
    };
    let mut out = vec![];
    let mut cur: Vec<f64> = vec![];
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\r' || ch == '\n' {
            out.push(median(&mut cur));
            cur.clear();
            i += if ch == '\r' && i + 1 < chars.len() && chars[i + 1] == '\n' { 2 } else { 1 };
            continue;
        }
        if !pyutil::is_space(ch) {
            cur.push(pyutil::round_to(sizes.get(i).copied().unwrap_or(0.0), 1));
        }
        i += 1;
    }
    out.push(median(&mut cur));
    if out.len() == n_lines { out } else { vec![None; n_lines] }
}

// 글자 크기로 제목 줄 추정(P-2)
fn infer_headings(pages: &[Vec<String>], sizes: &[Vec<Option<f64>>], edge: &HashMap<(i64, usize), &'static str>)
                  -> HashMap<(i64, usize), i64> {
    let key = |f: f64| (f * 10.0).round() as i64; // 0.1 단위 크기를 정수 키로
    let mut weight: BTreeMap<i64, usize> = BTreeMap::new();
    let mut n_lines = 0usize;
    for (lines, szs) in pages.iter().zip(sizes) {
        for (ln, s) in lines.iter().zip(szs) {
            let t = pyutil::strip(ln);
            if let Some(s) = s {
                if *s != 0.0 && pyutil::clen(t) >= 15 {
                    *weight.entry(key(*s)).or_default() += pyutil::clen(t);
                }
            }
            if !t.is_empty() {
                n_lines += 1;
            }
        }
    }
    if weight.is_empty() {
        return HashMap::new();
    }
    // 최빈(가중) 크기 — 같으면 먼저 나온 것(파이썬 dict 순서)에 가깝게 작은 키
    let body = *weight.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).unwrap().0 as f64 / 10.0;
    let mut cand: Vec<((i64, usize), f64)> = vec![];
    for (pi, (lines, szs)) in pages.iter().zip(sizes).enumerate() {
        let no = pi as i64 + 1;
        for (j, (ln, s)) in lines.iter().zip(szs).enumerate() {
            let t = pyutil::strip(ln);
            let s = match s { Some(s) if *s != 0.0 => *s, _ => continue };
            if t.is_empty() || edge.contains_key(&(no, j)) || pyutil::clen(t) > HEAD_MAX_CHARS {
                continue;
            }
            if s >= body * HEAD_RATIO && HANGUL_OR_LATIN.is_match(t) && !TOC_TAIL.is_match(t) {
                cand.push(((no, j), s));
            }
        }
    }
    if cand.len() < MIN_INFERRED || cand.len() as f64 > n_lines as f64 * HEAD_MAX_SHARE {
        return HashMap::new();
    }
    let rk = |s: f64| (pyutil::round_to(s / body, 1) * 10.0).round() as i64;
    let mut ranks: Vec<i64> = cand.iter().map(|(_, s)| rk(*s)).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    ranks.dedup();
    cand.into_iter().map(|(pos, s)| {
        let k = ranks.iter().position(|r| *r == rk(s)).unwrap_or(0) as i64;
        (pos, (k + 1).min(MAX_LEVEL))
    }).collect()
}

//------------------------------------------------------------------
// 쪽 가장자리 줄 종류 (머리말·꼬리말·쪽번호)
//------------------------------------------------------------------
pub(crate) fn edge_kinds(pages: &[Vec<String>]) -> HashMap<(i64, usize), &'static str> {
    let mut out: HashMap<(i64, usize), &'static str> = HashMap::new();
    // 키 순서를 처음 나온 순서로 지키려고 Vec 로 모은다
    let mut tops: Vec<(String, Vec<(i64, usize)>)> = vec![];
    let mut bottoms: Vec<(String, Vec<(i64, usize)>)> = vec![];
    for (pi, lines) in pages.iter().enumerate() {
        let no = pi as i64 + 1;
        let idx: Vec<usize> = (0..lines.len()).filter(|&j| !pyutil::strip(&lines[j]).is_empty()).collect();
        let top: Vec<usize> = idx.iter().take(EDGE_LINES).copied().collect();
        let bottom: Vec<usize> = idx.iter().skip(idx.len().saturating_sub(EDGE_LINES)).copied().collect();
        for (zone, picks) in [("top", top), ("bottom", bottom)] {
            for j in picks {
                let s = pyutil::strip(&lines[j]);
                if PAGE_NO.is_match(s) {
                    out.insert((no, j), B::PAGE_NUMBER);
                    continue;
                }
                let key = DIGITS.replace_all(&WS.replace_all(s, " "), "#").to_string();
                if pyutil::clen(s) > MAX_HEADER_CHARS || pyutil::clen(&NONWORD.replace_all(&key, "")) < MIN_HEADER_LETTERS {
                    continue;
                }
                let bucket = if zone == "top" { &mut tops } else { &mut bottoms };
                match bucket.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, v)) => v.push((no, j)),
                    None => bucket.push((key, vec![(no, j)])),
                }
            }
        }
    }
    for (bucket, kind) in [(&tops, B::HEADER), (&bottoms, B::FOOTER)] {
        for (_, spots) in bucket.iter() {
            let pages_seen: std::collections::HashSet<i64> = spots.iter().map(|s| s.0).collect();
            if pages_seen.len() >= MIN_REPEAT_PAGES {
                let mut uniq = spots.clone();
                uniq.sort();
                uniq.dedup();
                for spot in uniq.into_iter().skip(1) {
                    out.entry(spot).or_insert(kind);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdfium::{ExtraKind, ExtraText, PageText};

    // 시험용 쪽 — 보이는 글상자 하나, 숨긴 글상자 하나
    fn page_with_notes() -> PageText {
        let mut p = crate::pdfium::read_pages_empty_for_test();
        p.extra = vec![
            ExtraText { kind: ExtraKind::FreeText, text: "보임".into(), hidden: false, rect: [0.0, 0.0, 10.0, 10.0] },
            ExtraText { kind: ExtraKind::FreeText, text: "숨김".into(), hidden: true, rect: [0.0, 0.0, 10.0, 10.0] },
        ];
        p
    }

    // OCR 하지 않는 쪽은 보이는 · 숨긴 주석을 모두 넣는다
    #[test]
    fn extras_on_text_page_take_all() {
        assert_eq!(page_extras(&page_with_notes(), false), vec!["보임".to_string(), "숨김".to_string()]);
    }

    // OCR 하는 쪽은 그림에 안 그려진 숨긴 주석만 넣는다(보이는 것은 OCR 이 읽는다)
    #[test]
    fn extras_on_ocr_page_take_hidden_only() {
        assert_eq!(page_extras(&page_with_notes(), true), vec!["숨김".to_string()]);
    }

    // 본문 밖 글 블록은 source "extra" 문단이고 쪽 번호가 붙는다
    #[test]
    fn extra_blocks_are_paragraphs_from_extra() {
        let mut out = vec![];
        push_extras(&mut out, &["가".to_string()], 3);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].kind, out[0].source, out[0].page), (B::PARAGRAPH, B::SRC_EXTRA, Some(3)));
    }
}
