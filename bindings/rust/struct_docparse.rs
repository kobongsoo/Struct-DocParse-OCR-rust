//------------------------------------------------------------------
// struct_docparse 러스트 바인딩 — 실행 중에 라이브러리를 여는 방식(libloading)
//=> 이 파일 하나를 프로젝트에 복사해 `mod struct_docparse;` 로 쓴다.
//   의존성: libloading = "0.8", serde_json = "1"
//   빌드할 때 .dll/.so 가 없어도 되고, 실행할 때 경로를 넘겨 연다.
//
//   let sdp = StructDocParse::open("lib/struct_docparse.dll", r#"{"ocr":{"mode":"inline"}}"#)?;
//   let doc = sdp.parse("보고서.hwpx", None)?;        // serde_json::Value
//------------------------------------------------------------------
#![allow(dead_code)]

use std::ffi::{c_char, c_void, CStr, CString};

use libloading::Library;
use serde_json::Value;

type FnOpen = unsafe extern "C" fn(*const c_char, Option<unsafe extern "C" fn(i32, *const c_char, *mut c_void)>, *mut c_void, *mut *mut c_void) -> i32;
type FnClose = unsafe extern "C" fn(*mut c_void);
type FnVersion = unsafe extern "C" fn() -> *const c_char;
type FnAbi = unsafe extern "C" fn() -> u32;
type FnPathOpt = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char, *mut *mut c_char) -> i32;
type FnPath = unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> i32;
type FnStatus = unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> i32;
type FnSetOpt = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32;
type FnExtract = unsafe extern "C" fn(*mut c_void, *const c_char, i32, *mut *mut c_char) -> i32;
type FnLastErr = unsafe extern "C" fn() -> *const c_char;
type FnFree = unsafe extern "C" fn(*mut c_char);

//------------------------------------------------------------------
// 라이브러리가 돌려준 오류
// -필드: code = 상태 코드(-1 EINVAL … -99 EPANIC) / msg = 오류문
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct SdpError {
    pub code: i32,
    pub msg: String,
}

impl std::fmt::Display for SdpError {
    // "[코드] 오류문" 모양으로
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.msg)
    }
}

impl std::error::Error for SdpError {}

//------------------------------------------------------------------
// 열린 라이브러리 + 핸들
// -필드: _lib = 라이브러리(함수 포인터가 가리키므로 끝까지 쥔다) / h = sdp_open 핸들 / 나머지 = 함수
//------------------------------------------------------------------
pub struct StructDocParse {
    _lib: Library,
    h: *mut c_void,
    close: FnClose,
    version: FnVersion,
    parse: FnPathOpt,
    ocr_file: FnPathOpt,
    pdf_info: FnPath,
    status: FnStatus,
    set_option: FnSetOpt,
    extract_text: FnExtract,
    // 0.3.0~ 다시 청킹 — 옛 라이브러리에는 없어 None
    chunk: Option<FnPathOpt>,
    last_error: FnLastErr,
    free: FnFree,
}

// 핸들은 여러 스레드가 같이 써도 된다(라이브러리가 보장)
unsafe impl Send for StructDocParse {}
unsafe impl Sync for StructDocParse {}

// &str → C 문자열(안의 NUL 은 지운다)
fn cs(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

impl StructDocParse {
    //------------------------------------------------------------------
    // 라이브러리 열기 + sdp_open
    //
    // -in: lib_path     = struct_docparse.dll / libstruct_docparse.so 경로
    // -in: options_json = 열 때 옵션 JSON("" 이면 기본값)
    //
    // -out: Ok(StructDocParse)
    // -out: error = 적재 실패 · ABI 다름 · 열기 실패면 Err(SdpError)
    //------------------------------------------------------------------
    pub fn open(lib_path: &str, options_json: &str) -> Result<Self, SdpError> {
        let err = |code: i32, msg: String| SdpError { code, msg };
        unsafe {
            let lib = Library::new(lib_path).map_err(|e| err(-7, format!("라이브러리 적재 실패: {}", e)))?;
            macro_rules! sym {
                ($t:ty, $n:expr) => {{
                    *lib.get::<$t>($n).map_err(|e| err(-7, format!("함수 없음: {}", e)))?
                }};
            }
            let abi: FnAbi = sym!(FnAbi, b"sdp_abi_version\0");
            if abi() != 1 {
                return Err(err(-1, format!("ABI 가 다릅니다: {}", abi())));
            }
            let open: FnOpen = sym!(FnOpen, b"sdp_open\0");
            let last_error: FnLastErr = sym!(FnLastErr, b"sdp_last_error\0");
            let o = cs(options_json);
            let mut h: *mut c_void = std::ptr::null_mut();
            let rc = open(o.as_ptr(), None, std::ptr::null_mut(), &mut h);
            if rc != 0 {
                return Err(err(rc, CStr::from_ptr(last_error()).to_string_lossy().into_owned()));
            }
            Ok(StructDocParse {
                h,
                close: sym!(FnClose, b"sdp_close\0"),
                version: sym!(FnVersion, b"sdp_version\0"),
                parse: sym!(FnPathOpt, b"sdp_parse\0"),
                ocr_file: sym!(FnPathOpt, b"sdp_ocr_file\0"),
                pdf_info: sym!(FnPath, b"sdp_pdf_info\0"),
                status: sym!(FnStatus, b"sdp_get_status\0"),
                set_option: sym!(FnSetOpt, b"sdp_set_option\0"),
                extract_text: sym!(FnExtract, b"sdp_extract_text\0"),
                chunk: lib.get::<FnPathOpt>(b"sdp_chunk\0").ok().map(|f| *f),
                last_error,
                free: sym!(FnFree, b"sdp_free\0"),
                _lib: lib,
            })
        }
    }

    //------------------------------------------------------------------
    // 결과 받기 — 0 이면 JSON 을 읽고 sdp_free, 아니면 마지막 오류문
    //
    // -in: rc = 상태 코드 / p = 결과 문자열
    //
    // -out: Ok(Value) / Err(SdpError)
    //------------------------------------------------------------------
    fn take(&self, rc: i32, p: *mut c_char) -> Result<Value, SdpError> {
        unsafe {
            if rc != 0 {
                return Err(SdpError { code: rc, msg: CStr::from_ptr((self.last_error)()).to_string_lossy().into_owned() });
            }
            if p.is_null() {
                return Ok(Value::Null);
            }
            let s = CStr::from_ptr(p).to_string_lossy().into_owned();
            (self.free)(p);
            serde_json::from_str(&s).map_err(|e| SdpError { code: -1, msg: e.to_string() })
        }
    }

    // 버전 문자열 — "0.1.0 blocks=5 ocr=…"
    pub fn version(&self) -> String {
        unsafe { CStr::from_ptr((self.version)()).to_string_lossy().into_owned() }
    }

    // 상태 JSON(OCR 켜짐 · 모델 · pdfium)
    pub fn status(&self) -> Result<Value, SdpError> {
        let mut out = std::ptr::null_mut();
        let rc = unsafe { (self.status)(self.h, &mut out) };
        self.take(rc, out)
    }

    //------------------------------------------------------------------
    // 문서 파싱 — 결과 JSON(format · title · pages · warnings · raw_text · ocr · blocks)
    //
    // -in: path      = 파일 경로
    // -in: call_opts = 호출 옵션 JSON(예: r#"{"ocr":{"enabled":false}}"#), None 이면 없음
    //                  0.3.0~ r#"{"chunk":true}"# 면 결과에 chunk_version · chunking · chunks 도
    //
    // -out: Ok(Value) / Err(SdpError)
    //------------------------------------------------------------------
    pub fn parse(&self, path: &str, call_opts: Option<&str>) -> Result<Value, SdpError> {
        let (p, o) = (cs(path), call_opts.map(cs));
        let mut out = std::ptr::null_mut();
        let rc = unsafe { (self.parse)(self.h, p.as_ptr(), o.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), &mut out) };
        self.take(rc, out)
    }

    //------------------------------------------------------------------
    // 저장해 둔 파싱 결과를 다시 청킹 (0.3.0~) — 파싱 · OCR 없이 크기만 바꿔 자른다
    //
    // -in: doc_json = parse 결과 JSON 글
    // -in: opts     = 청킹 옵션 JSON(예: r#"{"tokens":256}"#), None 이면 열 때 기본값
    //
    // -out: Ok({"chunk_version", "chunking", "chunks"}) / Err(SdpError) — 옛 라이브러리면 code -1
    //------------------------------------------------------------------
    pub fn chunk(&self, doc_json: &str, opts: Option<&str>) -> Result<Value, SdpError> {
        let f = self.chunk.ok_or_else(|| SdpError { code: -1, msg: "이 라이브러리에는 sdp_chunk 가 없습니다(0.3.0 부터)".into() })?;
        let (d, o) = (cs(doc_json), opts.map(cs));
        let mut out = std::ptr::null_mut();
        let rc = unsafe { f(self.h, d.as_ptr(), o.as_ref().map_or(std::ptr::null(), |c| c.as_ptr()), &mut out) };
        self.take(rc, out)
    }

    // 평문 추출 — {"text", "table_rows"}
    pub fn extract_text(&self, path: &str, fix_tables: bool) -> Result<Value, SdpError> {
        let p = cs(path);
        let mut out = std::ptr::null_mut();
        let rc = unsafe { (self.extract_text)(self.h, p.as_ptr(), fix_tables as i32, &mut out) };
        self.take(rc, out)
    }

    // 실행 중 옵션 바꾸기 — 예: set_option("ocr.enabled", "false")
    pub fn set_option(&self, key: &str, value_json: &str) -> Result<(), SdpError> {
        let (k, v) = (cs(key), cs(value_json));
        let rc = unsafe { (self.set_option)(self.h, k.as_ptr(), v.as_ptr()) };
        self.take(rc, std::ptr::null_mut()).map(|_| ())
    }

    // PDF 속성 — {"info", "pages", "toc", "text_chars"}
    pub fn pdf_info(&self, path: &str) -> Result<Value, SdpError> {
        let p = cs(path);
        let mut out = std::ptr::null_mut();
        let rc = unsafe { (self.pdf_info)(self.h, p.as_ptr(), &mut out) };
        self.take(rc, out)
    }
}

impl Drop for StructDocParse {
    // 핸들 닫기(OCR 엔진도 내린다)
    fn drop(&mut self) {
        unsafe { (self.close)(self.h) };
    }
}
