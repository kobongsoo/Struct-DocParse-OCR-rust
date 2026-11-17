//------------------------------------------------------------------
// sdp.exe — 구조화 문서 파서 시험·덤프·같음 비교 도구 (설계서 §11)
//=> DLL 과 같은 C ABI 함수(sdp-ffi)를 그대로 거쳐 부른다 — FFI 층까지 함께 시험하기 위해서.
//   명령:
//     parse   <파일> [--pages 1,3-5] [옵션] 결과 JSON 을 화면(또는 --out)에 — --pages 를 주면 그 쪽만 OCR 하고 그 쪽 블록만 남긴다
//     dump    <목록.txt> <폴더> [옵션]      목록의 문서마다 <폴더>/<번호>/<이름>.blocks.json
//     compare <기준 폴더> <새 폴더>         블록 JSON 이 글자 하나까지 같은지
//     ocr     <목록.txt|파일> [옵션]        남은 쪽을 OCR 해 캐시에 채운다
//     ocrfile <파일|폴더> [--pages 1,3-5] [--out <json>] [--all-rows]   캐시 없이 OCR 시험(화면에는 쪽마다 앞 5줄, --all-rows 면 전부)
//   옵션: --mode inline|deferred  --no-ocr  --layout | --no-layout  --cache <폴더>  --ocr-dir <폴더>  --out <파일>  --threads N  --max-pages N (문서 하나에서 OCR 할 최대 쪽 수 — 기본 100)
//         --no-arena (OCR 아레나 끔)  --page-cap a4|none (큰 쪽 · 그림 상한)  --image-cap (그림 파일에도 상한 — 기본 끔)
//         --no-page-up (작은 쪽 확대 끔)  --no-orient (쪽 방향 보정 끔)  --orient-flip (뒤집힘(180°) 확인 켬 — 기본 끔)
//------------------------------------------------------------------

use std::ffi::{c_char, CStr, CString};
use std::path::Path;

use serde_json::{json, Value};
use struct_docparse as ffi;

//------------------------------------------------------------------
// 명령줄 옵션
// -필드: args    = 옵션을 뺀 위치 인자
// -필드: mode    = inline | deferred
// -필드: no_ocr  = OCR 끔
// -필드: cache   = OCR 캐시 폴더
// -필드: ocr_dir = OCR 모델 폴더
// -필드: out     = 결과 파일
// -필드: pages   = 쪽 고르기
// -필드: threads = OCR 스레드
// -필드: max_pages = 문서 하나에서 OCR 할 최대 쪽 수(None 이면 라이브러리 기본 100)
// -필드: all_rows  = ocrfile 화면 출력에 쪽마다 모든 줄(기본은 앞 5줄)
// -필드: layout  = OCR 구조 분석: Some(true) --layout / Some(false) --no-layout / None 라이브러리 기본값(켬)
// -필드: no_arena     = OCR 아레나 끔(0.1.0 과 같은 메모리 방식)
// -필드: page_cap     = 큰 쪽 · 그림 상한 "a4" | "none" (None 이면 라이브러리 기본값 a4)
// -필드: image_cap    = 그림 파일에도 상한 켬(라이브러리 기본은 끔)
// -필드: no_page_up      = 작은 쪽 확대 끔(이전과 같은 해상도 — 비교용)
// -필드: no_orient       = 쪽 방향 보정 끔(이전과 같은 결과 — 비교용)
// -필드: orient_flip     = 뒤집힘(180°) 확인 켬(라이브러리 기본은 끔)
//------------------------------------------------------------------
#[derive(Default, Clone)]
struct Cli {
    args: Vec<String>,
    mode: Option<String>,
    no_ocr: bool,
    cache: Option<String>,
    ocr_dir: Option<String>,
    out: Option<String>,
    pages: Option<String>,
    threads: Option<i64>,
    max_pages: Option<i64>,
    all_rows: bool,
    layout: Option<bool>,
    no_arena: bool,
    page_cap: Option<String>,
    image_cap: bool,
    no_page_up: bool,
    no_orient: bool,
    orient_flip: bool,
}

//------------------------------------------------------------------
// 명령줄 읽기
//
// -in: v = 프로그램 이름을 뺀 인자들
//
// -out: Cli
// -out: error = 값이 빠진 옵션이면 종료 코드 2 로 끝낸다
//------------------------------------------------------------------
fn parse_cli(v: Vec<String>) -> Cli {
    let mut c = Cli::default();
    let mut it = v.into_iter();
    while let Some(a) = it.next() {
        // 값을 받는 옵션은 다음 인자를 꺼낸다
        let mut val = |name: &str| it.next().unwrap_or_else(|| die(&format!("{} 값이 없습니다", name)));
        match a.as_str() {
            "--mode" => c.mode = Some(val("--mode")),
            "--no-ocr" => c.no_ocr = true,
            "--layout" => c.layout = Some(true),
            "--no-layout" => c.layout = Some(false),
            "--cache" => c.cache = Some(val("--cache")),
            "--ocr-dir" => c.ocr_dir = Some(val("--ocr-dir")),
            "--out" => c.out = Some(val("--out")),
            "--pages" => c.pages = Some(val("--pages")),
            "--threads" => c.threads = val("--threads").parse().ok(),
            "--max-pages" => c.max_pages = val("--max-pages").parse().ok(),
            "--all-rows" => c.all_rows = true,
            "--no-arena" => c.no_arena = true,
            "--page-cap" => c.page_cap = Some(val("--page-cap")),
            "--image-cap" => c.image_cap = true,
            "--no-page-up" => c.no_page_up = true,
            "--no-orient" => c.no_orient = true,
            "--orient-flip" => c.orient_flip = true,
            _ => c.args.push(a),
        }
    }
    c
}

// 오류문을 찍고 끝낸다
fn die(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(2)
}

// DLL 로그 콜백 — 표준 오류로
unsafe extern "C" fn on_log(_level: i32, msg: *const c_char, _user: *mut std::ffi::c_void) {
    eprintln!("[sdp] {}", CStr::from_ptr(msg).to_string_lossy());
}

//------------------------------------------------------------------
// 핸들 열기 — 명령줄 옵션을 sdp_open 옵션 JSON 으로
//
// -in: c = 명령줄 옵션
//
// -out: 핸들 포인터
// -out: error = 열기 실패면 종료 코드 2
//------------------------------------------------------------------
fn open(c: &Cli) -> *mut ffi::SdpHandle {
    let mut ocr = json!({"enabled": !c.no_ocr, "mode": c.mode.clone().unwrap_or_else(|| "inline".into())});
    if let Some(d) = &c.cache {
        ocr["cache_dir"] = json!(d);
    }
    if let Some(d) = c.ocr_dir.clone().or_else(|| std::env::var("SDP_OCR_DIR").ok()) {
        ocr["model_dir"] = json!(d);
    }
    if let Some(t) = c.threads {
        ocr["threads"] = json!(t);
    }
    // 100쪽 넘는 문서를 끝까지 OCR 해 볼 때
    if let Some(n) = c.max_pages {
        ocr["max_pages"] = json!(n);
    }
    // 줄 때만 넣는다 — 안 주면 라이브러리 기본값(켬). --no-layout 은 이전(블록 버전 5) 출력과 비교할 때 쓴다
    if let Some(on) = c.layout {
        ocr["layout"] = json!(on);
    }
    // OCR 속도개선 옵션 — 줄 때만 넣는다(0.1.0 과 비교할 때 --no-arena --page-cap none)
    if c.no_arena {
        ocr["arena"] = json!(false);
    }
    if let Some(p) = &c.page_cap {
        ocr["page_cap"] = json!(p);
    }
    if c.image_cap {
        ocr["image_cap"] = json!(true);
    }
    // 작은 쪽 확대 — 줄 때만 끈다(확대 전 결과와 비교할 때)
    if c.no_page_up {
        ocr["page_up"] = json!(false);
    }
    // 쪽 방향 보정 — 줄 때만 끈다(보정 전 결과와 비교할 때)
    if c.no_orient {
        ocr["orient"] = json!(false);
    }
    if c.orient_flip {
        ocr["orient_flip"] = json!(true);
    }
    let opts = CString::new(json!({"ocr": ocr}).to_string()).unwrap();
    let mut h: *mut ffi::SdpHandle = std::ptr::null_mut();
    let rc = unsafe { ffi::sdp_open(opts.as_ptr(), Some(on_log), std::ptr::null_mut(), &mut h) };
    if rc != 0 {
        die(&format!("sdp_open 실패({}): {}", rc, last_err()));
    }
    h
}

// 이 스레드의 마지막 오류문
fn last_err() -> String {
    unsafe { CStr::from_ptr(ffi::sdp_last_error()).to_string_lossy().into_owned() }
}

//------------------------------------------------------------------
// DLL 결과 문자열 → JSON (받은 문자열은 sdp_free 로 돌려준다)
//
// -in: rc = 상태 코드 / p = 결과 포인터
//
// -out: Ok(Value) / Err((코드, 오류문))
// -out: error = 실패 코드면 Err
//------------------------------------------------------------------
fn take(rc: i32, p: *mut c_char) -> Result<Value, (i32, String)> {
    if rc != 0 {
        return Err((rc, last_err()));
    }
    let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
    unsafe { ffi::sdp_free(p) };
    serde_json::from_str(&s).map_err(|e| (-1, e.to_string()))
}

// 문서 하나 파싱
fn parse_one(h: *mut ffi::SdpHandle, path: &str) -> Result<Value, (i32, String)> {
    let p = CString::new(path).unwrap();
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { ffi::sdp_parse(h, p.as_ptr(), std::ptr::null(), &mut out) };
    take(rc, out)
}

// "1,3-5" → 쪽 번호 배열 JSON
fn pages_json(s: Option<&str>) -> Option<CString> {
    let s = s?;
    let mut v = vec![];
    for part in s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    v.extend(a.max(1)..=b);
                }
            }
            None => v.extend(part.parse::<usize>().ok()),
        }
    }
    Some(CString::new(json!(v).to_string()).unwrap())
}

// 목록 파일 읽기(빈 줄 빼고)
fn read_list(p: &str) -> Vec<String> {
    std::fs::read_to_string(p).unwrap_or_else(|e| die(&format!("목록 읽기 실패: {}", e)))
        .lines().map(|l| l.trim().trim_start_matches('\u{feff}').to_string()).filter(|l| !l.is_empty()).collect()
}

// 파일 이름 부분
fn basename(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

//------------------------------------------------------------------
// dump — 목록의 문서마다 블록 JSON 을 남긴다 (SimpleRAG extract --dump 와 같은 자리·이름)
//
// -in: c.args = [목록, 결과 폴더]
//
// -out: 종료 코드 0
// -out: error = 문서별 실패는 index.json 의 rc 로 남기고 계속
//------------------------------------------------------------------
fn cmd_dump(c: &Cli) -> i32 {
    let (list, out) = match c.args.as_slice() {
        [l, o] => (l.clone(), o.clone()),
        _ => die("사용법: sdp dump <목록.txt> <폴더>"),
    };
    let h = open(c);
    let paths = read_list(&list);
    let mut index = serde_json::Map::new();
    let t_all = std::time::Instant::now();
    for (i, p) in paths.iter().enumerate() {
        let key = format!("{:04}", i);
        let d = Path::new(&out).join(&key);
        std::fs::create_dir_all(&d).ok();
        let t0 = std::time::Instant::now();
        let r = parse_one(h, p);
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        match r {
            Ok(v) => {
                let f = d.join(format!("{}.blocks.json", basename(p)));
                std::fs::write(&f, serde_json::to_string_pretty(&v).unwrap()).ok();
                index.insert(key, json!({"path": p, "rc": 0, "ms": ms}));
            }
            Err((rc, e)) => {
                index.insert(key, json!({"path": p, "rc": rc, "err": e, "ms": ms}));
            }
        }
        eprintln!("[{}/{}] {:.0}ms {}", i + 1, paths.len(), ms, basename(p));
    }
    std::fs::write(Path::new(&out).join("index.json"), serde_json::to_string_pretty(&Value::Object(index)).unwrap()).ok();
    eprintln!("끝 — {}건 {:.1}초", paths.len(), t_all.elapsed().as_secs_f64());
    unsafe { ffi::sdp_close(h) };
    0
}

// 두 JSON 의 첫 차이 위치 찾기(설명용)
fn first_diff(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                match y.get(k) {
                    Some(vb) => {
                        if let Some(d) = first_diff(va, vb, &format!("{}.{}", path, k)) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{}.{} 없음", path, k)),
                }
            }
            y.keys().find(|k| !x.contains_key(*k)).map(|k| format!("{}.{} 더 있음", path, k))
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{} 길이 {} ≠ {}", path, x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (va, vb))| first_diff(va, vb, &format!("{}[{}]", path, i)))
        }
        _ if a == b => None,
        _ => Some(format!("{}: {} ≠ {}", path, short(a), short(b))),
    }
}

// 긴 값 줄여 보이기
fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 80 { format!("{}…", s.chars().take(80).collect::<String>()) } else { s }
}

//------------------------------------------------------------------
// compare — 기준(분리 전 SimpleRAG 덤프)과 새 덤프 비교
//=> 기준 덤프의 키(format · title · pages · warnings · raw_text · blocks)만 본다.
//   새 덤프에 더해진 schema · blocks_version · parser · ocr 는 비교하지 않는다.
//
// -in: c.args = [기준 폴더, 새 폴더]
//
// -out: 종료 코드 0 = 모두 같음, 1 = 차이 있음
// -out: error = 폴더가 없으면 2
//------------------------------------------------------------------
fn cmd_compare(c: &Cli) -> i32 {
    let (a, b) = match c.args.as_slice() {
        [a, b] => (a.clone(), b.clone()),
        _ => die("사용법: sdp compare <기준 폴더> <새 폴더>"),
    };
    let mut keys: Vec<String> = std::fs::read_dir(&a).unwrap_or_else(|e| die(&e.to_string()))
        .filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    keys.sort();
    let (mut same, mut diff, mut missing) = (0, 0, 0);
    for k in &keys {
        let files: Vec<_> = std::fs::read_dir(Path::new(&a).join(k)).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        let Some(fa) = files.iter().find(|p| p.to_string_lossy().ends_with(".blocks.json")) else { continue };
        let fb = Path::new(&b).join(k).join(fa.file_name().unwrap());
        let va: Value = serde_json::from_str(&std::fs::read_to_string(fa).unwrap_or_default()).unwrap_or(Value::Null);
        let Ok(sb) = std::fs::read_to_string(&fb) else {
            println!("없음  {} {}", k, fa.file_name().unwrap().to_string_lossy());
            missing += 1;
            continue;
        };
        let vb: Value = serde_json::from_str(&sb).unwrap_or(Value::Null);
        // 기준에 있는 키만 골라 비교
        let pick = |v: &Value| -> Value {
            let mut m = serde_json::Map::new();
            for key in va.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default() {
                m.insert(key.clone(), v.get(&key).cloned().unwrap_or(Value::Null));
            }
            Value::Object(m)
        };
        let (pa, pb) = (pick(&va), pick(&vb));
        if pa == pb {
            same += 1;
        } else {
            diff += 1;
            println!("다름  {} {} — {}", k, fa.file_name().unwrap().to_string_lossy(), first_diff(&pa, &pb, "").unwrap_or_default());
        }
    }
    println!("같음 {} · 다름 {} · 새 쪽에 없음 {} (기준 {}건)", same, diff, missing, keys.len());
    if diff == 0 && missing == 0 { 0 } else { 1 }
}

// OCR 진행 콜백 — 쪽마다 한 줄
unsafe extern "C" fn on_progress(k: i32, of: i32, ms: f64, _user: *mut std::ffi::c_void) -> i32 {
    eprintln!("    OCR {}/{} {:.0}ms", k, of, ms);
    0
}

//------------------------------------------------------------------
// ocr — 문서(또는 목록)의 남은 쪽을 OCR 해 캐시에 채운다
//=> 1) deferred 로 파싱해 남은 쪽(ocr.pending)을 알아낸다
//   2) 남은 쪽이 있으면 sdp_ocr_pages 로 캐시에 넣는다
//
// -in: c.args = [목록.txt 또는 파일]
//
// -out: 종료 코드 0 / 실패가 있으면 1
// -out: error = 문서별 실패는 찍고 계속
//------------------------------------------------------------------
fn cmd_ocr(c: &Cli) -> i32 {
    let Some(target) = c.args.first().cloned() else { die("사용법: sdp ocr <목록.txt|파일> --cache <폴더>") };
    // OCR 결과에 영향을 주는 옵션(아레나 · 상한 · 확대 · 방향 보정)은 그대로 넘긴다 — 비교 실행이 조용히 기본값으로 돌지 않게
    let c2 = Cli {
        mode: Some("deferred".into()), cache: c.cache.clone(), ocr_dir: c.ocr_dir.clone(), threads: c.threads, max_pages: c.max_pages,
        no_arena: c.no_arena, page_cap: c.page_cap.clone(), image_cap: c.image_cap,
        no_page_up: c.no_page_up, no_orient: c.no_orient, orient_flip: c.orient_flip,
        ..Default::default()
    };
    let h = open(&c2);
    let paths = if target.to_lowercase().ends_with(".txt") { read_list(&target) } else { vec![target] };
    let mut failed = 0;
    for p in &paths {
        let pending = match parse_one(h, p) {
            Ok(v) => v.pointer("/ocr/pending").cloned().unwrap_or(json!([])),
            Err(_) => continue,
        };
        if pending.as_array().map_or(true, |a| a.is_empty()) {
            continue;
        }
        eprintln!("{} — 남은 쪽 {}", basename(p), pending);
        let cp = CString::new(p.as_str()).unwrap();
        let pj = CString::new(pending.to_string()).unwrap();
        let mut out: *mut c_char = std::ptr::null_mut();
        let rc = unsafe { ffi::sdp_ocr_pages(h, cp.as_ptr(), pj.as_ptr(), Some(on_progress), std::ptr::null_mut(), &mut out) };
        match take(rc, out) {
            Ok(v) => eprintln!("  → {}", v),
            Err((rc, e)) => {
                failed += 1;
                eprintln!("  실패({}): {}", rc, e);
            }
        }
    }
    unsafe { ffi::sdp_close(h) };
    if failed == 0 { 0 } else { 1 }
}

//------------------------------------------------------------------
// ocrfile — 캐시 없이 파일(또는 그림 폴더)을 OCR 해 쪽마다 줄을 보여 준다
//
// -in: c.args = [파일|폴더], c.pages, c.out
//
// -out: 종료 코드 0 / 1
// -out: error = OCR 실패면 1
//------------------------------------------------------------------
fn cmd_ocrfile(c: &Cli) -> i32 {
    let Some(target) = c.args.first().cloned() else { die("사용법: sdp ocrfile <파일|폴더> [--pages ..] [--out ..]") };
    let h = open(c);
    let cp = CString::new(target).unwrap();
    let pj = pages_json(c.pages.as_deref());
    let mut out: *mut c_char = std::ptr::null_mut();
    let rc = unsafe { ffi::sdp_ocr_file(h, cp.as_ptr(), pj.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()), &mut out) };
    let r = take(rc, out);
    unsafe { ffi::sdp_close(h) };
    match r {
        Ok(v) => {
            if let Some(pages) = v.get("pages").and_then(|p| p.as_object()) {
                for (no, pg) in pages {
                    println!("── {}쪽 · OCR {:.0}ms · 상자 {}", no, pg["ocr_ms"].as_f64().unwrap_or(0.0),
                             pg["lines"].as_array().map_or(0, |a| a.len()));
                    // 기본은 쪽마다 앞 5줄만 — 전체는 --all-rows 또는 --out JSON
                    let n = if c.all_rows { usize::MAX } else { 5 };
                    for r in pg["rows"].as_array().into_iter().flatten().take(n) {
                        println!("  {}", r.as_str().unwrap_or(""));
                    }
                }
            }
            if let Some(o) = &c.out {
                std::fs::write(o, serde_json::to_string_pretty(&v).unwrap()).ok();
            }
            0
        }
        Err((rc, e)) => {
            eprintln!("OCR 실패({}): {}", rc, e);
            1
        }
    }
}

//------------------------------------------------------------------
// parse — 파일 하나를 파싱해 결과 JSON 을 보여 준다
//
// -in: c.args = [파일]
//
// -out: 종료 코드 0 / 실패 코드의 절댓값
// -out: error = 파싱 실패면 오류문을 찍는다
//------------------------------------------------------------------
fn cmd_parse(c: &Cli) -> i32 {
    let Some(path) = c.args.first().cloned() else { die("사용법: sdp parse <파일> [--pages 1,3-5] [옵션]") };
    let r = match c.pages.as_deref() {
        Some(sel) => parse_pages(c, &path, sel),
        None => {
            let h = open(c);
            let r = parse_one(h, &path);
            unsafe { ffi::sdp_close(h) };
            r
        }
    };
    match r {
        Ok(v) => {
            let s = serde_json::to_string_pretty(&v).unwrap();
            match &c.out {
                Some(o) => {
                    std::fs::write(o, s).ok();
                }
                None => println!("{}", s),
            }
            0
        }
        Err((rc, e)) => {
            eprintln!("실패({}): {}", rc, e);
            rc.abs()
        }
    }
}

//------------------------------------------------------------------
// parse --pages — 고른 쪽만 OCR 해 파싱하고 그 쪽 블록만 남긴다 (시험용)
//=> 라이브러리의 keep_pages 는 OCR 을 하지 않으므로 쓰지 않는다. 대신 CLI 에서:
//    1) deferred 로 한 번 파싱해 OCR 이 남은 쪽(ocr.pending)을 알아낸다
//    2) 그중 고른 쪽만 sdp_ocr_pages 로 OCR 해 캐시에 넣는다(--no-ocr 이면 건너뜀)
//    3) 다시 파싱하고(캐시를 읽음) 쪽 번호가 고른 쪽이 아닌 블록은 뺀다(쪽 번호 없는 블록은 남김)
//   다른 쪽은 OCR 하지 않으므로 ocr.pending 에 그대로 남는다. 머리말 · 꼬리말처럼 여러 쪽을 함께 보는
//   구조 분석은 고른 쪽만으로 판단하므로 문서 전체를 파싱할 때와 다를 수 있다.
//
// -in: c = 명령줄 옵션 / path = 파일 / sel = "1,3-5" 꼴 쪽 고르기
//
// -out: Ok(결과 JSON — blocks 는 고른 쪽만)
// -out: error = 파싱 · OCR 실패면 Err((코드, 사유))
//------------------------------------------------------------------
fn parse_pages(c: &Cli, path: &str, sel: &str) -> Result<Value, (i32, String)> {
    let want: Vec<usize> = pages_json(Some(sel))
        .and_then(|s| serde_json::from_str::<Vec<usize>>(s.to_str().unwrap_or("[]")).ok())
        .unwrap_or_default();
    // 캐시만 보고 남은 쪽을 알려 주는 모드로 연다 — 고르지 않은 쪽을 OCR 하지 않게
    let h = open(&Cli { mode: Some("deferred".into()), ..c.clone() });
    let r = (|| {
        let first = parse_one(h, path)?;
        let pending: Vec<usize> = first.pointer("/ocr/pending").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        let done: Vec<usize> = first.pointer("/ocr/done").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
        // 고른 쪽 중 OCR 대상이 아닌 쪽 — 글자층이 있는 쪽이거나 max_pages 상한 밖(조용히 넘어가지 않게 알린다)
        let skip: Vec<usize> = want.iter().copied().filter(|p| !pending.contains(p) && !done.contains(p)).collect();
        if !skip.is_empty() {
            eprintln!("OCR 대상 아님(글자층 쪽 · 없는 쪽 · --max-pages 상한 밖): {:?}", skip);
        }
        let todo: Vec<usize> = pending.into_iter().filter(|p| want.contains(p)).collect();
        if !todo.is_empty() && !c.no_ocr {
            eprintln!("고른 쪽 OCR {:?}", todo);
            let cp = CString::new(path).unwrap();
            let pj = CString::new(json!(todo).to_string()).unwrap();
            let mut out: *mut c_char = std::ptr::null_mut();
            let rc = unsafe { ffi::sdp_ocr_pages(h, cp.as_ptr(), pj.as_ptr(), Some(on_progress), std::ptr::null_mut(), &mut out) };
            take(rc, out)?;
        }
        let mut v = parse_one(h, path)?;
        // 고른 쪽 블록만 — 쪽 번호가 없는 블록(문서 전체 정보)은 남긴다
        if let Some(bl) = v.get_mut("blocks").and_then(|b| b.as_array_mut()) {
            bl.retain(|b| b.get("page").and_then(|p| p.as_u64()).map_or(true, |p| want.contains(&(p as usize))));
        }
        Ok(v)
    })();
    unsafe { ffi::sdp_close(h) };
    r
}

// 진입점 — 첫 인자로 명령을 고른다
fn main() {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.is_empty() {
        die("사용법: sdp <parse|dump|compare|ocr|ocrfile|version> ...");
    }
    let cmd = argv.remove(0);
    let c = parse_cli(argv);
    let rc = match cmd.as_str() {
        "parse" => cmd_parse(&c),
        "dump" => cmd_dump(&c),
        "compare" => cmd_compare(&c),
        "ocr" => cmd_ocr(&c),
        "ocrfile" => cmd_ocrfile(&c),
        "version" => {
            println!("{} (abi {})", unsafe { CStr::from_ptr(ffi::sdp_version()).to_string_lossy() }, ffi::sdp_abi_version());
            0
        }
        _ => die(&format!("모르는 명령: {}", cmd)),
    };
    std::process::exit(rc);
}
