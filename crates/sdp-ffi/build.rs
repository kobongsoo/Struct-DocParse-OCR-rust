//------------------------------------------------------------------
// sdp-ffi 빌드 스크립트 — struct_docparse.dll 에 Windows 버전 정보 넣기
//=> 파일 속성 '자세히' 탭의 파일 버전 · 제품 이름 · 저작권 · 언어를 채운다(crates/winres.rs).
//   Windows · MSVC 대상이 아니면 아무것도 하지 않는다.
//------------------------------------------------------------------

include!("../winres.rs");

fn main() {
    embed_version(Kind::Dll, "struct_docparse.dll", "struct_docparse 구조화 문서 파서 + OCR 라이브러리");
}
