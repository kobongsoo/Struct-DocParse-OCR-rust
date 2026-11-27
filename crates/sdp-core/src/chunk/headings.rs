//------------------------------------------------------------------
// 청킹 전에 제목 다듬기 (0.4.0~ — 청킹기법 추가검토 설계서 2026-11-27 3.1 · 3.5 · 3.6 · 3.7)
//=> 청킹은 문서 사본에서 돌므로 여기서 블록 종류 · 수준을 바꿔도 결과의 blocks 는 그대로다.
//   모두 옵션이고 기본은 꺼짐 — 끄면 SimpleRAG 와 같은 청크가 그대로 나온다.
//    3.6 문장인데 제목으로 읽힌 블록 → 본문으로      (demote_sentence_headings)
//    3.7 번호 체계로 제목 깊이 다시 매기기           (heading_level_by_number)
//    3.1 패턴에 맞는 짧은 문단 → 제목으로            (heading_patterns · heading_max_chars · heading_level)
//    3.5 접두에 쓰는 문서 제목 다듬기                (title_clean)
//   순서는 3.6 → 3.7 → 3.1 — 가짜 제목을 먼저 빼야 번호 깊이와 '지금 절' 수준이 바르게 잡힌다.
//------------------------------------------------------------------

use once_cell::sync::Lazy;
use regex::Regex;

use super::ChunkOpts;
use crate::docmodel::blocks::{self as B, Document};
use crate::pyutil;

// 문장으로 끝나는 글 — "…한다." "…함." "…음." "…임." "…됨." (마침표 뒤 공백 · 닫는 괄호 허용)
static SENTENCE_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"(다|함|음|임|됨|요)\.[\s)\]」』”’]*$").unwrap());
// 점 번호 "1." "1.2" "13.1.2" — 숫자 개수가 깊이
static DECIMAL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*(\d{1,3}(?:\.\d{1,3})*)\.?(?:\s|$)").unwrap());
// 점 번호 아래에 오는 번호 꼴 — 처음 나온 순서대로 한 단계씩 아래
static SUB_MARKS: Lazy<Vec<(&'static str, Regex)>> = Lazy::new(|| {
    vec![
        ("가.", Regex::new(r"^\s*[가-하]\.(?:\s|$)").unwrap()),
        ("가)", Regex::new(r"^\s*[가-하]\)").unwrap()),
        ("1)", Regex::new(r"^\s*\d{1,3}\)").unwrap()),
        ("(1)", Regex::new(r"^\s*\(\d{1,3}\)").unwrap()),
        ("(가)", Regex::new(r"^\s*\([가-하]\)").unwrap()),
        ("①", Regex::new(r"^\s*[①-⑳]").unwrap()),
    ]
});
// 파일 이름 앞 번호 "03." "25_" "36-" "1 "
static TITLE_HEAD_NO: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{1,4}\s*[._\-]\s*|^\d{1,4}\s+").unwrap());
// 파일 이름 끝 날짜 "_16.04.01" "-2019.03.29" "_20240101" " 2024-01-01"
static TITLE_TAIL_DATE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\s_\-]*(?:\d{2,4}[.\-]\d{1,2}[.\-]\d{1,2}|\d{8})$").unwrap());
// 파일 이름 끝 사본 표시 "[1]" "(1)"
static TITLE_TAIL_COPY: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*(?:\[\d+\]|\(\d+\))$").unwrap());

//------------------------------------------------------------------
// 문서 사본의 제목 다듬기 (입구)
//=> 켜진 옵션만 차례로 적용한다. 셋 다 꺼져 있으면 아무것도 하지 않는다.
//
// -in: doc = 청킹할 문서 사본(블록 종류 · 수준을 바꾼다)
// -in: o   = 청킹 옵션
//
// -out: 없음
// -out: error = 예외 없음(패턴은 validate 에서 미리 검사 — 여기서 못 읽는 패턴은 건너뛴다)
//------------------------------------------------------------------
pub fn adjust(doc: &mut Document, o: &ChunkOpts) {
    if o.demote_sentence_headings {
        demote_sentences(doc, o.heading_max_chars);
    }
    if o.heading_level_by_number {
        levels_by_number(doc);
    }
    if !o.heading_patterns.is_empty() {
        promote_patterns(doc, o);
    }
}

//------------------------------------------------------------------
// 3.6 문장형 가짜 제목 → 본문
//=> 출장여비규정의 "13.1.2 7.5. 규정에 따라 준비금을 지급할 수 있다." 처럼 문장이 제목으로 읽히면
//   절 경로에 문장이 통째로 들어간다. 너무 길거나 "다." "함." 등으로 끝나는 제목은 문단으로 돌린다.
//   합성 제목("슬라이드 3")은 건드리지 않는다.
//
// -in: doc       = 문서 사본
// -in: max_chars = 이보다 긴 제목은 본문(heading_max_chars)
//
// -out: 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn demote_sentences(doc: &mut Document, max_chars: usize) {
    for b in doc.blocks.iter_mut() {
        if b.kind != B::HEADING || b.synthetic {
            continue;
        }
        let t = pyutil::collapse_ws(&b.text);
        // 길이는 글자 수로 — 공백을 한 칸으로 줄인 뒤 잰다
        if pyutil::clen(&t) > max_chars || SENTENCE_END.is_match(&t) {
            b.kind = B::PARAGRAPH;
            b.level = 0;
        }
    }
}

//------------------------------------------------------------------
// 3.7 번호로 제목 깊이 다시 매기기
//=> "1." → 1, "1.2" → 2, "13.1.2" → 3. 그 아래 번호 꼴("가." "1)" "(1)" "①" …)은 문서에서 처음 나온
//   순서대로 바로 위 점 번호 깊이에서 한 단계씩 내려간다. 번호가 없는 제목은 파서가 준 수준 그대로다.
//
// -in: doc = 문서 사본
//
// -out: 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn levels_by_number(doc: &mut Document) {
    // 점 번호 아래 번호 꼴이 처음 나온 순서
    let mut order: Vec<&'static str> = vec![];
    // 가장 최근 점 번호 제목의 깊이(없으면 0)
    let mut base: i64 = 0;
    for b in doc.blocks.iter_mut() {
        if b.kind != B::HEADING || b.synthetic {
            continue;
        }
        let t = pyutil::strip(&b.text);
        if let Some(c) = DECIMAL.captures(t) {
            let depth = c[1].split('.').filter(|s| !s.is_empty()).count() as i64;
            b.level = depth.clamp(1, 9);
            base = b.level;
            continue;
        }
        if let Some((name, _)) = SUB_MARKS.iter().find(|(_, re)| re.is_match(t)) {
            let rank = match order.iter().position(|x| x == name) {
                Some(i) => i,
                None => {
                    order.push(name);
                    order.len() - 1
                }
            };
            b.level = (base + 1 + rank as i64).clamp(1, 9);
        }
    }
}

//------------------------------------------------------------------
// 3.1 패턴에 맞는 짧은 문단 → 제목
//=> 사례집 · Q&A 집의 "사례 15 …" "Q3. …" 는 제목 블록이 아니라 문단으로 읽혀, 여러 사례가 한 절에 섞인다.
//   패턴으로 시작하고 heading_max_chars 이하인 paragraph · list_item 을 제목으로 바꾼다.
//    - heading_level 0("child") = 바로 앞 제목(바꾼 제목은 빼고)의 한 단계 아래 → 사례끼리는 형제가 된다
//    - heading_level n          = 그 수준
//   바꾼 제목의 출처는 regex 다(절 source 에 드러난다).
//   demote_sentence_headings 도 켜져 있으면 "다." "함." 등으로 끝나는 문단은 올리지 않는다.
//
// -in: doc = 문서 사본
// -in: o   = 옵션(heading_patterns · heading_max_chars · heading_level)
//
// -out: 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn promote_patterns(doc: &mut Document, o: &ChunkOpts) {
    let pats: Vec<Regex> = o.heading_patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();
    if pats.is_empty() {
        return;
    }
    // 바로 앞 '원래' 제목의 수준 — child 수준을 정하는 기준
    let mut parent: i64 = 0;
    for b in doc.blocks.iter_mut() {
        if B::excluded(b.kind) {
            continue;
        }
        if b.kind == B::HEADING {
            parent = b.level.max(1);
            continue;
        }
        if b.kind != B::PARAGRAPH && b.kind != B::LIST_ITEM {
            continue;
        }
        let t = pyutil::collapse_ws(&b.text);
        if t.is_empty() || pyutil::clen(&t) > o.heading_max_chars || !pats.iter().any(|re| re.is_match(&t)) {
            continue;
        }
        // 문장형 제목을 거르는 옵션이 켜져 있으면 문장으로 끝나는 문단은 제목으로 올리지 않는다
        //   (예: 패턴 "사례" 에 걸린 "사례금에 포함되지 않는다.")
        if o.demote_sentence_headings && SENTENCE_END.is_match(&t) {
            continue;
        }
        b.kind = B::HEADING;
        b.level = if o.heading_level > 0 { o.heading_level.min(9) } else { (parent + 1).min(9) };
        b.source = B::SRC_REGEX;
    }
}

//------------------------------------------------------------------
// 3.5 문서 제목 다듬기 — "03.경조사지원규정_12.01.01" → "경조사지원규정"
//=> 앞 번호("03." "25_"), 끝 사본 표시("[1]" "(1)"), 끝 날짜("_16.04.01" "-2019.03.29")를 뗀다.
//   다 떼고 나서 빈 글이면 원래 제목을 그대로 쓴다.
//
// -in: title = 문서 제목(파일 이름에서 확장자를 뗀 것 또는 호출 옵션 title)
//
// -out: 다듬은 제목
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn clean_title(title: &str) -> String {
    let mut t = pyutil::strip(title).to_string();
    // 사본 표시 → 날짜 → 다시 사본 표시 순("규정_16.04.01[1]" · "규정[1]_16.04.01" 둘 다)
    for _ in 0..2 {
        t = TITLE_TAIL_COPY.replace(&t, "").to_string();
        t = TITLE_TAIL_DATE.replace(&t, "").to_string();
    }
    t = TITLE_HEAD_NO.replace(&t, "").to_string();
    // 남은 앞뒤 구분 문자 정리
    let t = t.trim_matches(|c: char| c == '_' || c == '-' || c == '.' || pyutil::is_space(c)).to_string();
    if t.is_empty() {
        pyutil::strip(title).to_string()
    } else {
        t
    }
}
