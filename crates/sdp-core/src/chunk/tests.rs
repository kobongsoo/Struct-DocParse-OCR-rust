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
    let j = chunks_json(&c, &CharMeasure);
    let o = j[0].as_object().unwrap();
    let keys: Vec<&str> = o.keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["no", "tokens", "section_id", "path", "source", "page", "page_from", "page_to", "text"]);
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
    let direct = chunks_json(&chunk_document(&d, &o, &CharMeasure), &CharMeasure);
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
