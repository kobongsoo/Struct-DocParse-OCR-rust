//------------------------------------------------------------------
// 글자층이 있는 PDF 쪽을 OCR 로 고치기 (0.4.0~ — 청킹기법 추가검토 설계서 2026-11-27 3.8 · 3.9)
//=> 어떤 PDF 는 글자층에서 숫자가 빠져 있다("제8조제1항" → "제 조제 항", "1인당 3만원" → "인당 만원").
//   글자층이 있으니 스캔 쪽 판정(pdf_ocr_pages)에 걸리지 않아 OCR 되지 않는다. 호출 옵션으로 쪽을 골라
//   쪽 전체를 OCR(run::full_page_rows)한 뒤 두 방식 중 하나로 고친다.
//    · fill(기본, 3.9) — 글자층 블록은 그대로 두고 '숫자가 빠진 틈'만 OCR 글로 채운다. 띄어쓰기는 글자층 것
//    · replace(3.8)   — 그 쪽의 글자층 본문 블록(표 빼고)을 OCR 줄 문단으로 바꾼다. 제목 구조는 사라진다
//   DecideRAG ocrfix.py 와 같은 규칙이다(한글 · 영문 글자만 맞춰 보고, 이웃한 두 글자가 OCR 에서도 이웃하고
//   그 사이 틈에 숫자가 있을 때만 바꾼다. 맞은 글자 90% 미만이거나 OCR 쪽에서 흩어지면 바꾸지 않는다).
//------------------------------------------------------------------

use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::docmodel::blocks::{self as B, Block, Document};
use crate::pyutil;

// 숫자가 빠진 흔적 — "제 조" "조제 항" "회 만원" " 만원 " "제조제항"(DecideRAG ocrfix 와 같은 패턴)
static MISSING_DIGITS: Lazy<Regex> = Lazy::new(|| Regex::new(r"제\s+조|조제\s+항|\s회\s+만원|\s+만원\s|제\s*조\s*제\s*항").unwrap());

// 맞춰 볼 글자 수가 이보다 적은 블록은 고치지 않는다(어디에 맞는지 믿을 수 없다)
const MIN_LETTERS: usize = 4;
// 맞은 글자 비율 하한
const MIN_COVER: f64 = 0.9;
// 정렬 표 크기 상한(블록 글자 수 × 쪽 글자 수) — 넘으면 건너뛴다
const MAX_CELLS: usize = 30_000_000;

//------------------------------------------------------------------
// 강제 OCR 옵션 (호출 옵션 "ocr" 안)
// -필드: pages = 글자층이 있어도 OCR 할 쪽(1부터) — force_pages
// -필드: when  = 스스로 고를 조건 — "missing_digits" 면 숫자 빠짐 흔적이 있는 쪽 (force_when)
// -필드: mode  = "fill"(빠진 숫자 틈만 채움, 기본) | "replace"(쪽 본문을 OCR 글로 바꿈) (force_mode)
//------------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct Force {
    pub pages: Vec<usize>,
    pub when: Option<String>,
    pub mode: String,
}

//------------------------------------------------------------------
// 고친 결과 요약 (결과 JSON 의 ocr.forced)
// -필드: pages  = OCR 한 쪽(1부터)
// -필드: mode   = fill | replace
// -필드: fixed  = 글을 고친 블록 수(fill) / 바꾼 쪽 수(replace)
// -필드: tried  = 고쳐 보려 한 블록 수(fill 에서 숫자 틈이 있을 수 있는 블록)
//------------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub pages: Vec<usize>,
    pub mode: String,
    pub fixed: usize,
    pub tried: usize,
}

// 고칠 수 있는 블록인가 — 표 · 제외 블록 · 이미 OCR 글인 블록은 건드리지 않는다
fn fixable(b: &Block) -> bool {
    b.kind != B::TABLE && !B::excluded(b.kind) && b.source != B::SRC_OCR && !b.synthetic
}

//------------------------------------------------------------------
// 숫자 빠짐 흔적이 있는 쪽 고르기 (force_when = "missing_digits")
//
// -in: doc = 추출 결과
//
// -out: 쪽 번호(1부터, 오름차순)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn pages_missing_digits(doc: &Document) -> Vec<usize> {
    let mut text: BTreeMap<usize, String> = BTreeMap::new();
    for b in &doc.blocks {
        if let (true, Some(p)) = (fixable(b), b.page) {
            let e = text.entry(p.max(0) as usize).or_default();
            e.push_str(&b.text);
            e.push('\n');
        }
    }
    text.into_iter().filter(|(p, t)| *p >= 1 && MISSING_DIGITS.is_match(t)).map(|(p, _)| p).collect()
}

// 글자(한글 · 영문 · 한자 등 — 숫자 · 기호 · 공백은 아님)와 그 바이트 범위
fn letters(s: &str) -> Vec<(char, usize, usize)> {
    s.char_indices().filter(|(_, c)| c.is_alphabetic()).map(|(i, c)| (c, i, i + c.len_utf8())).collect()
}

//------------------------------------------------------------------
// 블록 글자를 쪽 OCR 글자에 맞추기 (반전역 정렬)
//=> 블록 글자는 모두 맞춰야 하고, OCR 쪽은 앞뒤를 공짜로 건너뛴다(블록은 쪽의 일부이므로).
//   맞음 +2, 틀림 · 건너뜀 −1. 거꾸로 따라가며 같은 글자로 맞은 쌍만 돌려준다.
//
// -in: t = 블록 글자 / o = 쪽 OCR 글자
//
// -out: Some([(블록 글자 번호, OCR 글자 번호)]) — 앞에서부터 / None = 표가 너무 큼
// -out: error = 예외 없음
//------------------------------------------------------------------
fn align(t: &[(char, usize, usize)], o: &[(char, usize, usize)]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (t.len(), o.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_CELLS {
        return None;
    }
    let w = m + 1;
    // 점수는 앞 줄 · 지금 줄 두 줄만, 되짚기 표시는 전부(0 = 대각, 1 = 위(블록 글자 건너뜀), 2 = 왼쪽(OCR 글자 건너뜀))
    let mut prev: Vec<i32> = vec![0; w];
    let mut cur: Vec<i32> = vec![0; w];
    let mut back: Vec<u8> = vec![0; (n + 1) * w];
    for i in 1..=n {
        // 블록 글자를 건너뛰는 값 — OCR 쪽 앞은 공짜지만 블록 쪽은 아니다
        cur[0] = -(i as i32);
        back[i * w] = 1;
        for j in 1..=m {
            let d = prev[j - 1] + if t[i - 1].0 == o[j - 1].0 { 2 } else { -1 };
            let u = prev[j] - 1;
            let l = cur[j - 1] - 1;
            let (v, k) = if d >= u && d >= l { (d, 0) } else if u >= l { (u, 1) } else { (l, 2) };
            cur[j] = v;
            back[i * w + j] = k;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    // 끝은 OCR 쪽 아무 자리에서나(뒤도 공짜)
    let mut j = (0..=m).max_by_key(|&j| (prev[j], std::cmp::Reverse(j))).unwrap_or(0);
    let mut i = n;
    let mut pairs = vec![];
    while i > 0 && j > 0 {
        match back[i * w + j] {
            0 => {
                if t[i - 1].0 == o[j - 1].0 {
                    pairs.push((i - 1, j - 1));
                }
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    pairs.reverse();
    Some(pairs)
}

//------------------------------------------------------------------
// 블록 글 하나의 빠진 숫자 채우기 (fill 의 핵심)
//    1) 블록 · 쪽 OCR 의 글자만 뽑아 맞춘다(align)
//    2) 맞은 글자가 90% 미만이거나, OCR 쪽에서 맞은 범위가 블록 글자 수의 1.2배 + 3 보다 넓으면(흩어짐) 그대로
//    3) 블록에서 이웃한 두 글자가 OCR 에서도 바로 이웃하고, OCR 의 두 글자 사이 틈에 숫자가 있고 줄바꿈이 없고,
//       블록의 틈에는 숫자가 없으면 → 블록의 틈을 OCR 의 틈으로 바꾼다. 다른 틈(띄어쓰기)은 그대로
//
// -in: text = 블록 글 / page = 쪽 OCR 글(줄을 \n 으로 이은 것) / pl = page 의 글자(letters)
//
// -out: Some(고친 글) = 한 곳 이상 바꿈 / None = 바꿀 곳 없음 · 믿을 수 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn fill_digits(text: &str, page: &str, pl: &[(char, usize, usize)]) -> Option<String> {
    let tl = letters(text);
    if tl.len() < MIN_LETTERS {
        return None;
    }
    let pairs = align(&tl, pl)?;
    if (pairs.len() as f64) < MIN_COVER * tl.len() as f64 {
        return None;
    }
    let (o_first, o_last) = (pairs.first()?.1, pairs.last()?.1);
    if (o_last - o_first + 1) as f64 > tl.len() as f64 * 1.2 + 3.0 {
        return None;
    }
    // 블록 글자 번호 → OCR 글자 번호
    let mut to_o: Vec<Option<usize>> = vec![None; tl.len()];
    for &(a, b) in &pairs {
        to_o[a] = Some(b);
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut last = 0usize;
    let mut changed = false;
    for k in 0..tl.len().saturating_sub(1) {
        let (Some(oa), Some(ob)) = (to_o[k], to_o[k + 1]) else { continue };
        if ob != oa + 1 {
            continue;
        }
        let t_gap = &text[tl[k].2..tl[k + 1].1];
        let o_gap = &page[pl[oa].2..pl[ob].1];
        if o_gap.contains('\n') || !o_gap.chars().any(|c| c.is_ascii_digit()) || t_gap.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        // 틈을 OCR 것으로 — 앞부분은 그대로 옮기고 틈만 바꿔 넣는다
        out.push_str(&text[last..tl[k].2]);
        out.push_str(o_gap);
        last = tl[k + 1].1;
        changed = true;
    }
    if !changed {
        return None;
    }
    out.push_str(&text[last..]);
    Some(out)
}

//------------------------------------------------------------------
// 고른 쪽들의 OCR 글로 문서 고치기 (입구)
//
// -in: doc  = 추출 결과(블록 글 · 출처를 바꾼다)
// -in: rows = 쪽 번호(1부터) → OCR 줄 글(run::full_page_rows)
// -in: mode = "fill" | "replace"
//
// -out: Report
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn apply(doc: &mut Document, rows: &BTreeMap<usize, Vec<String>>, mode: &str) -> Report {
    let mut rep = Report { pages: rows.keys().copied().collect(), mode: mode.to_string(), ..Default::default() };
    if mode == "replace" {
        for (&no, lines) in rows {
            if replace_page(doc, no, lines) {
                rep.fixed += 1;
            }
        }
        return rep;
    }
    // fill — 쪽마다 OCR 글과 그 글자 목록을 한 번만 만든다
    let pages: BTreeMap<usize, (String, Vec<(char, usize, usize)>)> = rows.iter().map(|(&no, lines)| {
        let t = lines.join("\n");
        let l = letters(&t);
        (no, (t, l))
    }).collect();
    for b in doc.blocks.iter_mut() {
        if !fixable(b) {
            continue;
        }
        let Some((ptext, pl)) = b.page.and_then(|p| pages.get(&(p.max(0) as usize))) else { continue };
        rep.tried += 1;
        if let Some(t) = fill_digits(&b.text, ptext, pl) {
            b.text = t;
            // 나중에 걸러 볼 수 있게 출처를 남긴다
            b.source = B::SRC_OCR_FIX;
            rep.fixed += 1;
        }
    }
    rep
}

//------------------------------------------------------------------
// replace — 쪽 하나의 글자층 본문 블록(표 · 제외 블록 빼고)을 OCR 줄 문단으로 바꾸기
//=> 첫 본문 블록 자리에 OCR 줄을 순서대로 넣는다. 그 쪽에 본문 블록이 없으면 쪽 마지막 블록 뒤에.
//
// -in: doc = 문서 / no = 쪽 번호(1부터) / lines = OCR 줄 글
//
// -out: true = 바꿈 / false = 그 쪽 블록이 없거나 OCR 글이 없어 그대로
// -out: error = 예외 없음
//------------------------------------------------------------------
fn replace_page(doc: &mut Document, no: usize, lines: &[String]) -> bool {
    let lines: Vec<&str> = lines.iter().map(|l| pyutil::strip(l)).filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    let on_page = |b: &Block| b.page == Some(no as i64);
    let first = doc.blocks.iter().position(|b| on_page(b) && fixable(b));
    let at = match first {
        Some(i) => i,
        None => match doc.blocks.iter().rposition(|b| on_page(b)) {
            Some(i) => i + 1,
            None => return false,
        },
    };
    // 자리를 잡은 뒤 그 쪽 본문 블록을 뺀다(자리 앞의 블록은 빠지지 않으므로 at 는 그대로 맞다)
    let mut kept: Vec<Block> = Vec::with_capacity(doc.blocks.len());
    let mut inserted = false;
    for (i, b) in std::mem::take(&mut doc.blocks).into_iter().enumerate() {
        if i == at && !inserted {
            kept.extend(lines.iter().map(|l| Block::new(B::PARAGRAPH, *l).pg(Some(no as i64)).src(B::SRC_OCR)));
            inserted = true;
        }
        if on_page(&b) && fixable(&b) {
            continue;
        }
        kept.push(b);
    }
    if !inserted {
        kept.extend(lines.iter().map(|l| Block::new(B::PARAGRAPH, *l).pg(Some(no as i64)).src(B::SRC_OCR)));
    }
    doc.blocks = kept;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 숫자 틈만 채우고 띄어쓰기는 글자층 그대로
    fn fill_gaps_only() {
        let page = "청탁금지법에 따르면,누구에게든 1인당 3만원 이하의 식사만\n(제8조제1항,제2항,제4항,제5항 참조),";
        let pl = letters(page);
        assert_eq!(fill_digits("청탁금지법에 따르면, 누구에게든 인당 만원 이하의 식사만", page, &pl).as_deref(),
            Some("청탁금지법에 따르면, 누구에게든 1인당 3만원 이하의 식사만"));
        assert_eq!(fill_digits("(제 조제 항, 제 항, 제 항, 제 항 참조),", page, &pl).as_deref(),
            Some("(제8조제1항, 제2항, 제4항, 제5항 참조),"));
        // 이미 숫자가 있으면 그대로
        assert_eq!(fill_digits("누구에게든 1인당 3만원 이하의 식사만", page, &pl), None);
        // 맞는 곳이 없는 글은 그대로
        assert_eq!(fill_digits("전혀 다른 문장이 여기에 있습니다", page, &pl), None);
    }

    #[test]
    // 흔적 패턴으로 쪽 고르기
    fn pick_pages() {
        let mut d = Document::new("pdf", "x");
        d.blocks.push(Block::new(B::PARAGRAPH, "제 조제 항에 따라").pg(Some(2)));
        d.blocks.push(Block::new(B::PARAGRAPH, "제8조제1항에 따라").pg(Some(3)));
        d.blocks.push(Block::new(B::PARAGRAPH, "1인당 3만원").pg(Some(4)));
        assert_eq!(pages_missing_digits(&d), vec![2]);
    }

    #[test]
    // replace — 그 쪽 본문만 OCR 문단으로, 표와 다른 쪽은 그대로
    fn replace_one_page() {
        let mut d = Document::new("pdf", "x");
        d.blocks.push(Block::new(B::PARAGRAPH, "1쪽 글").pg(Some(1)));
        d.blocks.push(Block::new(B::HEADING, "제 조").lv(1).pg(Some(2)));
        d.blocks.push(Block::table(vec![vec!["a".into()]], 0).pg(Some(2)));
        d.blocks.push(Block::new(B::PARAGRAPH, "인당 만원").pg(Some(2)));
        d.blocks.push(Block::new(B::PARAGRAPH, "3쪽 글").pg(Some(3)));
        let mut rows = BTreeMap::new();
        rows.insert(2usize, vec!["제8조".to_string(), "1인당 3만원".to_string()]);
        let r = apply(&mut d, &rows, "replace");
        assert_eq!(r.fixed, 1);
        let t: Vec<(&str, &str)> = d.blocks.iter().map(|b| (b.kind, b.text.as_str())).collect();
        assert_eq!(t, vec![("paragraph", "1쪽 글"), ("paragraph", "제8조"), ("paragraph", "1인당 3만원"), ("table", ""), ("paragraph", "3쪽 글")]);
        assert_eq!(d.blocks[1].source, B::SRC_OCR);
    }
}
