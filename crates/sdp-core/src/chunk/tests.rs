//------------------------------------------------------------------
// 청킹 단위 시험 (블록청킹 설계서 §9.2)
//=> 글자 수(CharMeasure)로 재서 토크나이저 없이 돈다. HfTok 시험만 tokenizer.json 이 있을 때 돈다
//   (SDP_TEST_TOKENIZER 환경변수 또는 SimpleRAG 배포 폴더의 e5-small-ko).
//------------------------------------------------------------------

use super::*;
use crate::docmodel::blocks::{self as B, Block, Document};

// 글자 수 옵션 — 작은 크기로 자르기를 쉽게 일으킨다
fn opts(tokens: usize) -> ChunkOpts {
    ChunkOpts { tokens, overlap: 4, min_tokens: 1, table_header_max_tokens: 64, table_whole: 0, ..ChunkOpts::default() }
}

// 접두 "[…]\n" 을 뗀 본문
fn body(t: &str) -> &str {
    match (t.starts_with('['), t.find("]\n")) {
        (true, Some(i)) => &t[i + 2..],
        _ => t,
    }
}

// 머리글 1행 + 값 n행 표 블록(렌더링 글 포함)
fn table(n: usize, page: i64) -> Block {
    let mut rows = vec![vec!["이름".to_string(), "금액".to_string()]];
    for i in 0..n {
        rows.push(vec![format!("항목{:02}", i), format!("{}원", i * 1000)]);
    }
    let text = rows.iter().map(|r| format!("| {} |", r.join(" | "))).collect::<Vec<_>>().join("\n");
    let mut b = Block::table(rows, 1).pg(Some(page));
    b.text = text;
    b
}

#[test]
// 청킹을 켜도 원본 블록(path 등)은 바뀌지 않는다
fn blocks_untouched() {
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::HEADING, "제1장 총칙").lv(1));
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 글입니다."));
    let before = d.to_json();
    let c = chunk_document(&d, &opts(128), &CharMeasure);
    assert!(!c.is_empty());
    assert_eq!(before, d.to_json());
    assert_eq!(c[0].path, vec!["제1장 총칙".to_string()]);
}

#[test]
// 제목만 든 상위 절은 앞 청크 끝에 매달리지 않고 다음 절과 함께 간다
fn heading_only_goes_forward() {
    let mut d = Document::new("docx", "문");
    d.blocks.push(Block::new(B::PARAGRAPH, "가".repeat(40)));
    d.blocks.push(Block::new(B::HEADING, "제1장").lv(1));
    d.blocks.push(Block::new(B::HEADING, "제1조").lv(2));
    d.blocks.push(Block::new(B::PARAGRAPH, "나".repeat(20)));
    let c = chunk_document(&d, &opts(70), &CharMeasure);
    // '제1장' 이 든 청크는 '제1조' 도 들고 있어야 한다
    for x in &c {
        if body(&x.text).contains("제1장") {
            assert!(x.text.contains("제1조"), "제목만 홀로 남음: {:?}", x.text);
        }
    }
    assert!(c.iter().any(|x| body(&x.text).contains("제1장")));
}

#[test]
// 큰 표는 행 경계에서 잘리고, 조각마다 머리글이 다시 붙고, 머리글이 조각 끝에 홀로 남지 않는다
fn table_header_repeats() {
    let mut d = Document::new("docx", "표");
    d.blocks.push(table(30, 1));
    let c = chunk_document(&d, &opts(80), &CharMeasure);
    assert!(c.len() > 2);
    let hdr = "| 이름 | 금액 |";
    for x in &c {
        let b = body(&x.text);
        assert!(b.starts_with(hdr), "머리글 없음: {:?}", b);
        assert_ne!(b.lines().last(), Some(hdr), "머리글이 끝에 홀로: {:?}", b);
    }
}

#[test]
// table_whole 이하 표는 예산을 넘어도 hard_max 안이면 통째로
fn small_table_whole() {
    let mut d = Document::new("docx", "표");
    d.blocks.push(table(5, 1));
    let mut o = opts(30);
    o.table_whole = 384;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert_eq!(c.len(), 1);
    assert!(c[0].text.contains("항목00") && c[0].text.contains("항목04"));
}

#[test]
// hard_max 를 넘는 청크는 없다
fn hard_max_kept() {
    let mut d = Document::new("docx", "문");
    d.blocks.push(Block::new(B::PARAGRAPH, "다".repeat(2000)));
    let mut o = opts(128);
    o.hard_max = 100;
    o.overflow_ratio = 2.0;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert!(c.len() > 1);
    assert!(c.iter().all(|x| CharMeasure.tlen(&x.text) <= 100));
}

#[test]
// 쪽 찾기 — 같은 절 안 쪽이 넘어가면 청크 시작 쪽도 따라간다
fn pages_follow() {
    let mut d = Document::new("pdf", "문");
    d.blocks.push(Block::new(B::HEADING, "제목").lv(1).pg(Some(1)));
    for p in 1..=3 {
        d.blocks.push(Block::new(B::PARAGRAPH, format!("{}쪽 글 {}", p, "라".repeat(50))).pg(Some(p)));
    }
    let c = chunk_document(&d, &opts(70), &CharMeasure);
    let pages: Vec<i64> = c.iter().filter_map(|x| x.page).collect();
    assert_eq!(pages.len(), c.len(), "쪽을 못 찾은 청크: {:?}", c);
    assert!(pages.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(pages.first(), Some(&1));
    assert_eq!(pages.last(), Some(&3));
}

#[test]
// 고정 크기 — 300글자를 100글자 창 · 겹침 10 으로: 0 · 90 · 180 · 270 시작, 4 조각
fn fixed_chars() {
    let mut d = Document::new("docx", "문");
    d.blocks.push(Block::new(B::PARAGRAPH, "마".repeat(300)));
    let mut o = opts(100);
    o.mode = "fixed".into();
    o.overlap = 10;
    o.min_tokens = 16;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert_eq!(c.len(), 4);
    assert_eq!(CharMeasure.tlen(&c[0].text), 100);
    assert_eq!(CharMeasure.tlen(&c[3].text), 30);
    assert!(c.iter().all(|x| x.source.is_empty() && x.section_id.is_none()));
}

#[test]
// 폴백 평문 — 출처 legacy, 접두는 문서 제목
fn legacy_plain() {
    let mut d = Document::new("legacy", "평문");
    d.raw_text = Some("첫 줄입니다.\n둘째 줄입니다.".into());
    let c = chunk_document(&d, &opts(128), &CharMeasure);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].source, "legacy");
    assert!(c[0].text.starts_with("[평문]"));
}

#[test]
// 접두 끔 — 청크가 본문 글로 시작하고 쪽 찾기도 된다
fn no_prefix() {
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::HEADING, "제목").lv(1).pg(Some(2)));
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 글입니다 충분히 긴 글").pg(Some(2)));
    let mut o = opts(128);
    o.prefix = false;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert!(c[0].text.starts_with("제목"));
    assert_eq!(c[0].page, Some(2));
}

#[test]
// 결과 JSON 모양 — SimpleRAG dump_chunks 와 같은 키
fn json_shape() {
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 글입니다."));
    let c = chunk_document(&d, &opts(128), &CharMeasure);
    let j = chunks_json(&c, &CharMeasure, "문서");
    let o = j[0].as_object().unwrap();
    let keys: Vec<&str> = o.keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["no", "tokens", "section_id", "path", "source", "page", "page_from", "page_to", "hash", "body_hash", "text"]);
}

#[test]
// e5 토크나이저 — 파일이 있을 때만
fn hf_tokenizer() {
    let p = std::env::var("SDP_TEST_TOKENIZER")
        .unwrap_or_else(|_| r"D:\Project\SimpleRAG\dist-rust\models\e5-small-ko\tokenizer.json".into());
    if !std::path::Path::new(&p).is_file() {
        eprintln!("tokenizer.json 없음 — 건너뜀: {}", p);
        return;
    }
    let t = HfTok::from_file(&p).unwrap();
    let s = "제1조(목적) 이 규정은 직원의 복무에 관한 사항을 정한다.";
    let ids = t.ids(s);
    assert!(!ids.is_empty() && ids.len() < s.chars().count());
    assert_eq!(t.tlen(s), ids.len());
    assert_eq!(t.decode(&ids), s);
    assert!(HfTok::from_file("없는/tokenizer.json").is_err());
}

// ── api 연결(0.3.0) ──────────────────────────────────────────

#[test]
// 열 때 옵션 chunk — tokens 만 주면 overlap 이 max(8, tokens/8) 로 따라간다 · 어긋난 값 · 모르는 키는 -1
fn open_options_chunk() {
    use crate::api;
    let o = api::options_from_json(&serde_json::json!({"chunk": {"tokens": 256, "unit": "chars"}})).unwrap();
    assert_eq!((o.chunk.opts.tokens, o.chunk.opts.overlap, o.chunk.unit.as_str()), (256, 32, "chars"));
    let o = api::options_from_json(&serde_json::json!({"chunk": {"tokens": 64, "overlap": 4}})).unwrap();
    assert_eq!(o.chunk.opts.overlap, 4);
    for bad in [serde_json::json!({"chunk": {"tokens": 10, "overlap": 10}}), serde_json::json!({"chunk": {"oops": 1}}),
                serde_json::json!({"chunk": {"enabled": true}}), serde_json::json!({"chunk": {"unit": "words"}}),
                serde_json::json!({"chunk": {"hard_max": 100, "tokens": 200}})] {
        assert_eq!(api::options_from_json(&bad).unwrap_err().0, api::EINVAL, "{}", bad);
    }
}

#[test]
// 호출 옵션 chunk — true · 객체는 켬, false · enabled:false 는 끔
fn call_options_chunk() {
    use crate::api;
    let c = |v: serde_json::Value| api::call_opts_from_json(&serde_json::json!({"chunk": v})).map(|c| c.chunk.map(|x| x.opts.tokens));
    assert_eq!(c(serde_json::json!(true)).unwrap(), Some(128));
    assert_eq!(c(serde_json::json!(false)).unwrap(), None);
    assert_eq!(c(serde_json::json!({"tokens": 200})).unwrap(), Some(200));
    assert_eq!(c(serde_json::json!({"enabled": false, "tokens": 200})).unwrap(), None);
    assert_eq!(c(serde_json::json!(3)).unwrap_err().0, api::EINVAL);
}

#[test]
// sdp_chunk 본체 — 결과 JSON(블록)을 다시 읽어 자르면 문서에서 바로 자른 것과 같다 · 토크나이저 없으면 -1
fn rechunk_from_json() {
    use crate::api;
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::HEADING, "제1장 총칙").lv(1).pg(Some(1)));
    d.blocks.push(table(20, 2));
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 ".repeat(40)).pg(Some(3)).src(B::SRC_EXTRA));
    let o = opts(60);
    let direct = chunks_json(&chunk_document(&d, &o, &CharMeasure), &CharMeasure, &d.title);
    let v = api::chunk(&d.to_json(), &serde_json::json!({"unit": "chars", "tokens": 60, "overlap": 4, "min_tokens": 1, "table_whole": 0})).unwrap();
    assert_eq!(v["chunks"], direct);
    assert_eq!(v["chunk_version"], 1);
    assert_eq!(v["chunking"]["unit"], "chars");
    // 토큰 단위인데 토크나이저 경로가 없다
    assert_eq!(api::chunk(&d.to_json(), &serde_json::Value::Null).unwrap_err().0, api::EINVAL);
    // 블록도 평문도 없는 JSON
    assert_eq!(api::chunk(&serde_json::json!({"title": "x"}), &serde_json::json!({"unit": "chars"})).unwrap_err().0, api::EINVAL);
}

#[test]
// 블록 JSON 왕복 — 종류 · 수준 · 쪽 · 출처 · 표 칸 · 합성 표시가 그대로
fn document_json_roundtrip() {
    let mut d = Document::new("pdf", "문서");
    d.pages = Some(3);
    d.blocks.push(Block::new(B::HEADING, "제목").lv(2).pg(Some(1)).src(B::SRC_INFERRED));
    d.blocks.push(table(2, 2));
    let mut s = Block::new(B::HEADING, "슬라이드 3").lv(1);
    s.synthetic = true;
    d.blocks.push(s);
    d.blocks.push(Block::new(B::NOTE, "메모").src(B::SRC_EXTRA));
    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(back.to_json(), d.to_json());
}

#[test]
// 청크 지문 — SHA-256 앞 16자리 · 접두 줄 떼기 · 공백 정리
fn chunk_hashes() {
    use super::hash::{body_of, hashes, sha16};
    // SHA-256("abc") = ba7816bf8f01cfea…  (표준 시험값)
    assert_eq!(sha16("abc"), "ba7816bf8f01cfea");
    // 접두 줄은 빠지고, 공백 · 줄바꿈은 한 칸으로
    assert_eq!(body_of("[규정 > 제1조]\n가  나\n다", "규정"), "가 나 다");
    // 작은 절을 묶은 청크 — 접두 줄이 여러 개
    assert_eq!(body_of("[규정 > 제1조]\n가\n\n[규정 > 제2조]\n나", "규정"), "가 나");
    // 본문의 "[별표 1]" 줄은 제목으로 시작하지 않으므로 남는다
    assert_eq!(body_of("[규정 > 부칙]\n[별표 1]\n가", "규정"), "[별표 1] 가");
    // 제목이 없으면 첫 줄만 접두로 본다
    assert_eq!(body_of("[제1조]\n[별표 1]\n가", ""), "[별표 1] 가");
    // 접두 줄만 든 청크 — 빈 글이 아니라 접두를 남긴 글로(서로 다른 목차 청크가 한 지문으로 묶이지 않게)
    assert_eq!(body_of("[규정 > 1. 목적]

[규정 > 2. 범위]", "규정"), "[규정 > 1. 목적] [규정 > 2. 범위]");
    assert_ne!(hashes("[규정 > 1. 목적]", "규정").1, hashes("[규정 > 2. 범위]", "규정").1);
    // 다른 문서 · 같은 본문 → hash 는 다르고 body_hash 는 같다
    let (h1, b1) = hashes("[인사규정 > 부칙]\n이 규정은 공포한 날부터 시행한다.", "인사규정");
    let (h2, b2) = hashes("[보안규정 > 부칙]\n이 규정은  공포한 날부터\n시행한다.", "보안규정");
    assert_ne!(h1, h2);
    assert_eq!(b1, b2);
    // 결과 JSON 에도 같은 값이 들어간다
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::HEADING, "제1조").lv(1));
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 글입니다."));
    let c = chunk_document(&d, &opts(128), &CharMeasure);
    let j = chunks_json(&c, &CharMeasure, &d.title);
    let t = j[0]["text"].as_str().unwrap();
    assert_eq!(j[0]["hash"], sha16(t));
    assert_eq!(j[0]["body_hash"], sha16(&body_of(t, "문서")));
    assert_eq!(body_of(t, "문서"), "제1조 본문 글입니다.");
}

// ── 0.4.0 옵션 (청킹기법 추가검토 설계서 2026-11-27) ──

#[test]
// 3.1 패턴 제목 — 사례 문단이 지금 절의 한 단계 아래 제목이 되어 사례끼리 형제가 된다
fn heading_patterns_child() {
    let mut d = Document::new("pdf", "사례집");
    d.blocks.push(Block::new(B::HEADING, "1. 인적 적용범위").lv(2));
    d.blocks.push(Block::new(B::PARAGRAPH, "사례 1 의사가 공직자등인지"));
    d.blocks.push(Block::new(B::PARAGRAPH, "답변 본문 하나입니다. ".repeat(3)));
    d.blocks.push(Block::new(B::PARAGRAPH, "사례 2 국회의원이 공직자등인지"));
    d.blocks.push(Block::new(B::PARAGRAPH, "답변 본문 둘입니다. ".repeat(3)));
    d.blocks.push(Block::new(B::PARAGRAPH, "사례금에 포함되지 않는다."));
    let mut o = opts(60);
    // 끄면(기본) 사례 문단은 본문
    let c0 = chunk_document(&d, &o, &CharMeasure);
    assert!(c0.iter().all(|c| c.path == vec!["1. 인적 적용범위".to_string()]));
    o.heading_patterns = vec![r"^\s*사례".to_string()];
    let c = chunk_document(&d, &o, &CharMeasure);
    let paths: Vec<Vec<String>> = c.iter().map(|x| x.path.clone()).collect();
    assert!(paths.contains(&vec!["1. 인적 적용범위".to_string(), "사례 2 국회의원이 공직자등인지".to_string()]));
    // 문장 끝 문단도 패턴에 걸리면 제목이 된다 — 문장형 거르기를 켜면 올리지 않는다
    assert!(paths.iter().any(|p| p.last().map(|s| s.starts_with("사례금")).unwrap_or(false)));
    o.demote_sentence_headings = true;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert!(!c.iter().any(|x| x.path.last().map(|s| s.starts_with("사례금")).unwrap_or(false)));
    // 수준을 정수로 주면 그 수준 — 1 이면 "1. 인적 적용범위"(2) 밑이 아니라 맨 위
    let mut o2 = opts(60);
    o2.heading_patterns = vec![r"^\s*사례".to_string()];
    o2.heading_level = 1;
    let c = chunk_document(&d, &o2, &CharMeasure);
    assert!(c.iter().any(|x| x.path == vec!["사례 2 국회의원이 공직자등인지".to_string()]));
    // 원본 blocks 는 그대로
    assert_eq!(d.blocks[1].kind, B::PARAGRAPH);
}

#[test]
// 3.6 문장형 가짜 제목 → 본문, 3.7 번호로 수준 다시 매기기
fn heading_cleanup() {
    use super::headings::{demote_sentences, levels_by_number};
    let mut d = Document::new("doc", "규정");
    d.blocks.push(Block::new(B::HEADING, "제1장 총칙").lv(1));
    d.blocks.push(Block::new(B::HEADING, "4.1. 일수 계산 시, 기산은 오전 0시부터로 하며 1일 미만의 단수는 1일로 계산한다.").lv(2));
    d.blocks.push(Block::new(B::HEADING, "7.5. 준비금").lv(2));
    d.blocks.push(Block::new(B::HEADING, "출장 신청은 승인을 받아야 함.").lv(2));
    demote_sentences(&mut d, 60);
    let kinds: Vec<&str> = d.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(kinds, vec![B::HEADING, B::PARAGRAPH, B::HEADING, B::PARAGRAPH]);

    let mut d = Document::new("doc", "규정");
    for (t, lv) in [("1. 총칙", 3), ("1.2 목적", 1), ("가. 세부", 1), ("1) 항목", 1), ("나. 다른 세부", 5), ("2. 적용", 4), ("부칙", 2)] {
        d.blocks.push(Block::new(B::HEADING, t).lv(lv));
    }
    levels_by_number(&mut d);
    let lv: Vec<i64> = d.blocks.iter().map(|b| b.level).collect();
    // 번호 없는 "부칙"은 파서 수준 그대로
    assert_eq!(lv, vec![1, 2, 3, 4, 3, 1, 2]);
}

#[test]
// 3.5 문서 제목 다듬기
fn title_cleanup() {
    use super::headings::clean_title;
    assert_eq!(clean_title("03.경조사지원규정_12.01.01"), "경조사지원규정");
    assert_eq!(clean_title("25.출장여비규정_16.04.01"), "출장여비규정");
    assert_eq!(clean_title("37_청탁금지법_Q&A_사례집[1]"), "청탁금지법_Q&A_사례집");
    assert_eq!(clean_title("58_유형자산 관리 지침-2019.03.29"), "유형자산 관리 지침");
    assert_eq!(clean_title("Mpower 암호화 과정"), "Mpower 암호화 과정");
    // 다 떼면 빈 글이 되는 이름은 그대로
    assert_eq!(clean_title("2024.01.01"), "2024.01.01");
    // 접두 · body_hash 의 제목도 다듬은 제목
    let mut d = Document::new("docx", "03.경조사지원규정_12.01.01");
    d.blocks.push(Block::new(B::PARAGRAPH, "본문 글입니다."));
    let mut o = opts(128);
    o.title_clean = true;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert!(c[0].text.starts_with("[경조사지원규정]\n"));
    assert_eq!(effective_title(&d.title, &o), "경조사지원규정");
}

#[test]
// 3.2 짧은 청크 — drop(기본)은 버리고, merge 는 앞 · 뒤 청크와 합친다(같은 접두 줄은 한 번만)
fn short_merge() {
    use super::pack::finish_short;
    let mk = |t: &str, p: i64| ChunkMeta { page_from: Some(p), page_to: Some(p), page: Some(p), ..ChunkMeta::plain(t.to_string(), "parser") };
    let mut o = opts(100);
    // 글자 수에는 접두도 들어간다 — 접두 + "5만원" 은 14자라 16 미만
    o.min_tokens = 16;
    o.hard_max = 100;
    let a = mk("[문서 > 제4조]\n제4조 지급 기준은 다음과 같다", 1);
    let s = mk("[문서 > 제4조]\n5만원", 2);
    let b = mk("[문서 > 제6조]\n제6조 신청은 서면으로 한다", 3);
    // drop — 짧은 청크가 사라진다
    assert_eq!(finish_short(vec![a.clone(), s.clone(), b.clone()], &CharMeasure, &o).len(), 2);
    o.short = "merge".to_string();
    let r = finish_short(vec![a.clone(), s.clone(), b.clone()], &CharMeasure, &o);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].text, "[문서 > 제4조]\n제4조 지급 기준은 다음과 같다\n5만원");
    assert_eq!((r[0].page_from, r[0].page_to), (Some(1), Some(2)));
    // 맨 앞의 짧은 청크는 다음 청크 앞에 붙는다
    let r = finish_short(vec![s.clone(), b.clone()], &CharMeasure, &o);
    assert_eq!(r.len(), 1);
    assert!(r[0].text.starts_with("[문서 > 제4조]\n5만원\n\n[문서 > 제6조]"));
    // 어디에도 붙일 자리가 없으면(상한) 짧은 채로 남긴다 — 버리지 않는다
    o.hard_max = 20;
    o.tokens = 20;
    let r = finish_short(vec![a.clone(), s.clone(), b.clone()], &CharMeasure, &o);
    assert_eq!(r.len(), 3);
}

#[test]
// 3.3 직전 짧은 줄 겹치기 — 표 앞 "(단위 : 원)" 이 표 조각 앞에도 들어간다
fn carry_short_line() {
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::HEADING, "제1조").lv(1));
    d.blocks.push(Block::new(B::PARAGRAPH, "가나다라마바사아자차카타파하 문단입니다 길게"));
    d.blocks.push(Block::new(B::PARAGRAPH, "(단위 : 원)"));
    d.blocks.push(table(10, 1));
    let mut o = opts(60);
    let c0 = chunk_document(&d, &o, &CharMeasure);
    assert_eq!(c0.iter().filter(|c| c.text.contains("(단위 : 원)")).count(), 1);
    o.carry_short_line_chars = 20;
    let c = chunk_document(&d, &o, &CharMeasure);
    assert_eq!(c.iter().filter(|c| c.text.contains("(단위 : 원)")).count(), 2);
    assert!(c.iter().any(|x| body(&x.text).starts_with("(단위 : 원)\n| 이름 | 금액 |")));
    // 넘겨받은 줄만 든 청크는 생기지 않는다
    assert!(c.iter().all(|x| body(&x.text) != "(단위 : 원)"));
}

#[test]
// 3.4 한국어 문장 끝 — 마침표 없는 "~다 · ~함" 뒤에서도 나눈다(sentence_split 이 켜졌을 때만)
fn ko_sentence_endings() {
    let mut d = Document::new("docx", "문서");
    d.blocks.push(Block::new(B::PARAGRAPH, "첫 문장은 마침표가 없다 둘째 문장도 그렇다 셋째는 개조식으로 끝남 넷째 줄도 이어짐"));
    let mut o = opts(30);
    o.sentence_split = true;
    let basic = chunk_document(&d, &o, &CharMeasure);
    o.sentence_endings = "ko".to_string();
    let ko = chunk_document(&d, &o, &CharMeasure);
    assert_ne!(basic, ko);
    assert!(ko.iter().any(|c| body(&c.text).starts_with("첫 문장은 마침표가 없다\n둘째")));
}

#[test]
// 옵션 읽기 · 검사 · chunking 기록 — 기본값이면 0.3.x 와 같은 키만
fn new_options_json() {
    use crate::api;
    let c = ChunkConf::default();
    assert_eq!(c.to_json().as_object().unwrap().len(), 13);
    let mut d = Document::new("pdf", "사례집");
    d.blocks.push(Block::new(B::HEADING, "1. 범위").lv(1));
    d.blocks.push(Block::new(B::PARAGRAPH, "사례 1 질문"));
    d.blocks.push(Block::new(B::PARAGRAPH, "답변 본문입니다."));
    let v = api::chunk(&d.to_json(), &serde_json::json!({"unit": "chars", "min_tokens": 1, "tokens": 20, "overlap": 2,
        "heading_patterns": ["^사례"], "heading_level": "child", "short": "merge"})).unwrap();
    assert_eq!(v["chunking"]["heading_patterns"], serde_json::json!(["^사례"]));
    assert_eq!(v["chunking"]["short"], "merge");
    assert!(v["chunking"].get("heading_level").is_none());
    let paths: Vec<serde_json::Value> = v["chunks"].as_array().unwrap().iter().map(|c| c["path"].clone()).collect();
    assert!(paths.contains(&serde_json::json!(["1. 범위", "사례 1 질문"])), "{:?}", paths);
    for bad in [serde_json::json!({"unit": "chars", "heading_patterns": ["(사례"]}),
                serde_json::json!({"unit": "chars", "heading_level": "x"}),
                serde_json::json!({"unit": "chars", "heading_level": 10}),
                serde_json::json!({"unit": "chars", "short": "keep"}),
                serde_json::json!({"unit": "chars", "sentence_endings": "en"})] {
        assert_eq!(api::chunk(&d.to_json(), &bad).unwrap_err().0, api::EINVAL, "{}", bad);
    }
}
