//------------------------------------------------------------------
// sdp-cli 빌드 스크립트 — sdp.exe 에 Windows 버전 정보 넣기
//=> 파일 속성 '자세히' 탭의 파일 버전 · 제품 이름 · 저작권 · 언어를 채운다(crates/winres.rs).
//   Windows · MSVC 대상이 아니면 아무것도 하지 않는다.
//------------------------------------------------------------------

include!("../winres.rs");

fn main() {
    embed_version(Kind::Exe, "sdp.exe", "struct_docparse 시험 CLI (sdp)");
}
