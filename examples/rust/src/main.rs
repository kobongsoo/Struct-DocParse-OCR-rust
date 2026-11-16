//------------------------------------------------------------------
// struct_docparse 러스트 예제 — 문서마다 형식 · 블록 수 · 앞 블록 3개
//=> 바인딩(bindings/rust/struct_docparse.rs)을 그대로 끌어와 쓴다.
//   cargo run --release -- 문서1 [문서2 ...]
//   라이브러리 경로: SDP_LIB 환경변수 → 묶음 lib/ → 저장소 target/release
//------------------------------------------------------------------
#[path = "../../../bindings/rust/struct_docparse.rs"]
mod struct_docparse;

use struct_docparse::StructDocParse;

// 운영체제별 라이브러리 파일 이름
const LIB: &str = if cfg!(windows) { "struct_docparse.dll" } else { "libstruct_docparse.so" };

//------------------------------------------------------------------
// 라이브러리 경로 찾기
//
// -in: 없음
//
// -out: 경로 / 못 찾으면 프로그램 종료
// -out: error = 없으면 종료 코드 2
//------------------------------------------------------------------
fn find_lib() -> String {
    let here = env!("CARGO_MANIFEST_DIR");
    let cands = [
        std::env::var("SDP_LIB").unwrap_or_default(),
        format!("{}/../../lib/{}", here, LIB),
        format!("{}/../../target/release/{}", here, LIB),
    ];
    for c in cands {
        if !c.is_empty() && std::path::Path::new(&c).is_file() {
            return c;
        }
    }
    eprintln!("라이브러리를 찾지 못했습니다 — SDP_LIB 에 {} 경로를 주세요", LIB);
    std::process::exit(2)
}

// 진입점 — 인자로 받은 문서를 차례로 파싱
fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    let sdp = match StructDocParse::open(&find_lib(), r#"{"ocr":{"enabled":true,"mode":"inline"}}"#) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("열기 실패: {}", e);
            std::process::exit(1)
        }
    };
    println!("struct_docparse {}", sdp.version());
    for f in &files {
        match sdp.parse(f, None) {
            Ok(d) => {
                let blocks = d["blocks"].as_array().cloned().unwrap_or_default();
                println!("  {} : {} · 블록 {}", f, d["format"].as_str().unwrap_or(""), blocks.len());
                for b in blocks.iter().take(3) {
                    let t: String = b["text"].as_str().unwrap_or("").chars().take(50).collect();
                    println!("      [{}] {}", b["type"].as_str().unwrap_or(""), t.replace('\n', " "));
                }
            }
            Err(e) => println!("  {} : 실패 {}", f, e),
        }
    }
    // 이번 호출만 OCR 끄기 — 그림이면 -4 가 정상
    if let Some(img) = files.iter().find(|f| f.to_lowercase().ends_with(".jpg") || f.to_lowercase().ends_with(".png")) {
        match sdp.parse(img, Some(r#"{"ocr":{"enabled":false}}"#)) {
            Ok(_) => println!("OCR 끔: 성공(뜻밖)"),
            Err(e) => println!("OCR 끔: {}", e),
        }
    }
}
