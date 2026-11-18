//------------------------------------------------------------------
// Windows 버전 정보 넣기 — sdp-ffi · sdp-cli 의 build.rs 가 include! 로 함께 쓴다
//=> Rust 빌드는 DLL · exe 에 Windows 버전 정보(VERSIONINFO)를 넣지 않아, 파일 속성의 '자세히' 탭에
//   파일 버전 · 제품 이름 · 저작권 · 언어가 비어 보인다. 빌드할 때 리소스를 만들어 링크에 붙인다.
//    1) Windows · MSVC 대상일 때만 한다(리눅스 .so 에는 이런 칸이 없다)
//    2) Cargo.toml 의 버전(CARGO_PKG_VERSION)으로 .rc 를 OUT_DIR 에 쓴다(UTF-8, 한국어 설명)
//    3) rc.exe 로 .res 를 만들고 링커 인자로 붙인다 — 새 크레이트 없이(리눅스 빌드서버에 받을 것이 늘지 않게)
//   rc.exe 를 못 찾거나 실패하면 경고만 남기고 빌드는 계속한다(버전 정보만 빠진다).
//   rc.exe 위치: 환경변수 SDP_RC_EXE → Windows SDK(Windows Kits\10\bin\<가장 높은 판>\x64\rc.exe) → PATH 의 rc.exe
//------------------------------------------------------------------

use std::path::{Path, PathBuf};

//------------------------------------------------------------------
// 무엇에 붙이나
// -필드: Dll = 라이브러리(cdylib) — FILETYPE VFT_DLL
// -필드: Exe = 실행 파일(bin) — FILETYPE VFT_APP
//------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum Kind {
    Dll,
    Exe,
}

//------------------------------------------------------------------
// 버전 정보를 만들어 링크에 붙이기 (build.rs 의 main 에서 부른다)
//=> 1) 대상이 Windows · MSVC 가 아니면 아무것도 안 한다
//   2) "0.2.0" → 숫자 버전 0,2,0,0 과 문자열 버전 "0.2.0"
//   3) .rc 쓰기 → rc.exe 로 .res → cargo:rustc-link-arg-cdylib / -bins 로 붙이기
//
// -in: kind     = Dll | Exe
// -in: filename = 원래 파일 이름(예: "struct_docparse.dll") — OriginalFilename 칸
// -in: desc     = 파일 설명(속성 창 '파일 설명')
//
// -out: 없음
// -out: error = 실패해도 빌드를 멈추지 않는다 — cargo:warning 으로 알리고 버전 정보 없이 계속
//------------------------------------------------------------------
fn embed_version(kind: Kind, filename: &str, desc: &str) {
    // 버전 · 대상이 바뀌면 다시 만든다
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../winres.rs");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    println!("cargo:rerun-if-env-changed=SDP_RC_EXE");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // 리눅스 · MinGW 대상은 건너뛴다 — .res 를 링커 인자로 넘기는 방식은 MSVC 링커 것이다
    if os != "windows" || env != "msvc" {
        return;
    }
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let rc = out.join("version.rc");
    let res = out.join("version.res");
    if let Err(e) = std::fs::write(&rc, rc_text(kind, filename, desc, &ver)) {
        println!("cargo:warning=버전 정보 .rc 쓰기 실패: {} — 버전 정보 없이 빌드합니다", e);
        return;
    }
    let exe = match find_rc() {
        Some(p) => p,
        None => {
            println!("cargo:warning=rc.exe 를 찾지 못했습니다(Windows SDK) — 버전 정보 없이 빌드합니다. SDP_RC_EXE 로 지정할 수 있습니다");
            return;
        }
    };
    // /nologo = 머리말 끔, /c65001 = .rc 를 UTF-8 로 읽기(한국어 설명), /fo = 결과 파일
    let st = std::process::Command::new(&exe).arg("/nologo").arg("/c65001").arg("/fo").arg(&res).arg(&rc).status();
    match st {
        Ok(s) if s.success() => {}
        Ok(s) => {
            println!("cargo:warning=rc.exe 실패({}) — 버전 정보 없이 빌드합니다", s);
            return;
        }
        Err(e) => {
            println!("cargo:warning=rc.exe 실행 실패: {} — 버전 정보 없이 빌드합니다", e);
            return;
        }
    }
    // MSVC 링커는 .res 파일을 입력으로 받는다 — 이 크레이트의 해당 산출물에만 붙인다
    let arg = if kind == Kind::Dll { "cargo:rustc-link-arg-cdylib" } else { "cargo:rustc-link-arg-bins" };
    println!("{}={}", arg, res.display());
}

//------------------------------------------------------------------
// .rc 글 만들기 — VERSIONINFO 하나(한국어 0x0412, 유니코드 코드 페이지 1200)
//=> 숫자 버전은 "주.부.고침" 에서 숫자만 떼어 4칸(끝은 0)으로. 미리보기 꼬리("-beta" 등)는 숫자에서 뺀다.
//   문자열 칸 안의 따옴표는 rc 문법대로 두 번 쓴다.
//
// -in: kind · filename · desc = embed_version 과 같음
// -in: ver = CARGO_PKG_VERSION (예: "0.2.0")
//
// -out: .rc 글
// -out: error = 없음
//------------------------------------------------------------------
fn rc_text(kind: Kind, filename: &str, desc: &str, ver: &str) -> String {
    let mut n = [0u32; 4];
    for (i, part) in ver.split(['.', '-', '+']).take(3).enumerate() {
        n[i] = part.parse().unwrap_or(0);
    }
    let num = format!("{},{},{},{}", n[0], n[1], n[2], n[3]);
    // rc 문자열 안의 " 는 "" 로
    let q = |s: &str| s.replace('"', "\"\"");
    // VFT_DLL = 2, VFT_APP = 1
    let ftype = if kind == Kind::Dll { "0x2L" } else { "0x1L" };
    let internal = Path::new(filename).file_stem().and_then(|s| s.to_str()).unwrap_or(filename).to_string();
    format!(
        r#"#pragma code_page(65001)
1 VERSIONINFO
FILEVERSION {num}
PRODUCTVERSION {num}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE {ftype}
FILESUBTYPE 0x0L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "041204b0"
    BEGIN
      VALUE "CompanyName", "kobongsoo"
      VALUE "FileDescription", "{desc}"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "{internal}"
      VALUE "LegalCopyright", "Copyright (c) 2026 kobongsoo. MIT License"
      VALUE "OriginalFilename", "{file}"
      VALUE "ProductName", "struct_docparse"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0412, 1200
  END
END
"#,
        num = num,
        ftype = ftype,
        desc = q(desc),
        ver = q(ver),
        internal = q(&internal),
        file = q(filename),
    )
}

//------------------------------------------------------------------
// rc.exe 찾기
//=> 1) 환경변수 SDP_RC_EXE
//   2) Windows SDK — "C:\Program Files (x86)\Windows Kits\10\bin\<판>\x64\rc.exe" 가운데 가장 높은 판
//   3) PATH 의 rc.exe (개발자 명령 창)
//
// -in: 없음
//
// -out: Some(경로) / 못 찾으면 None
// -out: error = 없음
//------------------------------------------------------------------
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SDP_RC_EXE") {
        if Path::new(&p).is_file() {
            return Some(p.into());
        }
    }
    let base = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
    let kits = Path::new(&base).join("Windows Kits").join("10").join("bin");
    let mut found: Vec<(Vec<u32>, PathBuf)> = vec![];
    if let Ok(rd) = std::fs::read_dir(&kits) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path().join("x64").join("rc.exe");
            // 판 이름("10.0.26100.0")을 숫자로 견줘 가장 높은 것을 고른다
            if p.is_file() {
                let key: Vec<u32> = name.split('.').map(|x| x.parse().unwrap_or(0)).collect();
                found.push((key, p));
            }
        }
    }
    found.sort();
    if let Some((_, p)) = found.pop() {
        return Some(p);
    }
    // 개발자 명령 창처럼 PATH 에 있으면 그것을 쓴다
    std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join("rc.exe")).find(|p| p.is_file()))
}
