//------------------------------------------------------------------
// 파서·OCR 옵션 (설계서 §7 "열 때 옵션")
//=> SimpleRAG 의 config::cfg() 자리를 대신한다. 옮겨 온 코드가 `crate::config::cfg().ocr` 처럼
//   그대로 읽을 수 있게 같은 필드 이름을 쓴다.
//    1) 프로세스에 옵션 한 벌(sdp_open 이 정한다) — 핸들 여러 개는 같은 옵션을 함께 쓴다
//    2) 호출마다 덮어쓰기(call_opts.ocr.enabled)는 스레드별로 — 그 호출이 도는 스레드에서만 보인다
//------------------------------------------------------------------

use std::cell::Cell;
use std::sync::{Arc, RwLock};

// 추출 결과가 바뀌는 코드 변경마다 올린다 — 호스트는 이 값으로 재인덱싱을 판정한다(SimpleRAG BLOCKS_VERSION)
// 6 = OCR 구조 분석(ocr.layout) 기본 켬 — 스캔 문서 블록이 바뀐다(2026-11-17)
pub const BLOCKS_VERSION: i64 = 6;

//------------------------------------------------------------------
// 옵션 한 벌
// -필드: ocr                = OCR 켬(기본 true). 모델 파일이 없으면 켜져 있어도 꺼진 것처럼 동작
// -필드: ocr_images         = 그림 파일도 OCR (ocr 이 꺼지면 함께 꺼진 것으로 본다)
// -필드: ocr_inline         = true = 파싱 안에서 바로 OCR / false = 캐시만 보고 남은 쪽을 알려 줌(deferred)
// -필드: ocr_dir            = OCR 모델 폴더
// -필드: ocr_cache_dir      = OCR 캐시 폴더(None 이면 프로세스 메모리에만)
// -필드: ocr_threads        = ONNX 계산 스레드 수
// -필드: ocr_max_pages      = 문서 하나에서 OCR 할 최대 쪽 수
// -필드: ocr_layout         = OCR 줄 구조 분석(다단·문단·제목·목록·머리말) — 끄면 줄 하나가 문단 하나(OCR 구조분석 1단계 설계서)
// -필드: pdf_tables         = PDF 테두리 표 검출
// -필드: pdf_infer_headings = PDF 글자 크기로 제목 추론
// -필드: cso_snf            = 문서 보안(DRM) 풀기 도구 경로(없으면 SIMPLERAG_SNF_EXE 환경변수)
// -필드: pdfium_path        = pdfium.dll 경로(None 이면 DLL·exe 옆)
// -필드: ort_path           = onnxruntime.dll 경로(None 이면 ORT_DYLIB_PATH 또는 exe 옆)
//------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Options {
    pub ocr: bool,
    pub ocr_images: bool,
    pub ocr_inline: bool,
    pub ocr_dir: String,
    pub ocr_cache_dir: Option<String>,
    pub ocr_threads: i64,
    pub ocr_max_pages: i64,
    pub ocr_layout: bool,
    pub pdf_tables: bool,
    pub pdf_infer_headings: bool,
    pub cso_snf: Option<String>,
    pub pdfium_path: Option<String>,
    pub ort_path: Option<String>,
}

impl Default for Options {
    //------------------------------------------------------------------
    // 기본 옵션 — SimpleRAG config.yaml 기본값과 같게(설계서 §7)
    //=> OCR 켬 · 그림 켬 · inline · 스레드 4 · 최대 100쪽 · OCR 구조 분석 켬 · 표 검출 켬 · 제목 추론 끔
    //
    // -in: 없음
    //
    // -out: Options = 기본값
    // -out: error = 예외 없음
    //------------------------------------------------------------------
    fn default() -> Self {
        // OCR 모델 — DLL 옆 models\ocr (SimpleRAG 처럼 DLL 이 exe 옆에 있으면 exe 옆과 같다)
        let models = lib_dir().join("models").join("ocr");
        Options {
            ocr: true,
            ocr_images: true,
            ocr_inline: true,
            ocr_dir: models.to_string_lossy().into_owned(),
            ocr_cache_dir: None,
            ocr_threads: 4,
            ocr_max_pages: 100,
            // 다단 실문서 검증(2026-11-17) 후 기본 켬 — 끄면 이전(블록 버전 5)과 글자 하나까지 같다
            ocr_layout: true,
            pdf_tables: true,
            pdf_infer_headings: false,
            cso_snf: std::env::var("SIMPLERAG_SNF_EXE").ok().filter(|s| !s.is_empty()),
            pdfium_path: None,
            ort_path: None,
        }
    }
}

static OPTS: RwLock<Option<Arc<Options>>> = RwLock::new(None);

// 운영체제별 런타임 라이브러리 파일 이름 (Windows .dll / 리눅스 lib*.so)
pub const PDFIUM_LIB: &str = if cfg!(windows) { "pdfium.dll" } else { "libpdfium.so" };
pub const ORT_LIB: &str = if cfg!(windows) { "onnxruntime.dll" } else { "libonnxruntime.so" };

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut std::ffi::c_void) -> i32;
    fn GetModuleFileNameW(module: *mut std::ffi::c_void, buf: *mut u16, size: u32) -> u32;
}

// dladdr 가 채우는 정보(glibc Dl_info)
#[cfg(unix)]
#[repr(C)]
struct DlInfo {
    dli_fname: *const std::ffi::c_char,
    dli_fbase: *mut std::ffi::c_void,
    dli_sname: *const std::ffi::c_char,
    dli_saddr: *mut std::ffi::c_void,
}

#[cfg(unix)]
#[link(name = "dl")]
extern "C" {
    fn dladdr(addr: *const std::ffi::c_void, info: *mut DlInfo) -> std::ffi::c_int;
}

//------------------------------------------------------------------
// 이 함수가 들어 있는 모듈 파일 경로 — 리눅스(dladdr)
//
// -in: 없음
//
// -out: Some(libstruct_docparse.so 또는 exe 경로) / None
// -out: error = 예외 없음
//------------------------------------------------------------------
#[cfg(unix)]
fn module_path() -> Option<std::path::PathBuf> {
    unsafe {
        let mut info: DlInfo = std::mem::zeroed();
        if dladdr(module_path as *const std::ffi::c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let s = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
        // 상대 경로로 적재했으면 지금 폴더 기준으로 바꾼다
        std::path::absolute(s).ok()
    }
}

//------------------------------------------------------------------
// 이 함수가 들어 있는 모듈 파일 경로 — Windows(GetModuleHandleExW)
//
// -in: 없음
//
// -out: Some(struct_docparse.dll 또는 exe 경로) / None
// -out: error = 예외 없음
//------------------------------------------------------------------
#[cfg(windows)]
fn module_path() -> Option<std::path::PathBuf> {
    unsafe {
        // FROM_ADDRESS(4) | UNCHANGED_REFCOUNT(2) — 참조 수는 늘리지 않는다
        let mut m: *mut std::ffi::c_void = std::ptr::null_mut();
        let addr = module_path as *const () as *const u16;
        if GetModuleHandleExW(6, addr, &mut m) == 0 || m.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 1024];
        let n = GetModuleFileNameW(m, buf.as_mut_ptr(), buf.len() as u32) as usize;
        if n == 0 || n >= buf.len() {
            return None;
        }
        Some(std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n])))
    }
}

//------------------------------------------------------------------
// 이 코드가 들어 있는 모듈(struct_docparse.dll, 또는 정적으로 묶인 exe)의 폴더
//=> 기본 런타임(pdfium.dll · onnxruntime.dll)과 OCR 모델은 "DLL 옆"에서 찾는다(설계서 §7).
//   exe 폴더를 쓰면 다른 프로그램(예: python.exe)에서 불렀을 때 엉뚱한 곳 — 시스템 폴더의 다른 버전
//   onnxruntime.dll — 을 집는다(2026-11-16 파이썬 래퍼 시험에서 1.17 을 집어 OCR 실패).
//    1) 이 코드가 든 모듈 파일 경로를 얻는다(Windows GetModuleHandleExW · 리눅스 dladdr)
//    2) 그 경로에서 폴더를 뗀다
//
// -in: 없음
//
// -out: 폴더 경로 (실패하면 exe 폴더)
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn lib_dir() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        module_path().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_else(crate::pyutil::exe_dir)
    }).clone()
}

thread_local! {
    // 이 스레드에서만 보이는 OCR 켬/끔 덮어쓰기(호출마다 옵션)
    static OCR_OVERRIDE: Cell<Option<bool>> = const { Cell::new(None) };
}

//------------------------------------------------------------------
// 지금 옵션 읽기 (옮겨 온 코드의 cfg() 자리)
//=> 1) 정한 옵션이 없으면 기본값
//   2) 이 스레드에 OCR 덮어쓰기가 있으면 그 값으로 바꾼 사본을 돌려준다
//
// -in: 없음
//
// -out: Arc<Options> = 지금 옵션
// -out: error = 예외 없음(잠금이 깨져도 안의 값을 쓴다)
//------------------------------------------------------------------
pub fn cfg() -> Arc<Options> {
    let base = {
        let g = OPTS.read().unwrap_or_else(|p| p.into_inner());
        g.clone()
    };
    let base = base.unwrap_or_else(|| Arc::new(Options::default()));
    match OCR_OVERRIDE.with(|c| c.get()) {
        Some(on) if on != base.ocr => {
            let mut o = (*base).clone();
            o.ocr = on;
            Arc::new(o)
        }
        _ => base,
    }
}

//------------------------------------------------------------------
// 옵션 바꾸기 — sdp_open · sdp_set_option 이 부른다
//
// -in: o = 새 옵션
//
// -out: 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn set(o: Options) {
    *OPTS.write().unwrap_or_else(|p| p.into_inner()) = Some(Arc::new(o));
}

//------------------------------------------------------------------
// 이 스레드에서만 OCR 켬/끔을 덮어쓰고 f 를 돈다 (호출마다 옵션)
//=> 끝나면(패닉이어도) 원래 값으로 되돌린다.
//
// -in: on = Some(켬/끔) / None 이면 덮어쓰지 않음
// -in: f  = 그동안 할 일
//
// -out: f 의 결과
// -out: error = f 의 패닉은 그대로 전파(되돌리기는 한다)
//------------------------------------------------------------------
pub fn with_ocr_override<T>(on: Option<bool>, f: impl FnOnce() -> T) -> T {
    // 패닉이 나도 덮어쓰기를 되돌리는 지킴이
    struct Reset(Option<bool>);
    impl Drop for Reset {
        fn drop(&mut self) {
            let v = self.0;
            OCR_OVERRIDE.with(|c| c.set(v));
        }
    }
    let prev = OCR_OVERRIDE.with(|c| c.replace(on.or(c.get())));
    let _r = Reset(prev);
    f()
}
