//------------------------------------------------------------------
// 로그 — 호스트가 넘긴 콜백으로 보낸다 (SimpleRAG cli::log 자리)
//=> 콜백이 없으면 버린다. 수준: 1 = 경고, 2 = 정보.
//------------------------------------------------------------------

use std::sync::Mutex;

pub const WARN: i32 = 1;
pub const INFO: i32 = 2;

type Sink = Box<dyn Fn(i32, &str) + Send + Sync>;

static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static LEVEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(WARN);

//------------------------------------------------------------------
// 로그 받을 곳 정하기
//
// -in: sink  = 받을 함수(None 이면 버림)
// -in: level = 이 수준 이하만 보낸다(1 경고 · 2 정보)
//
// -out: 없음
// -out: error = 예외 없음
//------------------------------------------------------------------
pub fn set_sink(sink: Option<Sink>, level: i32) {
    *SINK.lock().unwrap_or_else(|p| p.into_inner()) = sink;
    LEVEL.store(level, std::sync::atomic::Ordering::Relaxed);
}

//------------------------------------------------------------------
// 로그 한 줄 보내기
//
// -in: level = 1 경고 / 2 정보
// -in: msg   = 글
//
// -out: 없음
// -out: error = 예외 없음(받는 쪽 패닉도 삼킨다)
//------------------------------------------------------------------
pub fn emit(level: i32, msg: &str) {
    if level > LEVEL.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let g = SINK.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(f) = g.as_ref() {
        // 호스트 콜백의 패닉이 파서까지 번지지 않게
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(level, msg)));
    }
}

// 경고 한 줄 (옮겨 온 코드의 cli::log 자리)
pub fn log(msg: &str) {
    emit(WARN, msg);
}
