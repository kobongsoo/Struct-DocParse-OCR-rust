//------------------------------------------------------------------
// struct_docparse.dll — C ABI 껍데기 (설계서 §7)
//=> sdp-core::api 를 extern "C" 함수로 내놓는다.
//    · 문자열은 모두 UTF-8 + NUL 끝. DLL 이 돌려준 문자열은 sdp_free 로 돌려준다
//    · 반환값은 상태 코드(0 = 성공), 결과 JSON 은 out 포인터로
//    · 모든 함수 몸통을 catch_unwind 로 감싸 패닉이 FFI 경계를 넘지 않게 한다(SDP_EPANIC)
//------------------------------------------------------------------

use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use sdp_core::api::{self, ApiResult};
use serde_json::Value;

// 리눅스 .so 의 버전 정보(ELF 노트) — Windows DLL 은 build.rs 의 VERSIONINFO 리소스가 맡는다
#[cfg(target_os = "linux")]
mod elfnote;

// 로그 콜백: (수준 1 경고 · 2 정보, 글, 사용자 포인터)
pub type SdpLogFn = Option<unsafe extern "C" fn(level: i32, msg: *const c_char, user: *mut c_void)>;
// 진행 콜백: (k(1부터), 전체, 걸린 ms, 사용자 포인터) → 0 이 아니면 멈춤
pub type SdpProgressFn = Option<unsafe extern "C" fn(page_index: i32, page_count: i32, ms: f64, user: *mut c_void) -> i32>;

//------------------------------------------------------------------
// 핸들 — 지금은 옵션을 프로세스에 한 벌 두므로 표식 역할만 한다
// -필드: open = 닫혔는지 확인용 표식
//------------------------------------------------------------------
pub struct SdpHandle {
    open: bool,
}

thread_local! {
    // 이 스레드의 마지막 오류문(sdp_last_error 가 돌려준다)
    static LAST_ERR: RefCell<CString> = RefCell::new(CString::default());
}

// 버전 문자열 (프로세스 동안 그대로)
static VERSION: std::sync::OnceLock<CString> = std::sync::OnceLock::new();

//------------------------------------------------------------------
// 마지막 오류문 남기기
//
// -in: msg = 오류문
//
// -out: 없음
// -out: error = 예외 없음(글 안의 NUL 은 지운다)
//------------------------------------------------------------------
fn set_err(msg: &str) {
    let c = CString::new(msg.replace('\0', "")).unwrap_or_default();
    LAST_ERR.with(|e| *e.borrow_mut() = c);
}

//------------------------------------------------------------------
// C 문자열 → &str
//
// -in: p    = NUL 끝 UTF-8 포인터
// -in: name = 오류문에 쓸 인자 이름
//
// -out: Ok(&str)
// -out: error = 널 포인터·UTF-8 아님이면 Err((EINVAL, 사유))
//------------------------------------------------------------------
unsafe fn cstr<'a>(p: *const c_char, name: &str) -> Result<&'a str, (i32, String)> {
    if p.is_null() {
        return Err((api::EINVAL, format!("{} 가 널입니다", name)));
    }
    CStr::from_ptr(p).to_str().map_err(|_| (api::EINVAL, format!("{} 가 UTF-8 이 아닙니다", name)))
}

//------------------------------------------------------------------
// 생략 가능한 JSON 인자 읽기 — 널·빈 문자열이면 Null
//
// -in: p    = JSON 글 포인터(널 가능)
// -in: name = 오류문에 쓸 인자 이름
//
// -out: Ok(Value)
// -out: error = JSON 이 깨졌으면 Err((EINVAL, 사유))
//------------------------------------------------------------------
unsafe fn json_arg(p: *const c_char, name: &str) -> Result<Value, (i32, String)> {
    if p.is_null() {
        return Ok(Value::Null);
    }
    let s = cstr(p, name)?;
    if s.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(s).map_err(|e| (api::EINVAL, format!("{} JSON 오류: {}", name, e)))
}

//------------------------------------------------------------------
// 결과 JSON 을 out 에 담고 상태 코드 돌려주기 (모든 결과 함수의 공통 꼬리)
//=> 1) 몸통을 catch_unwind 로 돌린다
//   2) 성공이면 JSON 글을 새로 할당해 out 에, 실패면 마지막 오류문을 남긴다
//
// -in: out = 결과 포인터를 받을 곳(널이면 결과는 버림)
// -in: f   = 몸통
//
// -out: 상태 코드
// -out: error = 패닉은 SDP_EPANIC 으로
//------------------------------------------------------------------
fn run_json(out: *mut *mut c_char, f: impl FnOnce() -> ApiResult) -> i32 {
    if !out.is_null() {
        unsafe { *out = std::ptr::null_mut() };
    }
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(v)) => {
            if !out.is_null() {
                let s = serde_json::to_string(&v).unwrap_or_default();
                let c = CString::new(s.replace('\0', "")).unwrap_or_default();
                unsafe { *out = c.into_raw() };
            }
            api::OK
        }
        Ok(Err((code, msg))) => {
            set_err(&msg);
            code
        }
        Err(_) => {
            set_err("DLL 내부 오류(panic)");
            api::EPANIC
        }
    }
}

// 결과 없는 함수의 공통 꼬리 — 상태 코드만
fn run_unit(f: impl FnOnce() -> Result<(), (i32, String)>) -> i32 {
    run_json(std::ptr::null_mut(), || f().map(|_| Value::Null))
}

// ABI 버전 — 호스트가 적재 직후 맞는지 본다
#[no_mangle]
pub extern "C" fn sdp_abi_version() -> u32 {
    api::ABI_VERSION
}

// 버전 문자열 — "0.1.0 blocks=5 ocr=…" (해제하지 않는다)
#[no_mangle]
pub extern "C" fn sdp_version() -> *const c_char {
    VERSION.get_or_init(|| CString::new(api::version()).unwrap_or_default()).as_ptr()
}

//------------------------------------------------------------------
// 열기 — 옵션을 정하고 핸들을 만든다
//=> 1) 옵션 JSON 을 읽어 프로세스 옵션으로 정한다(핸들 여러 개는 마지막 옵션을 함께 쓴다)
//   2) 로그 콜백·수준을 정한다
//
// -in: options_json = 설계서 §7 모양(널이면 기본값)
// -in: log          = 로그 콜백(널 가능)
// -in: user         = 콜백에 그대로 넘길 포인터
// -in: out          = 핸들을 받을 곳
//
// -out: 상태 코드 (성공 시 *out = 핸들)
// -out: error = 옵션 오류 SDP_EINVAL, 패닉 SDP_EPANIC
//------------------------------------------------------------------
#[no_mangle]
pub unsafe extern "C" fn sdp_open(options_json: *const c_char, log: SdpLogFn, user: *mut c_void, out: *mut *mut SdpHandle) -> i32 {
    if !out.is_null() {
        *out = std::ptr::null_mut();
    }
    // 스레드 사이로 넘기려고 포인터를 주소 값으로 바꿔 둔다
    let user_addr = user as usize;
    let out_addr = out as usize;
    run_unit(move || {
        if out_addr == 0 {
            return Err((api::EINVAL, "out 이 널입니다".into()));
        }
        let j = json_arg(options_json, "options_json")?;
        let opts = api::options_from_json(&j)?;
        // 로그 수준 — "warn"(기본) | "info" | "off"
        let level = match j.pointer("/log/level").and_then(|v| v.as_str()) {
            Some("info") => sdp_core::log::INFO,
            Some("off") => 0,
            _ => sdp_core::log::WARN,
        };
        match log {
            Some(f) => sdp_core::log::set_sink(Some(Box::new(move |lv, msg| {
                let c = CString::new(msg.replace('\0', "")).unwrap_or_default();
                // 호스트가 준 포인터를 그대로 돌려준다
                unsafe { f(lv, c.as_ptr(), user_addr as *mut c_void) };
            })), level),
            None => sdp_core::log::set_sink(None, level),
        }
        sdp_core::config::set(opts);
        let h = Box::into_raw(Box::new(SdpHandle { open: true }));
        unsafe { *(out_addr as *mut *mut SdpHandle) = h };
        Ok(())
    })
}

// 닫기 — 핸들을 돌려주고 OCR 엔진도 내린다
#[no_mangle]
pub unsafe extern "C" fn sdp_close(h: *mut SdpHandle) {
    if h.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(Box::from_raw(h));
        sdp_core::ocr::unload();
    }));
}

// 핸들 확인 — 널·닫힌 핸들이면 EINVAL
unsafe fn check(h: *mut SdpHandle) -> Result<(), (i32, String)> {
    if h.is_null() || !(*h).open {
        return Err((api::EINVAL, "핸들이 없거나 닫혔습니다".into()));
    }
    Ok(())
}

// 실행 중 옵션 하나 바꾸기 — 예: ("ocr.enabled", "false")
#[no_mangle]
pub unsafe extern "C" fn sdp_set_option(h: *mut SdpHandle, key: *const c_char, value_json: *const c_char) -> i32 {
    run_unit(|| {
        check(h)?;
        let k = cstr(key, "key")?;
        let v = json_arg(value_json, "value_json")?;
        api::set_option(k, &v)
    })
}

// 상태 JSON — OCR 켜짐·모델 유무·이유, pdfium 적재 여부
#[no_mangle]
pub unsafe extern "C" fn sdp_get_status(h: *mut SdpHandle, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        Ok(api::status())
    })
}

// 다룰 수 있는 파일인가 — 1 블록 경로 / 2 폴백 평문 / 0 미지원 (오류면 음수)
#[no_mangle]
pub unsafe extern "C" fn sdp_supports(h: *mut SdpHandle, path: *const c_char) -> i32 {
    match catch_unwind(AssertUnwindSafe(|| -> Result<i32, (i32, String)> {
        check(h)?;
        Ok(api::supports(cstr(path, "path")?))
    })) {
        Ok(Ok(n)) => n,
        Ok(Err((c, m))) => {
            set_err(&m);
            c
        }
        Err(_) => api::EPANIC,
    }
}

//------------------------------------------------------------------
// 문서 파싱 — 결과 JSON(설계서 §6)
//
// -in: path           = 파일 경로
// -in: call_opts_json = {"title", "keep_pages", "ocr":{"enabled"}} (널 가능)
// -in: out_json       = 결과
//
// -out: 상태 코드
// -out: error = SDP_EIO · SDP_EUNSUPPORTED · SDP_EPARSE · SDP_EOCR · SDP_EPANIC
//------------------------------------------------------------------
#[no_mangle]
pub unsafe extern "C" fn sdp_parse(h: *mut SdpHandle, path: *const c_char, call_opts_json: *const c_char, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        let p = cstr(path, "path")?;
        let c = api::call_opts_from_json(&json_arg(call_opts_json, "call_opts_json")?)?;
        api::parse(p, &c)
    })
}

//------------------------------------------------------------------
// 저장한 파싱 결과를 다시 청킹 (0.3.0~, 블록청킹 설계서 §5.4)
//=> 파싱 · OCR 없이 크기만 바꿔 다시 자른다.
//
// -in: doc_json        = sdp_parse 결과 JSON 그대로
// -in: chunk_opts_json = {"tokens":256, …} — 호출 옵션 chunk 객체와 같은 모양(널이면 열 때 기본값)
// -in: out_json        = {"chunk_version", "chunking", "chunks"}
//
// -out: 상태 코드
// -out: error = SDP_EINVAL(JSON 모양 · 옵션 · 토크나이저) · SDP_EPANIC
//------------------------------------------------------------------
#[no_mangle]
pub unsafe extern "C" fn sdp_chunk(h: *mut SdpHandle, doc_json: *const c_char, chunk_opts_json: *const c_char, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        let d = json_arg(doc_json, "doc_json")?;
        if d.is_null() {
            return Err((api::EINVAL, "doc_json 이 널입니다".into()));
        }
        api::chunk(&d, &json_arg(chunk_opts_json, "chunk_opts_json")?)
    })
}

// 평문 추출 — {"text", "table_rows"}
#[no_mangle]
pub unsafe extern "C" fn sdp_extract_text(h: *mut SdpHandle, path: *const c_char, fix_tables: i32, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        api::extract_text(cstr(path, "path")?, fix_tables != 0)
    })
}

// 쪽 번호 배열 JSON 읽기 — null 이면 None
fn pages_arg(v: &Value) -> Result<Option<Vec<usize>>, (i32, String)> {
    match v {
        Value::Null => Ok(None),
        Value::Array(a) => Ok(Some(a.iter().filter_map(|x| x.as_u64()).map(|x| x as usize).collect())),
        _ => Err((api::EINVAL, "pages_json 은 쪽 번호 배열".into())),
    }
}

//------------------------------------------------------------------
// 쪽 OCR → 캐시 저장 — {"pages", "skipped", "stopped"}
//
// -in: path       = PDF · 그림 파일
// -in: pages_json = [쪽 번호(1부터)] (널이면 남은 쪽 전부)
// -in: cb · user  = 진행 콜백(0 이 아닌 값을 돌려주면 그 쪽까지 하고 선다)
// -in: out_json   = 결과
//
// -out: 상태 코드
// -out: error = OCR 을 못 쓰거나 실패면 SDP_EOCR
//------------------------------------------------------------------
#[no_mangle]
pub unsafe extern "C" fn sdp_ocr_pages(h: *mut SdpHandle, path: *const c_char, pages_json: *const c_char, cb: SdpProgressFn,
                                       user: *mut c_void, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        let p = cstr(path, "path")?;
        let pages = pages_arg(&json_arg(pages_json, "pages_json")?)?;
        // 진행 콜백 — 없으면 멈추지 않는다
        let mut prog = |k: usize, of: usize, ms: f64| -> bool {
            match cb {
                Some(f) => unsafe { f(k as i32, of as i32, ms, user) != 0 },
                None => false,
            }
        };
        api::ocr_pages(p, pages, &mut prog)
    })
}

// 파일 OCR 시험(캐시 안 씀) — 쪽마다 줄·점수·상자·시간
#[no_mangle]
pub unsafe extern "C" fn sdp_ocr_file(h: *mut SdpHandle, path: *const c_char, pages_json: *const c_char, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        let p = cstr(path, "path")?;
        api::ocr_file(p, pages_arg(&json_arg(pages_json, "pages_json")?)?)
    })
}

// OCR 엔진 내리기 — 메모리를 돌려준다
#[no_mangle]
pub unsafe extern "C" fn sdp_ocr_unload(h: *mut SdpHandle) {
    if h.is_null() {
        return;
    }
    let _ = catch_unwind(sdp_core::ocr::unload);
}

// OCR 캐시 지우기 — 파일 내용 SHA-1
#[no_mangle]
pub unsafe extern "C" fn sdp_ocr_cache_remove(h: *mut SdpHandle, content_hash: *const c_char) -> i32 {
    run_unit(|| {
        check(h)?;
        api::ocr_cache_remove(cstr(content_hash, "content_hash")?);
        Ok(())
    })
}

// PDF 속성 — {"info":[[키,값]], "pages", "toc", "text_chars"}
#[no_mangle]
pub unsafe extern "C" fn sdp_pdf_info(h: *mut SdpHandle, path: *const c_char, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        api::pdf_info(cstr(path, "path")?)
    })
}

// PDF 쪽별 글자층 글 — {"pages": [...]}
#[no_mangle]
pub unsafe extern "C" fn sdp_pdf_page_texts(h: *mut SdpHandle, path: *const c_char, out_json: *mut *mut c_char) -> i32 {
    run_json(out_json, || {
        check(h)?;
        api::pdf_page_texts(cstr(path, "path")?)
    })
}

// 이 스레드의 마지막 오류문 (해제하지 않는다 · 다음 호출 전까지 유효)
#[no_mangle]
pub extern "C" fn sdp_last_error() -> *const c_char {
    LAST_ERR.with(|e| e.borrow().as_ptr())
}

// DLL 이 돌려준 문자열 해제
#[no_mangle]
pub unsafe extern "C" fn sdp_free(p: *mut c_char) {
    if !p.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(CString::from_raw(p))));
    }
}
