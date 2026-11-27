//------------------------------------------------------------------
// 바깥에 내놓는 기능 (설계서 §7) — FFI 껍데기(sdp-ffi)와 sdp-cli 가 이것만 부른다
//=> 결과는 모두 serde_json::Value. 실패는 (상태 코드, 오류문).
//------------------------------------------------------------------

use serde_json::{json, Map, Value};

use crate::chunk::{self, ChunkConf, CHUNK_VERSION};
use crate::config::{self, cfg, BLOCKS_VERSION};
use crate::docmodel::{self, blocks::Document};
use crate::ocr;
use crate::pyutil;

// 상태 코드 (설계서 §7 표와 같은 값)
pub const OK: i32 = 0;
pub const EINVAL: i32 = -1;
pub const EIO: i32 = -2;
pub const EPARSE: i32 = -3;
pub const EUNSUPPORTED: i32 = -4;
pub const EOCR: i32 = -5;
pub const ECANCELLED: i32 = -6;
pub const ERUNTIME: i32 = -7;
pub const EPANIC: i32 = -99;

// 결과 JSON 모양 버전
pub const SCHEMA: i64 = 1;
// 바깥 함수 모양(ABI) 버전 — 함수를 빼거나 인자를 바꿀 때만 올린다
pub const ABI_VERSION: u32 = 1;

pub type ApiResult = Result<Value, (i32, String)>;

// 폴백 평문 추출로 받는 확장자 (블록 경로가 아닌 것)
const LEGACY_EXTS: &[&str] = &[".hwp", ".ppt", ".xls", ".txt", ".csv", ".py", ".text", ".log"];

//------------------------------------------------------------------
// 버전 문자열 — "0.1.0 blocks=5 ocr=<엔진>"
//
// -in: 없음
//
// -out: String
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn version() -> String {
    // chunk= 가 있으면 청킹을 지원한다(0.3.0~) — 호스트는 이것으로 지원 여부를 안다
    format!("{} blocks={} chunk={} ocr={}", env!("CARGO_PKG_VERSION"), BLOCKS_VERSION, CHUNK_VERSION, ocr::ENGINE)
}

//------------------------------------------------------------------
// 열 때 옵션 JSON → Options (설계서 §7 모양)
//=> 빠진 키는 기본값. 모르는 최상위 키는 오류(오타를 조용히 넘기지 않게).
//
// -in: j = {"ocr":{...}, "pdf":{...}, "runtime":{...}, "log":{...}}
//
// -out: Ok(Options)
// -out: error = 모양이 틀리면 Err((EINVAL, 사유))
//------------------------------------------------------------------
pub fn options_from_json(j: &Value) -> Result<config::Options, (i32, String)> {
    let mut o = config::Options::default();
    let Some(m) = j.as_object() else {
        if j.is_null() {
            return Ok(o);
        }
        return Err((EINVAL, "옵션은 JSON 객체여야 합니다".into()));
    };
    for (k, v) in m {
        match k.as_str() {
            "ocr" => apply_ocr(&mut o, v)?,
            "pdf" => {
                for (k2, v2) in obj(v, "pdf")? {
                    match k2.as_str() {
                        "tables" => o.pdf_tables = boolv(v2, "pdf.tables")?,
                        "infer_headings" => o.pdf_infer_headings = boolv(v2, "pdf.infer_headings")?,
                        _ => return Err((EINVAL, format!("모르는 옵션: pdf.{}", k2))),
                    }
                }
            }
            "runtime" => {
                for (k2, v2) in obj(v, "runtime")? {
                    match k2.as_str() {
                        "pdfium" => o.pdfium_path = strv(v2),
                        "onnxruntime" => o.ort_path = strv(v2),
                        "snf" => o.cso_snf = strv(v2),
                        _ => return Err((EINVAL, format!("모르는 옵션: runtime.{}", k2))),
                    }
                }
            }
            // 본문 밖 글 읽기(기본 켬) — 끄면 0.1.x 와 같은 본문
            "extra_text" => o.extra_text = boolv(v, "extra_text")?,
            // 청킹 기본값(0.3.0~) — 켜는 것은 호출마다
            "chunk" => {
                apply_chunk(&mut o.chunk, v, false)?;
                o.chunk.validate().map_err(|e| (EINVAL, e))?;
            }
            // 로그 수준은 FFI 껍데기가 따로 읽는다
            "log" => {}
            _ => return Err((EINVAL, format!("모르는 옵션: {}", k))),
        }
    }
    Ok(o)
}

// ocr 묶음 옵션 적용 (열 때 · 실행 중 바꾸기 공통)
fn apply_ocr(o: &mut config::Options, v: &Value) -> Result<(), (i32, String)> {
    for (k, v) in obj(v, "ocr")? {
        match k.as_str() {
            "enabled" => o.ocr = boolv(v, "ocr.enabled")?,
            "images" => o.ocr_images = boolv(v, "ocr.images")?,
            "mode" => {
                o.ocr_inline = match v.as_str() {
                    Some("inline") => true,
                    Some("deferred") => false,
                    _ => return Err((EINVAL, "ocr.mode 는 inline | deferred".into())),
                }
            }
            "model_dir" => {
                if let Some(s) = strv(v) {
                    o.ocr_dir = s;
                }
            }
            "cache_dir" => o.ocr_cache_dir = strv(v),
            "threads" => o.ocr_threads = intv(v, "ocr.threads")?.clamp(1, 64),
            "max_pages" => o.ocr_max_pages = intv(v, "ocr.max_pages")?.max(1),
            "layout" => o.ocr_layout = boolv(v, "ocr.layout")?,
            "arena" => o.ocr_arena = boolv(v, "ocr.arena")?,
            // 큰 쪽 상한 — "a4" 켬 / "none" 끔
            "page_cap" => {
                o.ocr_page_cap = match v.as_str() {
                    Some("a4") => true,
                    Some("none") => false,
                    _ => return Err((EINVAL, "ocr.page_cap 은 a4 | none".into())),
                }
            }
            "image_cap" => o.ocr_image_cap = boolv(v, "ocr.image_cap")?,
            // 작은 쪽 확대 — A4 보다 작은 PDF 쪽을 최대 1.4배로 그려 OCR
            "page_up" => o.ocr_page_up = boolv(v, "ocr.page_up")?,
            // 쪽 방향 보정 — orient 끄면 이전과 같음, orient_flip 끄면 뒤집힘(180°) 확인 생략
            "orient" => o.ocr_orient = boolv(v, "ocr.orient")?,
            "orient_flip" => o.ocr_orient_flip = boolv(v, "ocr.orient_flip")?,
            // 지금 DPI 는 고정(200) — 캐시 좌표가 이 값에 묶여 있다
            "dpi" => {}
            _ => return Err((EINVAL, format!("모르는 옵션: ocr.{}", k))),
        }
    }
    Ok(())
}

//------------------------------------------------------------------
// chunk 묶음 옵션 적용 (열 때 · 실행 중 · 호출마다 공통)
//=> overlap 을 주지 않고 tokens 만 바꾸면 overlap = max(8, tokens/8) — SimpleRAG config 와 같은 규칙.
//
// -in: c             = 바꿀 청킹 설정
// -in: v             = {"tokenizer", "unit", "mode", "tokens", …} (호출 옵션이면 "enabled" 도)
// -in: allow_enabled = "enabled" 키를 받나(호출 옵션 · sdp_chunk 만)
//
// -out: Ok(enabled) — "enabled" 를 주었으면 그 값, 아니면 None
// -out: error = 모르는 키 · 틀린 값이면 Err((EINVAL, 사유))
//------------------------------------------------------------------
fn apply_chunk(c: &mut ChunkConf, v: &Value, allow_enabled: bool) -> Result<Option<bool>, (i32, String)> {
    let m = obj(v, "chunk")?;
    let mut enabled = None;
    // 0 이상 정수 읽기
    let uint = |v: &Value, name: &str| -> Result<usize, (i32, String)> {
        v.as_u64().map(|x| x as usize).ok_or_else(|| (EINVAL, format!("{} 는 0 이상 정수", name)))
    };
    // 글 읽기
    let text = |v: &Value, name: &str| -> Result<String, (i32, String)> {
        v.as_str().map(|s| s.to_string()).ok_or_else(|| (EINVAL, format!("{} 는 글", name)))
    };
    for (k, v) in m {
        let o = &mut c.opts;
        match k.as_str() {
            "enabled" if allow_enabled => enabled = Some(boolv(v, "chunk.enabled")?),
            "tokenizer" => c.tokenizer = strv(v),
            "unit" => c.unit = text(v, "chunk.unit")?,
            "mode" => o.mode = text(v, "chunk.mode")?,
            "tokens" => {
                o.tokens = uint(v, "chunk.tokens")?;
                // 겹침을 따로 주지 않았으면 크기를 따라간다
                if !m.contains_key("overlap") {
                    o.overlap = std::cmp::max(8, o.tokens / 8);
                }
            }
            "overlap" => o.overlap = uint(v, "chunk.overlap")?,
            "min_tokens" => o.min_tokens = uint(v, "chunk.min_tokens")?,
            "table_header_max_tokens" => o.table_header_max_tokens = uint(v, "chunk.table_header_max_tokens")?,
            "table_whole" => o.table_whole = uint(v, "chunk.table_whole")?,
            "overflow_ratio" => o.overflow_ratio = v.as_f64().ok_or_else(|| (EINVAL, "chunk.overflow_ratio 는 수".to_string()))?,
            "sentence_split" => o.sentence_split = boolv(v, "chunk.sentence_split")?,
            "hard_max" => o.hard_max = uint(v, "chunk.hard_max")?,
            "prefix" => o.prefix = boolv(v, "chunk.prefix")?,
            "prefix_max_chars" => o.prefix_max_chars = uint(v, "chunk.prefix_max_chars")?,
            // ── 0.4.0~ (청킹기법 추가검토 설계서) ──
            "heading_patterns" => {
                let a = v.as_array().ok_or_else(|| (EINVAL, "chunk.heading_patterns 는 글 배열".to_string()))?;
                o.heading_patterns = a.iter().map(|x| x.as_str().map(|s| s.to_string())
                    .ok_or_else(|| (EINVAL, "chunk.heading_patterns 는 글 배열".to_string()))).collect::<Result<_, _>>()?;
            }
            "heading_max_chars" => o.heading_max_chars = uint(v, "chunk.heading_max_chars")?,
            // "child" = 앞 제목의 한 단계 아래 / 1~9 = 그 수준
            "heading_level" => {
                o.heading_level = match v {
                    Value::String(s) if s == "child" => 0,
                    _ => v.as_i64().filter(|x| (1..=9).contains(x))
                        .ok_or_else(|| (EINVAL, "chunk.heading_level 은 \"child\" 또는 1~9".to_string()))?,
                }
            }
            "demote_sentence_headings" => o.demote_sentence_headings = boolv(v, "chunk.demote_sentence_headings")?,
            "heading_level_by_number" => o.heading_level_by_number = boolv(v, "chunk.heading_level_by_number")?,
            "short" => o.short = text(v, "chunk.short")?,
            "carry_short_line_chars" => o.carry_short_line_chars = uint(v, "chunk.carry_short_line_chars")?,
            "sentence_endings" => o.sentence_endings = text(v, "chunk.sentence_endings")?,
            "title_clean" => o.title_clean = boolv(v, "chunk.title_clean")?,
            _ => return Err((EINVAL, format!("모르는 옵션: chunk.{}", k))),
        }
    }
    Ok(enabled)
}

//------------------------------------------------------------------
// 호출마다 청킹 설정 풀기 — 열 때 기본값 위에 이 호출의 값을 덮는다
//=> true = 켬(기본값 그대로) / false · null = 끔 / 객체 = 켬 + 덮어쓰기("enabled": false 면 끔)
//
// -in: v = call_opts.chunk 값
//
// -out: Ok(Some(설정)) = 켬 / Ok(None) = 끔
// -out: error = 모양 · 값이 틀리면 Err((EINVAL, 사유))
//------------------------------------------------------------------
fn resolve_chunk(v: &Value) -> Result<Option<ChunkConf>, (i32, String)> {
    let mut c = cfg().chunk.clone();
    match v {
        Value::Null | Value::Bool(false) => return Ok(None),
        Value::Bool(true) => {}
        Value::Object(_) => {
            if apply_chunk(&mut c, v, true)? == Some(false) {
                return Ok(None);
            }
        }
        _ => return Err((EINVAL, "chunk 는 true/false 또는 객체".into())),
    }
    c.validate().map_err(|e| (EINVAL, e))?;
    Ok(Some(c))
}

//------------------------------------------------------------------
// 문서 하나 청킹 → chunk_version · chunking · chunks 를 결과 JSON 에 붙인다
//
// -in: out = 결과 JSON(객체)
// -in: doc = 추출 결과
// -in: c   = 청킹 설정
//
// -out: Ok(())
// -out: error = 토크나이저를 못 읽으면 Err((EINVAL, 사유))
//------------------------------------------------------------------
fn attach_chunks(out: &mut Value, doc: &Document, c: &ChunkConf) -> Result<(), (i32, String)> {
    let m = chunk::measure(c).map_err(|e| (EINVAL, e))?;
    let chunks = chunk::chunk_document(doc, &c.opts, m.as_ref());
    if let Some(o) = out.as_object_mut() {
        o.insert("chunk_version".into(), json!(CHUNK_VERSION));
        o.insert("chunking".into(), c.to_json());
        o.insert("chunks".into(), chunk::chunks_json(&chunks, m.as_ref(), &chunk::effective_title(&doc.title, &c.opts)));
    }
    Ok(())
}

//------------------------------------------------------------------
// 저장해 둔 파싱 결과를 다시 청킹 (sdp_chunk)
//=> 파싱 · OCR 없이 크기만 바꿔 다시 자른다. opts 는 호출 옵션의 chunk 객체와 같은 모양("enabled" 는 무시).
//
// -in: doc_json = sdp_parse 결과(blocks · raw_text · title · format)
// -in: opts     = 청킹 옵션 객체 또는 null(열 때 기본값)
//
// -out: Ok({"chunk_version", "chunking", "chunks"})
// -out: error = 모양이 틀리거나 토크나이저를 못 읽으면 Err((EINVAL, 사유))
//------------------------------------------------------------------
pub fn chunk(doc_json: &Value, opts: &Value) -> ApiResult {
    let mut c = cfg().chunk.clone();
    if !opts.is_null() {
        apply_chunk(&mut c, opts, true)?;
    }
    c.validate().map_err(|e| (EINVAL, e))?;
    let doc = Document::from_json(doc_json).map_err(|e| (EINVAL, e))?;
    let mut out = json!({});
    attach_chunks(&mut out, &doc, &c)?;
    Ok(out)
}

// JSON 객체로 읽기
fn obj<'a>(v: &'a Value, name: &str) -> Result<&'a Map<String, Value>, (i32, String)> {
    v.as_object().ok_or_else(|| (EINVAL, format!("{} 는 객체여야 합니다", name)))
}
// 참/거짓 읽기
fn boolv(v: &Value, name: &str) -> Result<bool, (i32, String)> {
    v.as_bool().ok_or_else(|| (EINVAL, format!("{} 는 true/false", name)))
}
// 정수 읽기
fn intv(v: &Value, name: &str) -> Result<i64, (i32, String)> {
    v.as_i64().ok_or_else(|| (EINVAL, format!("{} 는 정수", name)))
}
// 문자열 읽기 — 빈 문자열·null 은 None
fn strv(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(|s| s.to_string())
}

//------------------------------------------------------------------
// 실행 중 옵션 하나 바꾸기 (sdp_set_option)
//=> 키는 "ocr.enabled" 처럼 점으로 잇는다. OCR 을 끄면 엔진도 내려 메모리를 돌려준다.
//
// -in: key   = 옵션 이름(예: "ocr.enabled", "pdf.tables")
// -in: value = JSON 값
//
// -out: Ok(())
// -out: error = 모르는 키·틀린 값이면 Err((EINVAL, 사유))
//------------------------------------------------------------------
pub fn set_option(key: &str, value: &Value) -> Result<(), (i32, String)> {
    // 청킹 기본값 하나 — 바꾼 뒤 서로 어긋나지 않는지 본다
    if let Some(name) = key.strip_prefix("chunk.") {
        let mut o = (*cfg()).clone();
        let mut m = Map::new();
        m.insert(name.to_string(), value.clone());
        apply_chunk(&mut o.chunk, &Value::Object(m), false)?;
        o.chunk.validate().map_err(|e| (EINVAL, e))?;
        config::set(o);
        return Ok(());
    }
    // 묶음 없는 맨 위 옵션 — 지금은 extra_text 하나
    if key == "extra_text" {
        let mut o = (*cfg()).clone();
        o.extra_text = boolv(value, "extra_text")?;
        config::set(o);
        return Ok(());
    }
    let (sec, name) = key.split_once('.').ok_or_else(|| (EINVAL, format!("옵션 이름은 '묶음.이름': {}", key)))?;
    let mut o = (*cfg()).clone();
    let mut m = Map::new();
    m.insert(name.to_string(), value.clone());
    let wrapped = Value::Object(m);
    match sec {
        "ocr" => apply_ocr(&mut o, &wrapped)?,
        "pdf" | "runtime" => {
            let mut all = Map::new();
            all.insert(sec.to_string(), wrapped);
            let n = options_from_json(&Value::Object(all))?;
            if sec == "pdf" {
                o.pdf_tables = n.pdf_tables;
                o.pdf_infer_headings = n.pdf_infer_headings;
                // 한쪽만 바꾸면 다른 쪽은 기본값이 아니라 지금 값을 유지해야 한다
                if name != "tables" {
                    o.pdf_tables = cfg().pdf_tables;
                }
                if name != "infer_headings" {
                    o.pdf_infer_headings = cfg().pdf_infer_headings;
                }
            } else {
                match name {
                    "pdfium" => o.pdfium_path = n.pdfium_path,
                    "onnxruntime" => o.ort_path = n.ort_path,
                    _ => o.cso_snf = n.cso_snf,
                }
            }
        }
        _ => return Err((EINVAL, format!("모르는 옵션: {}", key))),
    }
    let off = !o.ocr;
    config::set(o);
    if off {
        ocr::unload();
    }
    Ok(())
}

//------------------------------------------------------------------
// OCR 상태 — 켜짐 여부와 꺼진 이유
//
// -in: 없음
//
// -out: (켜짐, 이유) — 이유 = None | "off" | "models_missing"
// -out: error = 예외 없음
//------------------------------------------------------------------
fn ocr_state() -> (bool, Option<&'static str>) {
    if !cfg().ocr {
        return (false, Some("off"));
    }
    if ocr::models_present().is_err() {
        return (false, Some("models_missing"));
    }
    (true, None)
}

//------------------------------------------------------------------
// 상태 JSON (sdp_get_status)
//
// -in: 없음
//
// -out: {version, abi, blocks_version, extra_text, ocr:{configured, enabled, images, reason, mode, engine, model_dir, cache_dir, threads, max_pages, layout, arena, page_cap, image_cap}, pdfium:{ok, error}}
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn status() -> Value {
    let c = cfg();
    let (on, why) = ocr_state();
    let pdf = crate::pdfium::available();
    json!({
        "version": version(), "abi": ABI_VERSION, "blocks_version": BLOCKS_VERSION, "schema": SCHEMA,
        "ocr": {
            "configured": c.ocr, "enabled": on, "images": on && c.ocr_images, "reason": why,
            "mode": if c.ocr_inline { "inline" } else { "deferred" }, "engine": ocr::ENGINE,
            "model_dir": c.ocr_dir, "cache_dir": c.ocr_cache_dir, "threads": c.ocr_threads, "max_pages": c.ocr_max_pages,
            "layout": c.ocr_layout, "arena": c.ocr_arena,
            "page_cap": if c.ocr_page_cap { "a4" } else { "none" }, "image_cap": c.ocr_image_cap, "page_up": c.ocr_page_up,
            "orient": c.ocr_orient, "orient_flip": c.ocr_orient_flip,
            "image_exts": ocr::image::IMAGE_EXTS,
        },
        "pdfium": {"ok": pdf.is_ok(), "error": pdf.err()},
        "extra_text": c.extra_text,
        // 청킹 기본값 · 읽어 둔 토크나이저(없으면 null)
        "chunk": {"defaults": c.chunk.to_json(), "tokenizer_path": c.chunk.tokenizer, "tokenizer_loaded": chunk::tokenizer_loaded()},
    })
}

//------------------------------------------------------------------
// 이 파일을 다룰 수 있나 (sdp_supports)
//
// -in: path = 파일 경로(확장자만 본다)
//
// -out: 1 = 블록 경로 / 2 = 폴백 평문 / 0 = 지원 안 함
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn supports(path: &str) -> i32 {
    if docmodel::supports(path) {
        return 1;
    }
    if LEGACY_EXTS.contains(&pyutil::ext_lower(path).as_str()) {
        return 2;
    }
    0
}

//------------------------------------------------------------------
// 문서 → 결과 JSON (설계서 §6 모양)
//=> 지금 진단 덤프(Document::to_json)에 schema · blocks_version · parser · ocr 를 더한다.
//
// -in: doc  = 추출 결과
// -in: info = 그 문서의 OCR 상태
//
// -out: Value
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn doc_json(doc: &Document, info: &ocr::DocOcr) -> Value {
    let (on, why) = ocr_state();
    json!({
        "schema": SCHEMA, "blocks_version": BLOCKS_VERSION, "parser": format!("struct_docparse {}", env!("CARGO_PKG_VERSION")),
        "format": doc.format, "title": doc.title, "pages": doc.pages, "warnings": doc.warnings, "raw_text": doc.raw_text,
        "ocr": {"enabled": on, "reason": why, "pending": info.pending, "done": info.done, "whole": info.whole},
        "blocks": doc.blocks.iter().map(|b| b.to_json()).collect::<Vec<_>>(),
    })
}

//------------------------------------------------------------------
// 호출마다 옵션 (sdp_parse 의 call_opts)
// -필드: title      = 문서 제목(없으면 파일 이름에서 확장자 뗀 것)
// -필드: keep_pages = PDF 에서 읽을 쪽(0부터). 있으면 그 쪽만, OCR 없이
// -필드: ocr        = OCR 켬/끔 덮어쓰기
// -필드: chunk      = 청킹 설정(Some 이면 결과에 chunks 를 붙인다, 0.3.0~)
// -필드: ocr_force  = 글자층이 있어도 OCR 해 고칠 쪽(0.4.0~ ocr.force_pages · force_when · force_mode)
//------------------------------------------------------------------
#[derive(Debug, Default, Clone)]
pub struct CallOpts {
    pub title: Option<String>,
    pub keep_pages: Option<Vec<usize>>,
    pub ocr: Option<bool>,
    pub chunk: Option<ChunkConf>,
    pub ocr_force: Option<ocr::fix::Force>,
}

//------------------------------------------------------------------
// call_opts JSON 읽기
//
// -in: j = {"title":..., "keep_pages":[...], "ocr":{"enabled":bool}} 또는 null
//
// -out: Ok(CallOpts)
// -out: error = 모양이 틀리면 Err((EINVAL, 사유))
//------------------------------------------------------------------
pub fn call_opts_from_json(j: &Value) -> Result<CallOpts, (i32, String)> {
    let mut c = CallOpts::default();
    if j.is_null() {
        return Ok(c);
    }
    for (k, v) in obj(j, "call_opts")? {
        match k.as_str() {
            "title" => c.title = strv(v),
            "keep_pages" => {
                let a = v.as_array().ok_or_else(|| (EINVAL, "keep_pages 는 배열".to_string()))?;
                c.keep_pages = Some(a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect());
            }
            "ocr" => {
                let mut f = ocr::fix::Force { mode: "fill".to_string(), ..Default::default() };
                let mut forced = false;
                for (k2, v2) in obj(v, "ocr")? {
                    match k2.as_str() {
                        "enabled" => c.ocr = Some(boolv(v2, "ocr.enabled")?),
                        // 0.4.0~ 글자층이 있어도 이 쪽은 OCR(1부터)
                        "force_pages" => {
                            let a = v2.as_array().ok_or_else(|| (EINVAL, "ocr.force_pages 는 쪽 번호(1부터) 배열".to_string()))?;
                            f.pages = a.iter().map(|x| x.as_u64().filter(|&n| n >= 1).map(|n| n as usize)
                                .ok_or_else(|| (EINVAL, "ocr.force_pages 는 쪽 번호(1부터) 배열".to_string()))).collect::<Result<_, _>>()?;
                            forced = true;
                        }
                        // 0.4.0~ 스스로 고를 조건 — 지금은 missing_digits(숫자 빠짐 흔적)만
                        "force_when" => {
                            match v2.as_str() {
                                Some("missing_digits") => f.when = Some("missing_digits".to_string()),
                                _ => return Err((EINVAL, "ocr.force_when 은 missing_digits".into())),
                            }
                            forced = true;
                        }
                        // 0.4.0~ 고치는 방식 — fill(빠진 숫자 틈만, 기본) | replace(쪽 본문을 OCR 글로)
                        "force_mode" => {
                            f.mode = match v2.as_str() {
                                Some(m @ ("fill" | "replace")) => m.to_string(),
                                _ => return Err((EINVAL, "ocr.force_mode 는 fill | replace".into())),
                            };
                        }
                        _ => return Err((EINVAL, format!("모르는 호출 옵션: ocr.{}", k2))),
                    }
                }
                if forced {
                    c.ocr_force = Some(f);
                }
            }
            // 청킹 켜기(0.3.0~) — true · 객체면 결과에 chunks 를 붙인다
            "chunk" => c.chunk = resolve_chunk(v)?,
            _ => return Err((EINVAL, format!("모르는 호출 옵션: {}", k))),
        }
    }
    Ok(c)
}

//------------------------------------------------------------------
// 문서 하나 파싱 (sdp_parse 의 본체)
//=> 1) 파일이 없으면 EIO, OCR 이 꺼진 그림 파일은 EUNSUPPORTED
//   2) 추출(블록 경로 → 실패 시 폴백 평문) — 스캔 쪽은 캐시를 보고 남은 쪽을 적는다
//   3) inline 모드이고 남은 쪽이 있으면 그 자리에서 OCR 해 캐시에 넣고 다시 추출한다
//   4) 결과 JSON
//
// -in: path = 파일 경로
// -in: c    = 호출 옵션
//
// -out: Ok(결과 JSON)
// -out: error = Err((EIO|EUNSUPPORTED|EPARSE|EOCR, 사유))
//------------------------------------------------------------------
pub fn parse(path: &str, c: &CallOpts) -> ApiResult {
    if !std::path::Path::new(path).is_file() {
        return Err((EIO, format!("파일 없음: {}", path)));
    }
    config::with_ocr_override(c.ocr, || {
        if ocr::image::is_image(path) && !ocr::images_enabled() {
            return Err((EUNSUPPORTED, "그림 파일은 OCR 이 켜져 있어야 읽을 수 있습니다".to_string()));
        }
        let title = c.title.as_deref();
        // 고른 쪽만 읽기 — 비슷한 문서 찾기용. OCR 은 하지 않는다
        if let Some(keep) = &c.keep_pages {
            let set: std::collections::HashSet<usize> = keep.iter().copied().collect();
            let f = move |i: usize, _n: usize| set.contains(&i);
            let doc = docmodel::extract_document_keep(path, title, Some(&f)).map_err(|e| (EPARSE, e))?;
            let mut out = doc_json(&doc, &ocr::DocOcr::default());
            if let Some(cc) = &c.chunk {
                attach_chunks(&mut out, &doc, cc)?;
            }
            return Ok(out);
        }
        let mut doc = docmodel::extract_document(path, title).map_err(|e| (EPARSE, e))?;
        let mut info = ocr::take_note(path);
        if cfg().ocr_inline && ocr::enabled() && !info.pending.is_empty() {
            ocr::run::ocr_pages(path, Some(info.pending.clone()), &mut |_, _, _| false).map_err(|e| (EOCR, e))?;
            doc = docmodel::extract_document(path, title).map_err(|e| (EPARSE, e))?;
            info = ocr::take_note(path);
        }
        // 0.4.0~ 글자층이 있는 쪽 강제 OCR · 고치기(PDF 만) — 결과는 out.ocr.forced 에 적는다
        let forced = match &c.ocr_force {
            Some(f) => Some(force_ocr(path, &mut doc, &info, f)?),
            None => None,
        };
        let mut out = doc_json(&doc, &info);
        if let (Some(r), Some(o)) = (forced, out.get_mut("ocr").and_then(|x| x.as_object_mut())) {
            o.insert("forced".into(), r);
        }
        // 청킹을 켠 호출에만 chunk_version · chunking · chunks 를 붙인다 — 끈 호출은 0.2.x 와 글자 하나까지 같다
        if let Some(cc) = &c.chunk {
            attach_chunks(&mut out, &doc, cc)?;
        }
        Ok(out)
    })
}

//------------------------------------------------------------------
// 글자층이 있는 쪽 강제 OCR 해 고치기 (0.4.0~ 호출 옵션 ocr.force_* — 추가검토 설계서 3.8 · 3.9)
//    1) PDF 가 아니면 경고만 남기고 그대로
//    2) 쪽 = force_pages ∪ (force_when 이면 숫자 빠짐 흔적이 있는 쪽) − 이미 스캔 쪽으로 OCR 한 쪽
//    3) OCR 을 쓸 수 없으면 경고만 남기고 그대로(파싱은 성공)
//    4) 쪽 전체 OCR(캐시 full 칸) → fill 또는 replace 로 고친다. deferred 모드여도 이 쪽들은 이 자리에서 OCR 한다
//
// -in: path = 파일 / doc = 추출 결과(고친다) / info = 스캔 쪽 OCR 상태 / f = 옵션
//
// -out: Ok({"pages", "mode", "fixed", "tried"}) 또는 Ok({"skipped": 사유})
// -out: error = OCR 실행이 실패하면 Err((EOCR, 사유))
//------------------------------------------------------------------
fn force_ocr(path: &str, doc: &mut Document, info: &ocr::DocOcr, f: &ocr::fix::Force) -> ApiResult {
    if doc.format != "pdf" {
        return Ok(json!({"skipped": "PDF 가 아님"}));
    }
    let mut pages: std::collections::BTreeSet<usize> = f.pages.iter().copied().collect();
    if f.when.as_deref() == Some("missing_digits") {
        pages.extend(ocr::fix::pages_missing_digits(doc));
    }
    // 문서 쪽 수 밖 · 이미 스캔 쪽으로 OCR 한(또는 기다리는) 쪽은 뺀다
    let np = doc.pages.unwrap_or(0).max(0) as usize;
    let pages: Vec<usize> = pages.into_iter()
        .filter(|p| *p >= 1 && (np == 0 || *p <= np) && !info.done.contains(p) && !info.pending.contains(p)).collect();
    if pages.is_empty() {
        return Ok(json!({"pages": [], "mode": f.mode, "fixed": 0, "tried": 0}));
    }
    let (on, why) = ocr_state();
    if !on {
        let msg = format!("강제 OCR 건너뜀 — OCR 을 쓸 수 없음({})", why.unwrap_or("?"));
        doc.warnings.push(msg.clone());
        return Ok(json!({"skipped": msg, "pages": pages}));
    }
    let rows = ocr::run::full_page_rows(path, &pages).map_err(|e| (EOCR, e))?;
    let r = ocr::fix::apply(doc, &rows, &f.mode);
    Ok(json!({"pages": r.pages, "mode": r.mode, "fixed": r.fixed, "tried": r.tried}))
}

//------------------------------------------------------------------
// 쪽 OCR → 캐시 (sdp_ocr_pages)
//
// -in: path     = PDF 또는 그림 파일
// -in: pages    = 쪽 번호(1부터). None 이면 남은 쪽 전부
// -in: progress = 쪽마다 (k, of, ms) → 멈출지
//
// -out: Ok({"pages": 새로 한 쪽 수, "skipped": 캐시에 있던 쪽 수, "stopped": bool})
// -out: error = OCR 을 쓸 수 없으면 EOCR, 멈추면 결과의 stopped=true(오류 아님), 그 밖 실패 EOCR
//------------------------------------------------------------------
pub fn ocr_pages(path: &str, pages: Option<Vec<usize>>, progress: &mut dyn FnMut(usize, usize, f64) -> bool) -> ApiResult {
    if !std::path::Path::new(path).is_file() {
        return Err((EIO, format!("파일 없음: {}", path)));
    }
    let (on, why) = ocr_state();
    if !on {
        return Err((EOCR, format!("OCRError: OCR 을 쓸 수 없음({})", why.unwrap_or("?"))));
    }
    let r = ocr::run::ocr_pages(path, pages, progress).map_err(|e| (EOCR, e))?;
    Ok(json!({"pages": r.pages, "skipped": r.skipped, "stopped": r.stopped}))
}

//------------------------------------------------------------------
// 파일 OCR 시험 (sdp_ocr_file) — 캐시를 쓰지 않고 쪽마다 줄·상자·시간을 돌려준다
//=> SimpleRAG `ocr <파일>` 명령과 RapidOCR 비교 측정이 쓴다. 폴더를 주면 안의 그림 파일을 이름 순으로.
//
// -in: path  = PDF · 그림 파일 · 그림 폴더
// -in: pages = 쪽 번호(1부터). None 이면 전부
//
//   큰 쪽 · 그림은 대기열 OCR(ocr::run)과 똑같이 A4 상한으로 읽는다 — 좌표 q 와 w · h 는 원래 기준
//   (PDF 200dpi · 그림 원래 픽셀)이고, 실제로 읽은 크기는 ocr_w · ocr_h 에 따로 적는다.
//   쪽 방향 보정(ocr.orient)으로 돌려 읽었으면 rot(시계 방향 각도)이 0 이 아니고, q · w · h 는 바로 선 쪽 기준이다.
// -out: Ok({"file", "engine", "dpi", "pages": {"<번호>": {w, h, ocr_w, ocr_h, rot, render_ms, ocr_ms, name?, rows:[글], lines:[{t,s,q}]}}})
// -out: error = 모델 없음·읽기·추론 실패면 Err((EOCR|EIO, 사유))
//------------------------------------------------------------------
pub fn ocr_file(path: &str, pages: Option<Vec<usize>>) -> ApiResult {
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err((EIO, format!("파일 없음: {}", path)));
    }
    ocr::models_present().map_err(|e| (EOCR, e))?;
    let mut out = Map::new();
    // 그림 한 장 OCR 해 결과 표에 담기
    //   pdf = Some(실제 dpi) 면 PDF 쪽(좌표를 200dpi 로 되돌림), None 이면 그림 파일(상한 줄이기는 ocr_image_file 안에서)
    let mut one = |no: usize, img: &ocr::Bgr, render_ms: f64, name: Option<String>, pdf: Option<f32>| -> Result<(), (i32, String)> {
        let t0 = std::time::Instant::now();
        let (lines, rot, k) = match pdf {
            Some(dpi) => {
                let (lines, rot) = ocr::ocr_image(img).map_err(|e| (EOCR, e))?;
                (ocr::scale_lines(lines, ocr::DPI / dpi), rot, ocr::DPI / dpi)
            }
            None => {
                // 줄인 크기는 cap_scale 로 다시 계산해 보여 주기만 한다
                let s = if cfg().ocr_page_cap && cfg().ocr_image_cap { ocr::cap_scale(img.w as f32, img.h as f32) } else { 1.0 };
                let (lines, rot) = ocr::ocr_image_file(img).map_err(|e| (EOCR, e))?;
                (lines, rot, 1.0 / s)
            }
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let rows = ocr::group_rows(&lines.iter().map(|l| (l.text.clone(), l.bbox())).collect::<Vec<_>>());
        // w · h 는 원래 기준 크기, ocr_w · ocr_h 는 실제로 OCR 한 그림 크기
        let (mut ow, mut oh) = if pdf.is_some() { (img.w, img.h) } else { (((img.w as f32 / k).round() as usize).max(1), ((img.h as f32 / k).round() as usize).max(1)) };
        let (mut w, mut h) = if pdf.is_some() { ((img.w as f32 * k).round() as usize, (img.h as f32 * k).round() as usize) } else { (img.w, img.h) };
        // 쪽을 90 · 270° 돌려 읽었으면 좌표 q 는 바로 선 쪽 기준 — 크기도 그 기준으로 보여 준다
        if ocr::swaps_sides(rot) {
            (w, h, ow, oh) = (h, w, oh, ow);
        }
        let mut e = json!({
            "w": w, "h": h, "ocr_w": ow, "ocr_h": oh, "rot": rot, "render_ms": render_ms, "ocr_ms": ms,
            "rows": rows.iter().map(|r| r.0.clone()).collect::<Vec<_>>(),
            "lines": lines.iter().map(|l| json!({"t": l.text, "s": l.score, "q": l.quad})).collect::<Vec<_>>(),
        });
        if let Some(n) = name {
            e["name"] = json!(n);
        }
        out.insert(no.to_string(), e);
        Ok(())
    };
    if p.is_dir() {
        let mut files: Vec<String> = std::fs::read_dir(p).map_err(|e| (EIO, e.to_string()))?
            .filter_map(|e| e.ok()).map(|e| e.path().to_string_lossy().into_owned())
            .filter(|f| ocr::image::is_image(f)).collect();
        files.sort();
        for (i, f) in files.iter().enumerate() {
            let t0 = std::time::Instant::now();
            let img = ocr::image::load(f).map_err(|e| (EOCR, e))?.remove(0);
            one(i + 1, &img, t0.elapsed().as_secs_f64() * 1000.0, Some(pyutil::basename(f).to_string()), None)?;
        }
    } else if pyutil::ext_lower(path) == ".pdf" {
        let data = std::fs::read(path).map_err(|e| (EIO, format!("읽기 실패: {}", e)))?;
        let n = crate::pdfium::page_count(&data).map_err(|e| (EPARSE, e))?;
        for no in pages.unwrap_or_else(|| (1..=n).collect()).into_iter().filter(|&x| x >= 1 && x <= n) {
            let t0 = std::time::Instant::now();
            let r = crate::pdfium::render_page_capped(&data, no - 1, ocr::DPI, cfg().ocr_page_cap, cfg().ocr_page_up).map_err(|e| (EOCR, e))?;
            one(no, &r.img, t0.elapsed().as_secs_f64() * 1000.0, None, Some(r.dpi))?;
        }
    } else {
        let t0 = std::time::Instant::now();
        let frames = ocr::image::load(path).map_err(|e| (EOCR, e))?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        for (i, img) in frames.iter().enumerate() {
            if pages.as_ref().map_or(true, |w| w.contains(&(i + 1))) {
                one(i + 1, img, ms, None, None)?;
            }
        }
    }
    Ok(json!({"file": path, "engine": ocr::ENGINE, "dpi": ocr::DPI, "pages": out}))
}

//------------------------------------------------------------------
// OCR 캐시 지우기 (sdp_ocr_cache_remove)
//
// -in: hash = 파일 내용 SHA-1
//
// -out: 없음
// -out: error = 예외 없음(없으면 조용히)
//------------------------------------------------------------------
pub fn ocr_cache_remove(hash: &str) {
    if hash.len() >= 2 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
        ocr::remove_cache(hash);
    }
}

//------------------------------------------------------------------
// PDF 속성 (sdp_pdf_info) — SimpleRAG meta/extract.rs 의 pdf_meta 용
//
// -in: path = PDF
//
// -out: Ok({"info": [[키, 값]...], "pages", "toc", "text_chars"})
// -out: error = 암호·손상 PDF 면 Err((EPARSE, "PdfiumError: …")), pdfium 적재 실패면 ERUNTIME
//------------------------------------------------------------------
pub fn pdf_info(path: &str) -> ApiResult {
    crate::pdfium::available().map_err(|e| (ERUNTIME, e))?;
    let p = crate::pdfium::meta_info(path).map_err(|e| (EPARSE, e))?;
    Ok(json!({"info": p.info, "pages": p.pages, "toc": p.toc, "text_chars": p.text_chars}))
}

//------------------------------------------------------------------
// PDF 쪽별 글자층 글 (sdp_pdf_page_texts) — SimpleRAG find.rs 의 쪽 찾기 용
//
// -in: path = PDF
//
// -out: Ok({"pages": [쪽 글...]})
// -out: error = 열기 실패면 Err((EPARSE, 사유)), pdfium 적재 실패면 ERUNTIME
//------------------------------------------------------------------
pub fn pdf_page_texts(path: &str) -> ApiResult {
    crate::pdfium::available().map_err(|e| (ERUNTIME, e))?;
    let (raw, _) = crate::pdfium::read_pages(path, false, false, |_, _| true).map_err(|e| (EPARSE, e))?;
    Ok(json!({"pages": raw.into_iter().map(|p| p.text).collect::<Vec<_>>()}))
}

//------------------------------------------------------------------
// 평문 추출 (sdp_extract_text) — 블록 없이 본문만 (SimpleRAG extract::extract_text 와 같음)
//
// -in: path       = 파일
// -in: fix_tables = 탭으로 나뉜 표 줄을 | 표로 고칠지
//
// -out: Ok({"text": 본문, "table_rows": 고친 표 행 수})
// -out: error = 파일 없음 EIO, 추출 실패 EPARSE
//------------------------------------------------------------------
pub fn extract_text(path: &str, fix_tables: bool) -> ApiResult {
    if !std::path::Path::new(path).is_file() {
        return Err((EIO, format!("파일 없음: {}", path)));
    }
    let (t, n) = crate::extract::extract_text(path, fix_tables).map_err(|e| (EPARSE, e))?;
    Ok(json!({"text": t, "table_rows": n}))
}
