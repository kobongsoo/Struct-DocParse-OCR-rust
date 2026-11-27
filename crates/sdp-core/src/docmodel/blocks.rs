//------------------------------------------------------------------
// 블록 모델 (Python docmodel/blocks.py)
//=> 추출 결과를 '한 덩어리 문자열'이 아니라 종류·제목 수준·쪽이 붙은 블록 목록으로 담는다.
//   포맷별 추출기는 모두 이 모양으로 결과를 내고, 청킹기는 이 모양만 받는다.
//------------------------------------------------------------------

use serde_json::{json, Map, Value};

pub const HEADING: &str = "heading";
pub const PARAGRAPH: &str = "paragraph";
pub const LIST_ITEM: &str = "list_item";
pub const TABLE: &str = "table";
pub const CAPTION: &str = "caption";
pub const CODE: &str = "code";
pub const HEADER: &str = "header";
pub const FOOTER: &str = "footer";
pub const PAGE_NUMBER: &str = "page_number";
pub const TOC: &str = "toc";
pub const NOTE: &str = "note";

// 청크에 넣지 않는 종류(진단 JSON 에는 남는다)
pub fn excluded(t: &str) -> bool {
    matches!(t, HEADER | FOOTER | PAGE_NUMBER | TOC)
}

pub const SRC_PARSER: &str = "parser";
pub const SRC_INFERRED: &str = "inferred";
pub const SRC_REGEX: &str = "regex";
// 스캔 쪽·그림 파일을 OCR 로 읽은 글 (OCR 설계서)
pub const SRC_OCR: &str = "ocr";
// 본문 밖 글 — PDF 글상자 · 서식 칸 값 · docx 메모 · SmartArt (본문밖글읽기 설계서)
pub const SRC_EXTRA: &str = "extra";

//------------------------------------------------------------------
// 블록 하나
// -필드: kind        = 블록 종류(위 상수)
// -필드: text        = 블록 글자(표는 후처리에서 렌더링 글이 채워진다)
// -필드: level       = 제목 수준 1~9 / 목록 깊이. 그 밖 0
// -필드: rows        = 표만 — 행 × 칸 글자
// -필드: header_rows = 표만 — 머리글 행 수
// -필드: page        = 시작 쪽·슬라이드·시트 번호(1부터)
// -필드: path        = 속한 제목 경로(절 조립에서 채움)
// -필드: source      = parser | inferred | regex | ocr | extra
// -필드: synthetic   = 문서에 없는 글자(제목 없는 슬라이드의 "슬라이드 3" 등)
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Block {
    pub kind: &'static str,
    pub text: String,
    pub level: i64,
    pub rows: Option<Vec<Vec<String>>>,
    pub header_rows: usize,
    pub page: Option<i64>,
    pub path: Vec<String>,
    pub source: &'static str,
    pub synthetic: bool,
}

impl Block {
    // 기본값으로 블록 만들기
    pub fn new(kind: &'static str, text: impl Into<String>) -> Block {
        Block {
            kind,
            text: text.into(),
            level: 0,
            rows: None,
            header_rows: 0,
            page: None,
            path: vec![],
            source: SRC_PARSER,
            synthetic: false,
        }
    }
    pub fn lv(mut self, level: i64) -> Block {
        self.level = level;
        self
    }
    pub fn pg(mut self, page: Option<i64>) -> Block {
        self.page = page;
        self
    }
    pub fn src(mut self, s: &'static str) -> Block {
        self.source = s;
        self
    }
    // 표 블록
    pub fn table(rows: Vec<Vec<String>>, header_rows: usize) -> Block {
        let mut b = Block::new(TABLE, "");
        b.rows = Some(rows);
        b.header_rows = header_rows;
        b
    }

    //------------------------------------------------------------------
    // 진단 JSON 용 사전 — 기본값 필드는 뺀다
    //------------------------------------------------------------------
    pub fn to_json(&self) -> Value {
        let mut d = Map::new();
        d.insert("type".into(), json!(self.kind));
        d.insert("text".into(), json!(self.text));
        if self.level != 0 {
            d.insert("level".into(), json!(self.level));
        }
        if let Some(r) = &self.rows {
            d.insert("rows".into(), json!(r));
            d.insert("header_rows".into(), json!(self.header_rows));
        }
        if let Some(p) = self.page {
            d.insert("page".into(), json!(p));
        }
        if !self.path.is_empty() {
            d.insert("path".into(), json!(self.path));
        }
        if self.source != SRC_PARSER {
            d.insert("source".into(), json!(self.source));
        }
        if self.synthetic {
            d.insert("synthetic".into(), json!(true));
        }
        Value::Object(d)
    }
}

//------------------------------------------------------------------
// 문서 하나
// -필드: format   = docx | pptx | xlsx | hwpx | html | md | ipynb | doc | pdf | legacy
// -필드: title    = 문서 제목(파일명에서 확장자 뗀 것)
// -필드: blocks   = 블록 목록(읽기 순서)
// -필드: raw_text = 폴백 경로의 평문(블록 경로면 None)
// -필드: pages    = 쪽·슬라이드·시트 수
// -필드: warnings = 경고 문장
//------------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct Document {
    pub format: String,
    pub title: String,
    pub blocks: Vec<Block>,
    pub raw_text: Option<String>,
    pub pages: Option<i64>,
    pub warnings: Vec<String>,
}

impl Document {
    pub fn new(format: &str, title: &str) -> Document {
        Document { format: format.into(), title: title.into(), ..Default::default() }
    }

    // 블록 경로인가(폴백 평문이 아니면)
    pub fn is_blocks(&self) -> bool {
        self.raw_text.is_none()
    }

    //------------------------------------------------------------------
    // 문서 전체 글자 — 블록 경로면 합성이 아닌 모든 블록 글자를 줄로 잇는다
    //------------------------------------------------------------------
    pub fn full_text(&self) -> String {
        if let Some(r) = &self.raw_text {
            return r.clone();
        }
        self.blocks.iter().filter(|b| !b.text.is_empty() && !b.synthetic).map(|b| b.text.as_str())
            .collect::<Vec<_>>().join("\n")
    }

    // 진단 JSON
    pub fn to_json(&self) -> Value {
        json!({
            "format": self.format, "title": self.title, "pages": self.pages,
            "warnings": self.warnings,
            "raw_text": self.raw_text,
            "blocks": self.blocks.iter().map(|b| b.to_json()).collect::<Vec<_>>(),
        })
    }

    //------------------------------------------------------------------
    // 결과 JSON → 문서 (sdp_chunk 가 저장해 둔 sdp_parse 결과를 다시 읽을 때)
    //=> to_json · api::doc_json 의 반대. SimpleRAG docparse::doc_from_json 과 같은 규칙이다.
    //    1) format · title · pages · raw_text · warnings 를 읽는다
    //    2) 블록마다 type(모르는 라벨은 문단) · text · level · page · source · rows · header_rows · synthetic
    //
    // -in: v = sdp_parse 결과 JSON
    //
    // -out: Ok(Document)
    // -out: error = 객체가 아니거나 blocks · raw_text 가 둘 다 없으면 Err(사유)
    //------------------------------------------------------------------
    pub fn from_json(v: &Value) -> Result<Document, String> {
        let m = v.as_object().ok_or("doc_json 은 sdp_parse 결과 객체여야 합니다")?;
        let raw = m.get("raw_text").and_then(|x| x.as_str()).map(|x| x.to_string());
        let blocks = m.get("blocks").and_then(|x| x.as_array());
        if blocks.is_none() && raw.is_none() {
            return Err("doc_json 에 blocks 도 raw_text 도 없습니다".into());
        }
        let s = |k: &str| m.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut doc = Document::new(&s("format"), &s("title"));
        doc.pages = m.get("pages").and_then(|x| x.as_i64());
        doc.raw_text = raw;
        doc.warnings = m.get("warnings").and_then(|x| x.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        for b in blocks.into_iter().flatten() {
            let t = b.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let mut blk = Block::new(kind_of(t), b.get("text").and_then(|x| x.as_str()).unwrap_or(""))
                .lv(b.get("level").and_then(|x| x.as_i64()).unwrap_or(0))
                .pg(b.get("page").and_then(|x| x.as_i64()))
                .src(source_of(b.get("source").and_then(|x| x.as_str())));
            // 표는 칸 글과 머리글 행 수를 함께 — 청킹이 행 경계 · 머리글을 쓴다
            if let Some(rows) = b.get("rows") {
                blk.rows = serde_json::from_value(rows.clone()).ok();
                blk.header_rows = b.get("header_rows").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            }
            blk.synthetic = b.get("synthetic").and_then(|x| x.as_bool()).unwrap_or(false);
            doc.blocks.push(blk);
        }
        Ok(doc)
    }
}

// 라벨 글 → 블록 종류 상수 (모르는 라벨은 문단으로)
fn kind_of(s: &str) -> &'static str {
    match s {
        "heading" => HEADING,
        "list_item" => LIST_ITEM,
        "table" => TABLE,
        "caption" => CAPTION,
        "code" => CODE,
        "header" => HEADER,
        "footer" => FOOTER,
        "page_number" => PAGE_NUMBER,
        "toc" => TOC,
        "note" => NOTE,
        _ => PARAGRAPH,
    }
}

// 출처 글 → 상수 (없거나 모르면 parser)
fn source_of(s: Option<&str>) -> &'static str {
    match s {
        Some("inferred") => SRC_INFERRED,
        Some("regex") => SRC_REGEX,
        Some("ocr") => SRC_OCR,
        Some("extra") => SRC_EXTRA,
        _ => SRC_PARSER,
    }
}
