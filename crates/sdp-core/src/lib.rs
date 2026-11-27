//------------------------------------------------------------------
// sdp-core — 구조화 문서 파서 + OCR 본체 (SimpleRAG Rust 판에서 옮김, 설계서 2026-11-16)
//=> 파일 하나 → 블록 목록(글 + 라벨 + 제목 수준 + 쪽 + 표 칸 + 출처).
//   바깥에서는 api 모듈만 부른다. 나머지는 SimpleRAG 코드를 로직 그대로 옮긴 것이다.
//------------------------------------------------------------------

pub mod api;
pub mod chunk;
pub mod config;
pub mod docmodel;
pub mod extract;
pub mod hash;
pub mod htmlparse;
pub mod legacy;
pub mod log;
pub mod ocr;
pub mod pdfium;
pub mod pyutil;
pub mod session;
pub mod structure;
pub mod xlsdate;
pub mod xmldom;
