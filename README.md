# Struct-DocParse-OCR (struct_docparse.dll)

구조화 문서 파서 + OCR. 파일 하나를 받아 **블록 단위 글 + 라벨**(heading · paragraph · list_item · table · caption · code · header · footer · page_number · toc · note)을 JSON 으로 돌려준다.
SimpleRAG Rust 판의 `docmodel` · `ocr` · `pdfium` · 폴백 추출기를 로직 그대로 옮겨 DLL 로 분리했다(설계서: `SimpleRAG/plan/구조화문서파서_DLL분리_설계서_20261116.html`).

| 형식 | 경로 |
|---|---|
| docx · pptx · xlsx · hwpx · doc · pdf · html · md · ipynb | 블록(라벨 포함) |
| hwp · ppt · xls · txt · csv 등 | 폴백 평문(`format: "legacy"`, `raw_text`) |
| png · jpg · tif · bmp · 스캔 PDF 쪽 | OCR (켜고 끌 수 있음, 기본 켬) |

## 구성

```
crates/sdp-core   파서·OCR 본체 (rlib) — api.rs 가 바깥 입구
crates/sdp-ffi    C ABI DLL → target/release/struct_docparse.dll
crates/sdp-cli    시험·덤프·같음 비교 도구 → target/release/sdp.exe
include/struct_docparse.h     C 헤더 (ABI 1)
bindings/python/struct_docparse.py   ctypes 래퍼(측정용)
tools/            baseline.py(분리 전 기준 만들기) · compare_index.py(두 SimpleRAG 색인 비교)
tools/make_samples.py   시험용 합성 문서 만들기
tests/samples/    지어낸 시험 문서 10건(docx · xlsx · pptx · md · txt · PDF · 스캔 그림 · 스캔 PDF), 목록 tests/samples.txt
tests/private/    실제 업무 문서로 시험할 때 두는 곳 — git 에 올라가지 않는다
```

## 빌드

```bash
cargo build --release
```

실행할 때 DLL 옆(또는 exe 옆)에 `pdfium.dll` · `onnxruntime.dll`(1.22 계열, ort 2.0.0-rc.10) · `models/ocr/*` 가 있어야 한다.
경로는 `sdp_open` 옵션(`runtime.pdfium` · `runtime.onnxruntime` · `ocr.model_dir`)으로 바꿀 수 있다.

## 리눅스

- `bash scripts/linux-build.sh` — 빌드서버(CentOS 7)에서 `libstruct_docparse.so` 빌드 · 시험 · Windows 결과와 비교 · `dist/…-linux-x64.tgz` 묶기
- `bash scripts/linux-examples.sh` — C · Python · Rust · Java 예제를 리눅스에서 실행
- 라이브러리는 glibc 2.15 이상이면 돈다. OCR(onnxruntime 1.22)은 glibc 2.27 이상 — CentOS 7 에서는 OCR 만 안 된다(컨테이너 rockylinux:8 에서 시험).

## 설명서

- `doc/빌드_CLI시험_설명서.html` — 빌드하고 `sdp.exe` 로 시험하기
- `doc/struct_docparse-라이브러리-연동설명서.html` — Python · Rust · C/C++ · Java · PHP 등에서 붙이기, 리눅스, 함수 목록

## OCR 켜기·끄기

- 열 때: `{"ocr": {"enabled": false}}`
- 실행 중: `sdp_set_option(h, "ocr.enabled", "false")` — 끄면 엔진도 내린다
- 호출마다: `sdp_parse(h, path, "{\"ocr\":{\"enabled\":false}}", &out)`
- 모델 파일이 없으면 켜져 있어도 꺼진 것처럼 동작하고 `sdp_get_status` 의 `ocr.reason` 이 `"models_missing"`.
- `mode`: `inline`(기본 — 파싱 안에서 바로 OCR) / `deferred`(캐시만 보고 남은 쪽을 `ocr.pending` 으로 알려 줌 → `sdp_ocr_pages` 로 처리. SimpleRAG 가 쓰는 방식)

## 같음 검증 (분리 전 SimpleRAG 와 글자 하나까지 같은지)

```bash
python tools/baseline.py <분리 전 simplerag-rs.exe> <문서 목록.txt> work/base <SIMPLERAG_HOME>
target/release/sdp.exe dump <문서 목록.txt> work/new --mode deferred [--cache <HOME>/ocr_cache]
target/release/sdp.exe compare work/base work/new
```

문서 목록은 한 줄에 경로 하나(UTF-8). 공개 저장소에는 `tests/samples.txt`(합성 10건)만 있고, 실제 문서 목록은 `tests/private/` 에 둔다.

2026-11-16 결과: 실제 문서 84건(공개 저장소에는 없음) — 캐시 없음 84/84 · 캐시 있음 84/84 · inline 84/84 같음, OCR 줄(스캔 PDF 5쪽 193줄 · 그림 10줄) 같음.

## SimpleRAG 연결

SimpleRAG `rust/simplerag/src/docparse.rs` 가 exe 옆 `struct_docparse.dll`(또는 `SIMPLERAG_DOCPARSE`)을 적재한다.
`rust/build_dist.ps1` 이 이 저장소의 `target/release/struct_docparse.dll` 을 배포 폴더로 복사한다(`SDP_ROOT` 로 위치 변경).

## 버전

- DLL: `Cargo.toml` 의 workspace 버전 (0.1.0)
- ABI: `sdp_abi_version()` = 1 — 함수를 빼거나 인자를 바꿀 때만 올린다
- 추출 결과: `blocks_version` = 5 — 블록 결과가 바뀌는 변경마다 올린다(SimpleRAG 가 재인덱싱 판정에 쓴다)
- OCR 엔진: `ppocrv6-tiny-det+ko-ppocrv5-mobile-rec/1` — 바뀌면 옛 OCR 캐시를 버린다
