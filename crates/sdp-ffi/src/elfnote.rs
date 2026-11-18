//------------------------------------------------------------------
// 리눅스 libstruct_docparse.so 버전 정보 — ELF 노트 (.note.struct_docparse)
//=> 리눅스 .so 에는 Windows 처럼 파일 속성 창의 버전 칸이 없다. 대신 리눅스에서 쓰는 방식으로
//   파일 안에 버전 · 제품 이름 · 저작권을 넣어, 라이브러리를 적재하지 않고도 확인할 수 있게 한다.
//    · readelf -n libstruct_docparse.so   → Owner "struct_docparse" 노트의 글
//    · strings libstruct_docparse.so | grep struct_docparse
//   Windows 의 DLL 은 crates/winres.rs 의 VERSIONINFO 리소스가 같은 일을 한다.
//   노트는 ELF 형식대로 [이름 길이, 글 길이, 종류, 이름(4바이트 맞춤), 글(4바이트 맞춤)] 이다.
//   글 길이는 Cargo 버전에 따라 달라지므로 컴파일할 때 const fn 으로 배열을 채운다.
//------------------------------------------------------------------

// 노트에 넣을 글 — Cargo.toml 의 버전이 바뀌면 함께 바뀐다
const DESC: &str = concat!(
    "struct_docparse ", env!("CARGO_PKG_VERSION"),
    " - Copyright (c) 2026 kobongsoo. MIT License"
);
// 노트 주인 이름(끝 NUL 포함 16바이트 — 4의 배수라 맞춤 칸이 필요 없다)
const NAME: &[u8; 16] = b"struct_docparse\0";
// 글 칸 크기 — 끝 NUL 1바이트를 더하고 4바이트 단위로 올린다
const DESC_LEN: usize = DESC.len() + 1;
const DESC_PAD: usize = (DESC_LEN + 3) / 4 * 4;

//------------------------------------------------------------------
// ELF 노트 하나 (Elf64_Nhdr + 이름 + 글)
// -필드: namesz = 이름 길이(NUL 포함) / descsz = 글 길이(NUL 포함) / kind = 노트 종류(1 = 버전 글)
// -필드: name   = 주인 이름 / desc = 버전 글(남는 칸은 0)
//------------------------------------------------------------------
#[repr(C, align(4))]
pub struct Note {
    namesz: u32,
    descsz: u32,
    kind: u32,
    name: [u8; 16],
    desc: [u8; DESC_PAD],
}

//------------------------------------------------------------------
// 글을 고정 크기 배열로 옮기기 (컴파일 때 계산)
//=> const 문맥이라 반복문으로 한 바이트씩 옮긴다. 남는 칸은 0(NUL 끝 · 맞춤)으로 둔다.
//
// -in: 없음 (DESC 를 쓴다)
//
// -out: [u8; DESC_PAD]
// -out: error = 없음
//------------------------------------------------------------------
const fn desc_bytes() -> [u8; DESC_PAD] {
    let src = DESC.as_bytes();
    let mut out = [0u8; DESC_PAD];
    let mut i = 0;
    while i < src.len() {
        out[i] = src[i];
        i += 1;
    }
    out
}

// 노트 본체 — 내보낸 이름(no_mangle)이라 링커가 쓰지 않는 칸으로 보고 지우지 않는다.
// 이름이 .note 로 시작하는 구역은 ELF 노트(SHT_NOTE)로 만들어져 readelf -n 에 나온다
#[used]
#[no_mangle]
#[link_section = ".note.struct_docparse"]
pub static SDP_VERSION_NOTE: Note = Note {
    namesz: NAME.len() as u32,
    descsz: DESC_LEN as u32,
    kind: 1,
    name: *NAME,
    desc: desc_bytes(),
};
