//------------------------------------------------------------------
// 바깥에 내놓는 기능 (설계서 §7) — FFI 껍데기(sdp-ffi)와 sdp-cli 가 이것만 부른다
//=> 결과는 모두 serde_json::Value. 실패는 (상태 코드, 오류문).
//------------------------------------------------------------------

use serde_json::{json, Map, Value};

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
    format!("{} blocks={} ocr={}", env!("CARGO_PKG_VERSION"), BLOCKS_VERSION, ocr::ENGINE)
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
            // 지금 DPI 는 고정(200) — 캐시 좌표가 이 값에 묶여 있다
            "dpi" => {}
            _ => return Err((EINVAL, format!("모르는 옵션: ocr.{}", k))),
        }
    }
    Ok(())
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
// -out: {version, abi, blocks_version, ocr:{configured, enabled, images, reason, mode, engine, model_dir, cache_dir, threads, max_pages}, pdfium:{ok, error}}
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
            "layout": c.ocr_layout,
            "image_exts": ocr::image::IMAGE_EXTS,
        },
        "pdfium": {"ok": pdf.is_ok(), "error": pdf.err()},
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
//------------------------------------------------------------------
#[derive(Debug, Default, Clone)]
pub struct CallOpts {
    pub title: Option<String>,
    pub keep_pages: Option<Vec<usize>>,
    pub ocr: Option<bool>,
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
                for (k2, v2) in obj(v, "ocr")? {
                    match k2.as_str() {
                        "enabled" => c.ocr = Some(boolv(v2, "ocr.enabled")?),
                        _ => return Err((EINVAL, format!("모르는 호출 옵션: ocr.{}", k2))),
                    }
                }
            }
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
            return Ok(doc_json(&doc, &ocr::DocOcr::default()));
        }
        let mut doc = docmodel::extract_document(path, title).map_err(|e| (EPARSE, e))?;
        let mut info = ocr::take_note(path);
        if cfg().ocr_inline && ocr::enabled() && !info.pending.is_empty() {
            ocr::run::ocr_pages(path, Some(info.pending.clone()), &mut |_, _, _| false).map_err(|e| (EOCR, e))?;
            doc = docmodel::extract_document(path, title).map_err(|e| (EPARSE, e))?;
            info = ocr::take_note(path);
        }
        Ok(doc_json(&doc, &info))
    })
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
// -out: Ok({"file", "engine", "dpi", "pages": {"<번호>": {w, h, render_ms, ocr_ms, name?, rows:[글], lines:[{t,s,q}]}}})
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
    let mut one = |no: usize, img: &ocr::Bgr, render_ms: f64, name: Option<String>| -> Result<(), (i32, String)> {
        let t0 = std::time::Instant::now();
        let lines = ocr::ocr_image(img).map_err(|e| (EOCR, e))?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let rows = ocr::group_rows(&lines.iter().map(|l| (l.text.clone(), l.bbox())).collect::<Vec<_>>());
        let mut e = json!({
            "w": img.w, "h": img.h, "render_ms": render_ms, "ocr_ms": ms,
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
            one(i + 1, &img, t0.elapsed().as_secs_f64() * 1000.0, Some(pyutil::basename(f).to_string()))?;
        }
    } else if pyutil::ext_lower(path) == ".pdf" {
        let data = std::fs::read(path).map_err(|e| (EIO, format!("읽기 실패: {}", e)))?;
        let n = crate::pdfium::page_count(&data).map_err(|e| (EPARSE, e))?;
        for no in pages.unwrap_or_else(|| (1..=n).collect()).into_iter().filter(|&x| x >= 1 && x <= n) {
            let t0 = std::time::Instant::now();
            let r = crate::pdfium::render_page(&data, no - 1, ocr::DPI).map_err(|e| (EOCR, e))?;
            one(no, &r.img, t0.elapsed().as_secs_f64() * 1000.0, None)?;
        }
    } else {
        let t0 = std::time::Instant::now();
        let frames = ocr::image::load(path).map_err(|e| (EOCR, e))?;
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        for (i, img) in frames.iter().enumerate() {
            if pages.as_ref().map_or(true, |w| w.contains(&(i + 1))) {
                one(i + 1, img, ms, None)?;
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
