//------------------------------------------------------------------
// ONNX 세션 만들기 (SimpleRAG embed::make_session_lean 과 같은 설정)
//=> onnxruntime.dll 을 실행 시점에 적재한다. 호스트(SimpleRAG)도 같은 DLL 을 쓰므로
//   ORT_DYLIB_PATH 가 이미 정해져 있으면 그대로 둔다.
//------------------------------------------------------------------

use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;

//------------------------------------------------------------------
// onnxruntime.dll 위치 정하기
//=> 1) 옵션 ort_path → 2) 이미 정한 ORT_DYLIB_PATH → 3) DLL 옆 → 4) exe 옆 → 5) exe 옆 _internal (파이썬 배포본 모양)
//
// -in: 없음
//
// -out: 없음 (ORT_DYLIB_PATH 환경변수를 정한다)
// -out: error = 예외 없음(못 찾으면 그대로 두고, 세션 만들 때 오류가 난다)
//------------------------------------------------------------------
pub fn ensure_dylib() {
    if let Some(p) = crate::config::cfg().ort_path.clone() {
        // 호스트가 이미 다른 경로로 적재했으면 바꾸지 않는다 — 한 프로세스에 onnxruntime 은 하나
        if std::env::var("ORT_DYLIB_PATH").map(|v| v.is_empty()).unwrap_or(true) {
            std::env::set_var("ORT_DYLIB_PATH", p);
        }
        return;
    }
    if std::env::var("ORT_DYLIB_PATH").map(|v| !v.is_empty()).unwrap_or(false) {
        return;
    }
    // DLL 옆 → exe 옆 → exe 옆 _internal (파이썬 배포본 모양)
    let (l, d) = (crate::config::lib_dir(), crate::pyutil::exe_dir());
    let n = crate::config::ORT_LIB;
    for p in [l.join(n), d.join(n), d.join("_internal").join("onnxruntime").join("capi").join(n)] {
        if p.is_file() {
            std::env::set_var("ORT_DYLIB_PATH", p);
            return;
        }
    }
}

//------------------------------------------------------------------
// 아레나를 끈 세션 — 쪽 크기가 매번 달라 메모리가 늘기만 하던 것을 막는다
//
// -in: path    = .onnx 모델 파일
// -in: threads = 계산 스레드 수
//
// -out: Ok(Session)
// -out: error = 모델·DLL 적재 실패면 Err(오류문). ort 의 패닉도 오류문으로 바꾼다
//------------------------------------------------------------------
pub fn make_session_lean(path: &str, threads: usize) -> Result<Session, String> {
    ensure_dylib();
    let path = path.to_string();
    std::panic::catch_unwind(move || -> Result<Session, String> {
        use ort::execution_providers::CPUExecutionProvider;
        Session::builder().map_err(|e| e.to_string())?
            .with_optimization_level(GraphOptimizationLevel::Level3).map_err(|e| e.to_string())?
            .with_intra_threads(threads).map_err(|e| e.to_string())?
            .with_inter_threads(1).map_err(|e| e.to_string())?
            .with_memory_pattern(true).map_err(|e| e.to_string())?
            .with_execution_providers([CPUExecutionProvider::default().with_arena_allocator(false).build()]).map_err(|e| e.to_string())?
            .commit_from_file(&path).map_err(|e| e.to_string())
    })
    .unwrap_or_else(|_| Err(format!("{} 을 적재할 수 없습니다", crate::config::ORT_LIB)))
}
