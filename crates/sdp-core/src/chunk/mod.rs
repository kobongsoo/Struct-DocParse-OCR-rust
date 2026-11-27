//------------------------------------------------------------------
// 블록 단위 청킹 (SimpleRAG Rust 판 chunk.rs · docmodel/assemble.rs 에서 옮김 — 블록청킹 설계서 2026-11-27)
//=> 블록 문서를 검색용 청크로 자른다. SimpleRAG 와 같은 청크가 나오도록 로직은 한 줄도 바꾸지 않았다.
//   바뀐 것은 두 가지뿐이다.
//    1) 전역 설정(cfg()) 대신 ChunkOpts 를 인자로 받는다 — 호출마다 다른 크기를 줘도 스레드끼리 섞이지 않게
//    2) e5 토크나이저(Tok) 대신 Measure 트레잇을 받는다 — tokenizer.json(HfTok) 또는 글자 수(CharMeasure)
//
//   세 가지 길:
//    · structured + 블록 문서  → pack::split_blocks   (절 단위, 표는 행 경계 · 머리글 다시 붙이기)
//    · structured + 폴백 평문  → fixed::split_structured (조문 · 번호절로 절 나누기)
//    · fixed                    → fixed::split_text     (토큰 창 + 겹침)
//------------------------------------------------------------------

pub mod assemble;
pub mod fixed;
pub mod hash;
pub mod headings;
pub mod pack;
pub mod pagefind;
pub mod tok;

use serde_json::{json, Value};

use crate::docmodel::blocks::Document;
pub use tok::{CharMeasure, HfTok, Measure};

// 청크 결과가 바뀌는 코드 변경마다 올린다 — 호스트는 blocks_version 과 함께 이 값으로 다시 자를지 정한다
pub const CHUNK_VERSION: i64 = 1;

//------------------------------------------------------------------
// 청킹 옵션 한 벌 (SimpleRAG config.yaml chunk.* 와 같은 이름 · 같은 기본값)
// -필드: mode                    = "structured"(절 단위) / 그 밖("fixed")은 고정 크기
// -필드: tokens                  = 청크 목표 크기(Measure 단위 — 토큰 또는 글자)
// -필드: overlap                 = 큰 단위를 창으로 자를 때 겹침
// -필드: min_tokens              = 이보다 짧은 청크는 버림
// -필드: table_header_max_tokens = 이보다 긴 표 머리글은 조각마다 다시 붙이지 않음
// -필드: table_whole             = 표 글이 이 이하면 행으로 나누지 않음(0 = 끔)
// -필드: overflow_ratio          = 약한 초과 α — 예산 × α 안이면 단위를 통째로(1.0 = 끔)
// -필드: sentence_split          = 넘치는 줄을 문장 경계로도 나눔
// -필드: hard_max                = 절대 상한(e5 입력 한계 510)
// -필드: prefix                  = 청크 앞에 "[문서 > 절]\n" 을 붙이나(SimpleRAG 는 붙임)
// -필드: prefix_max_chars        = 접두 길이 상한(넘으면 가운데부터 '…')
// ── 0.4.0~ (청킹기법 추가검토 설계서 2026-11-27) — 모두 기본 꺼짐, 끄면 0.3.x 와 같은 청크 ──
// -필드: heading_patterns         = 이 정규식으로 시작하는 짧은 문단 · 목록을 제목으로(3.1, 빈 목록 = 끔)
// -필드: heading_max_chars        = 제목으로 볼 최대 글자 수(3.1 · 3.6 공통, 기본 60)
// -필드: heading_level            = 바꾼 제목 수준 — 0 = "child"(앞 제목의 한 단계 아래), 1~9 = 그 수준
// -필드: demote_sentence_headings = 길거나 "다." "함." 등으로 끝나는 제목을 본문으로(3.6)
// -필드: heading_level_by_number  = 제목 앞 번호(1. · 1.2 · 가. · 1) …)로 수준 다시 매기기(3.7)
// -필드: short                    = min_tokens 미만 청크 — "drop"(버림, 기본) | "merge"(앞 · 뒤 청크와 합침, 3.2)
// -필드: carry_short_line_chars   = 크기 때문에 끊을 때 직전 줄이 이 글자 수 미만이면 다음 청크 앞에도(3.3, 0 = 끔)
// -필드: sentence_endings         = sentence_split 의 문장 끝 — "basic"(. ! ? 。) | "ko"(+ 다 · 요 · 음 · 함, 3.4)
// -필드: title_clean              = 접두의 문서 제목에서 앞 번호 · 끝 날짜 · 사본 표시를 뗀다(3.5)
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ChunkOpts {
    pub mode: String,
    pub tokens: usize,
    pub overlap: usize,
    pub min_tokens: usize,
    pub table_header_max_tokens: usize,
    pub table_whole: usize,
    pub overflow_ratio: f64,
    pub sentence_split: bool,
    pub hard_max: usize,
    pub prefix: bool,
    pub prefix_max_chars: usize,
    pub heading_patterns: Vec<String>,
    pub heading_max_chars: usize,
    pub heading_level: i64,
    pub demote_sentence_headings: bool,
    pub heading_level_by_number: bool,
    pub short: String,
    pub carry_short_line_chars: usize,
    pub sentence_endings: String,
    pub title_clean: bool,
}

impl Default for ChunkOpts {
    //------------------------------------------------------------------
    // 기본 옵션 — SimpleRAG config.rs 기본값과 같게
    //=> tokens 128 · overlap max(8, 128/8)=16 · min 16 · 표 머리글 64 · 통째 표 384 · α 1.0 · 문장 나눔 끔 · 상한 510
    //
    // -in: 없음
    //
    // -out: ChunkOpts = 기본값
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    fn default() -> Self {
        ChunkOpts {
            mode: "structured".to_string(),
            tokens: 128,
            overlap: 16,
            min_tokens: 16,
            table_header_max_tokens: 64,
            table_whole: 384,
            overflow_ratio: 1.0,
            sentence_split: false,
            hard_max: 510,
            prefix: true,
            prefix_max_chars: 80,
            heading_patterns: vec![],
            heading_max_chars: 60,
            heading_level: 0,
            demote_sentence_headings: false,
            heading_level_by_number: false,
            short: "drop".to_string(),
            carry_short_line_chars: 0,
            sentence_endings: "basic".to_string(),
            title_clean: false,
        }
    }
}

//------------------------------------------------------------------
// 청킹 설정 한 벌 (열 때 옵션 "chunk" · 호출 옵션 "chunk" 가 채운다)
// -필드: tokenizer = tokenizer.json 경로(unit 이 tokens 면 필요)
// -필드: unit      = "tokens"(토크나이저 토큰 수) | "chars"(글자 수 — 크기 값이 모두 글자 수 뜻)
// -필드: opts      = 자르는 옵션
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ChunkConf {
    pub tokenizer: Option<String>,
    pub unit: String,
    pub opts: ChunkOpts,
}

impl Default for ChunkConf {
    //------------------------------------------------------------------
    // 기본 청킹 설정 — 토크나이저 없음 · 토큰 단위 · SimpleRAG 기본 크기
    //
    // -in: 없음
    //
    // -out: ChunkConf
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    fn default() -> Self {
        ChunkConf { tokenizer: None, unit: "tokens".to_string(), opts: ChunkOpts::default() }
    }
}

impl ChunkConf {
    //------------------------------------------------------------------
    // 값 검사 — 서로 어긋난 크기를 미리 막는다
    //=> 겹침이 크기 이상이면 창이 앞으로 가지 않고, 상한이 크기보다 작으면 모든 청크가 다시 잘린다.
    //
    // -in: 없음
    //
    // -out: Ok(())
    // -out: error = 어긋나면 Err(사유)
    //------------------------------------------------------------------
    pub fn validate(&self) -> Result<(), String> {
        let o = &self.opts;
        if self.unit != "tokens" && self.unit != "chars" {
            return Err(format!("chunk.unit 은 tokens | chars: {}", self.unit));
        }
        if o.mode != "structured" && o.mode != "fixed" {
            return Err(format!("chunk.mode 는 structured | fixed: {}", o.mode));
        }
        if o.tokens == 0 {
            return Err("chunk.tokens 는 1 이상".into());
        }
        if o.overlap >= o.tokens {
            return Err(format!("chunk.overlap({}) 은 chunk.tokens({}) 보다 작아야 합니다", o.overlap, o.tokens));
        }
        if o.min_tokens > o.tokens {
            return Err(format!("chunk.min_tokens({}) 가 chunk.tokens({}) 보다 큽니다", o.min_tokens, o.tokens));
        }
        if o.hard_max < o.tokens {
            return Err(format!("chunk.hard_max({}) 가 chunk.tokens({}) 보다 작습니다", o.hard_max, o.tokens));
        }
        if !(o.overflow_ratio >= 1.0) {
            return Err(format!("chunk.overflow_ratio 는 1.0 이상: {}", o.overflow_ratio));
        }
        if o.prefix_max_chars == 0 {
            return Err("chunk.prefix_max_chars 는 1 이상".into());
        }
        // 0.4.0~ 옵션 — 패턴은 미리 읽어 보고, 틀린 정규식이면 여기서 막는다
        for p in &o.heading_patterns {
            regex::Regex::new(p).map_err(|e| format!("chunk.heading_patterns 정규식 오류 {:?}: {}", p, e))?;
        }
        if o.heading_max_chars == 0 {
            return Err("chunk.heading_max_chars 는 1 이상".into());
        }
        if !(0..=9).contains(&o.heading_level) {
            return Err(format!("chunk.heading_level 은 \"child\" 또는 1~9: {}", o.heading_level));
        }
        if o.short != "drop" && o.short != "merge" {
            return Err(format!("chunk.short 는 drop | merge: {}", o.short));
        }
        if o.sentence_endings != "basic" && o.sentence_endings != "ko" {
            return Err(format!("chunk.sentence_endings 는 basic | ko: {}", o.sentence_endings));
        }
        Ok(())
    }

    //------------------------------------------------------------------
    // 결과에 적는 실제 설정 (chunking)
    //=> 크기를 바꿔 가며 비교할 때 어느 값으로 잘린 결과인지 남긴다. 토크나이저는 파일 이름만.
    //
    // -in: 없음
    //
    // -out: Value
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn to_json(&self) -> Value {
        let o = &self.opts;
        let tok = self.tokenizer.as_deref().map(|p| crate::pyutil::basename(p).to_string());
        let v = json!({
            "unit": self.unit, "mode": o.mode, "tokens": o.tokens, "overlap": o.overlap, "min_tokens": o.min_tokens,
            "table_header_max_tokens": o.table_header_max_tokens, "table_whole": o.table_whole,
            "overflow_ratio": o.overflow_ratio, "sentence_split": o.sentence_split, "hard_max": o.hard_max,
            "prefix": o.prefix, "prefix_max_chars": o.prefix_max_chars,
            "tokenizer": if self.unit == "tokens" { tok } else { None },
        });
        // 0.4.0~ 옵션은 기본값이 아닐 때만 적는다 — 기본 설정의 chunking 은 0.3.x 와 글자 하나까지 같다
        //   (chunking 을 저장해 두고 설정이 바뀌었는지 비교하는 호스트가 판을 올렸다고 다시 자르지 않게)
        let d = ChunkOpts::default();
        let extra: Vec<(&str, Value, bool)> = vec![
            ("heading_patterns", json!(o.heading_patterns), o.heading_patterns != d.heading_patterns),
            ("heading_max_chars", json!(o.heading_max_chars), o.heading_max_chars != d.heading_max_chars),
            ("heading_level", if o.heading_level == 0 { json!("child") } else { json!(o.heading_level) }, o.heading_level != d.heading_level),
            ("demote_sentence_headings", json!(o.demote_sentence_headings), o.demote_sentence_headings != d.demote_sentence_headings),
            ("heading_level_by_number", json!(o.heading_level_by_number), o.heading_level_by_number != d.heading_level_by_number),
            ("short", json!(o.short), o.short != d.short),
            ("carry_short_line_chars", json!(o.carry_short_line_chars), o.carry_short_line_chars != d.carry_short_line_chars),
            ("sentence_endings", json!(o.sentence_endings), o.sentence_endings != d.sentence_endings),
            ("title_clean", json!(o.title_clean), o.title_clean != d.title_clean),
        ];
        let mut v = v;
        if let Some(m) = v.as_object_mut() {
            for (k, x, changed) in extra {
                if changed {
                    m.insert(k.to_string(), x);
                }
            }
        }
        v
    }
}

// 읽어 둔 토크나이저 — (경로, 토크나이저). 경로가 바뀌면 다시 읽는다
static TOKENIZER: std::sync::Mutex<Option<(String, std::sync::Arc<HfTok>)>> = std::sync::Mutex::new(None);

//------------------------------------------------------------------
// 설정에 맞는 길이 재기 꺼내기
//=> chars 면 글자 수. tokens 면 tokenizer.json 을 처음 한 번만 읽어 두고 함께 쓴다(17MB · 약 0.7초).
//
// -in: c = 청킹 설정
//
// -out: Ok(길이 재기)
// -out: error = 토크나이저 경로 없음 · 읽기 실패면 Err(사유)
//------------------------------------------------------------------
pub fn measure(c: &ChunkConf) -> Result<std::sync::Arc<dyn Measure>, String> {
    if c.unit == "chars" {
        return Ok(std::sync::Arc::new(CharMeasure));
    }
    let path = c.tokenizer.as_deref()
        .ok_or("chunk.tokenizer 가 없습니다 — tokenizer.json 경로를 주거나 chunk.unit 을 chars 로 하세요")?;
    let mut g = TOKENIZER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, t)) = g.as_ref() {
        if p == path {
            return Ok(t.clone());
        }
    }
    let t = std::sync::Arc::new(HfTok::from_file(path).map_err(|e| format!("chunk.tokenizer 를 읽지 못함({}): {}", path, e))?);
    *g = Some((path.to_string(), t.clone()));
    Ok(t)
}

// 토크나이저를 읽어 두었나 (상태 JSON 용)
pub fn tokenizer_loaded() -> Option<String> {
    TOKENIZER.lock().ok().and_then(|g| g.as_ref().map(|(p, _)| p.clone()))
}

//------------------------------------------------------------------
// 청크 하나 + 출처 정보 (SimpleRAG ChunkMeta 와 같은 필드)
// -필드: text       = 청크 글(접두 "[문서 > 절]\n" 포함 — 임베딩 · 검색 · LLM 에 들어가는 글 그대로)
// -필드: section_id = 첫 절 번호(폴백 평문 · 고정 크기면 None)
// -필드: path       = 첫 절의 제목 경로
// -필드: source     = 첫 절 제목의 출처(parser · inferred · regex · ocr · extra), 폴백 평문 "legacy", 고정 크기 ""
// -필드: page_from, page_to = 청크가 걸친 쪽 범위
// -필드: page       = 청크가 실제로 시작하는 쪽(절 글 대조로 찾음, 못 찾으면 None)
//------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkMeta {
    pub text: String,
    pub section_id: Option<i64>,
    pub path: Vec<String>,
    pub source: String,
    pub page_from: Option<i64>,
    pub page_to: Option<i64>,
    pub page: Option<i64>,
}

impl ChunkMeta {
    //------------------------------------------------------------------
    // 쪽 · 절 정보 없는 청크 (폴백 평문 · 고정 크기)
    //
    // -in: text   = 청크 글
    // -in: source = 출처 글("legacy" 또는 "")
    //
    // -out: ChunkMeta
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn plain(text: String, source: &str) -> ChunkMeta {
        ChunkMeta { text, section_id: None, path: vec![], source: source.to_string(), page_from: None, page_to: None, page: None }
    }

    //------------------------------------------------------------------
    // 결과 JSON 한 항목 (SimpleRAG dump_chunks 와 같은 모양)
    //
    // -in: no    = 청크 순번(0부터)
    // -in: m     = 길이 재기(tokens 값)
    // -in: title = 문서 제목 — body_hash 를 만들 때 접두 줄을 알아보는 데 쓴다
    //
    // -out: {"no","tokens","section_id","path","source","page","page_from","page_to","hash","body_hash","text"}
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    pub fn to_json(&self, no: usize, m: &dyn Measure, title: &str) -> Value {
        // 0.3.1~ 중복 알아보기 지문 — hash = text 그대로, body_hash = 접두 떼고 공백 정리한 본문
        let (h, bh) = hash::hashes(&self.text, title);
        json!({
            "no": no,
            "tokens": m.tlen(&self.text),
            "section_id": self.section_id,
            "path": self.path,
            "source": self.source,
            "page": self.page,
            "page_from": self.page_from,
            "page_to": self.page_to,
            "hash": h,
            "body_hash": bh,
            "text": self.text,
        })
    }
}

//------------------------------------------------------------------
// 문서 하나 청킹 (입구)
//=> SimpleRAG indexer::extract_blocks_keep 의 청킹 부분과 같다.
//    1) 문서 사본에서 돈다 — 절 조립이 블록 path 를 채우므로, 원본 블록(결과 JSON 의 blocks)은 그대로 둔다
//    2) structured 면 split_blocks(폴백 평문은 그 안에서 split_structured), 아니면 전체 글을 split_text
//    3) 제목은 문서 제목(doc.title — sdp_parse 의 title 옵션이 이미 들어 있다)
//
// -in: doc = 추출 결과
// -in: o   = 청킹 옵션
// -in: m   = 길이 재기(토큰 또는 글자)
//
// -out: Vec<ChunkMeta> — 글 없는 문서면 빈 목록
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn chunk_document(doc: &Document, o: &ChunkOpts, m: &dyn Measure) -> Vec<ChunkMeta> {
    if o.mode == "structured" {
        // 절 조립이 블록 path 를 쓰므로 사본에서
        let mut d = doc.clone();
        // 0.4.0~ 제목 다듬기(패턴 제목 · 문장형 제목 · 번호 깊이) — 사본에서만, 결과의 blocks 는 그대로
        headings::adjust(&mut d, o);
        let title = effective_title(&doc.title, o);
        return pack::split_blocks(&mut d, m, o, Some(&title));
    }
    // 고정 크기 — SimpleRAG 와 같이 출처는 빈 글
    fixed::split_text(&doc.full_text(), m, o, None, None).into_iter().map(|t| ChunkMeta::plain(t, "")).collect()
}

//------------------------------------------------------------------
// 접두에 쓰는 문서 제목 — title_clean 이면 다듬은 제목(0.4.0~)
//=> body_hash 의 접두 줄 판정도 이 제목으로 해야 하므로 결과 JSON 을 만들 때 같은 함수를 쓴다.
//
// -in: title = 문서 제목(doc.title)
// -in: o     = 청킹 옵션
//
// -out: 실제 접두에 쓴 제목
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn effective_title(title: &str, o: &ChunkOpts) -> String {
    if o.title_clean {
        headings::clean_title(title)
    } else {
        title.to_string()
    }
}

//------------------------------------------------------------------
// 청크 목록 → JSON 배열
//
// -in: chunks = chunk_document 결과
// -in: m      = 길이 재기
// -in: title  = 문서 제목(doc.title — body_hash 의 접두 줄 판정)
//
// -out: Value::Array
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn chunks_json(chunks: &[ChunkMeta], m: &dyn Measure, title: &str) -> Value {
    Value::Array(chunks.iter().enumerate().map(|(i, c)| c.to_json(i, m, title)).collect())
}

#[cfg(test)]
mod tests;
