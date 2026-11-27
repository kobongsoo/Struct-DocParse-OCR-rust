# Struct-DocParse-OCR (struct_docparse.dll)

구조화 문서 파서 + OCR. 파일 하나를 받아 **블록 단위 글 + 라벨**(heading · paragraph · list_item · table · caption · code · header · footer · page_number · toc · note)을 JSON 으로 돌려준다.
0.3.0 부터는 원하면 그 블록을 **검색(RAG)용 청크**로 잘라 함께 돌려준다(SimpleRAG 와 같은 규칙 — 아래 '블록 단위 청킹').
SimpleRAG Rust 판의 `docmodel` · `ocr` · `pdfium` · 폴백 추출기를 로직 그대로 옮겨 DLL 로 분리했다(설계서: `SimpleRAG/plan/구조화문서파서_DLL분리_설계서_20261116.html`).

| 형식 | 경로 |
|---|---|
| docx · pptx · xlsx · hwpx · doc · pdf · html · md · ipynb | 블록(라벨 포함) |
| hwp · ppt · xls · txt · csv 등 | 폴백 평문(`format: "legacy"`, `raw_text`) |
| png · jpg · tif · bmp · 스캔 PDF 쪽 | OCR (켜고 끌 수 있음, 기본 켬) |

## 받기 — 빌드 없이 바로 쓰기

**빌드한 라이브러리(DLL · .so)는 저장소가 아니라 [GitHub Releases](https://github.com/kobongsoo/Struct-DocParse-OCR-rust/releases) 에 있다.** 최신 판을 받아 풀면 된다.

| 파일 | 플랫폼 |
|---|---|
| `struct_docparse-<버전>-<커밋>-windows-x64.zip` | Windows 10/11 64비트 |
| `struct_docparse-<버전>-<커밋>-linux-x64.tgz` | 리눅스 x86_64 (glibc 2.17 이상, OCR 은 glibc 2.27 이상) |

풀면 `lib/` 에 라이브러리(`struct_docparse.dll` · `libstruct_docparse.so`) · 시험 CLI(`sdp`) · pdfium · onnxruntime · OCR 모델이 함께 있다.
라이브러리가 **자기 옆 폴더**에서 런타임과 모델을 찾으므로 `lib/` 폴더째 둔다. 그 밖에 `include/`(C 헤더) · `bindings/` · `examples/` · `doc/`(설명서) · `licenses/`.

```bash
./lib/sdp version
./lib/sdp parse 문서.docx
```

저장소의 `third_party/`(pdfium · onnxruntime) · `models/ocr/` 는 소스에서 빌드할 때 쓰는 런타임 · 모델이다 — 이 프로젝트가 만든 DLL 은 저장소에 커밋하지 않는다(`target/` 은 git 에서 뺀다).

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
models/ocr/       OCR 모델 3개(PaddlePaddle PP-OCR, Apache-2.0) — 출처 · 체크섬은 models/ocr/README.md
third_party/      런타임 — Windows(pdfium.dll · onnxruntime.dll 1.29.0) · 리눅스(libpdfium.so · libonnxruntime.so 1.22.0) · JNA jar. 출처 · 라이선스는 third_party/README.md
```

## 빌드 (Windows)

### 1. 준비물

| 필요한 것 | 구하는 곳 |
|---|---|
| Rust stable + Visual Studio C++ 빌드 도구 | https://rustup.rs |
| `pdfium.dll` | **저장소에 들어 있다** — `third_party/pdfium/windows-x64/` |
| `onnxruntime.dll` · `onnxruntime_providers_shared.dll` | **저장소에 들어 있다** — `third_party/onnxruntime/windows-x64/`(1.29.0). 다른 판을 쓰려면 1.22 이상(ort 2.0.0-rc.10 은 1.22 미만만 거절) |
| OCR 모델 | 저장소 `models/ocr/` 에 들어 있다 |

그 밖에 Visual C++ 런타임(`vcruntime140.dll` 등)이 있어야 하는데 대부분의 PC 에 이미 있다. OCR 을 안 쓰면 onnxruntime 은 없어도 된다(OCR 이 저절로 꺼진다).

### 2. 빌드

```powershell
git clone https://github.com/kobongsoo/Struct-DocParse-OCR-rust.git
cd Struct-DocParse-OCR-rust
cargo build --release              # → target/release/struct_docparse.dll · struct_docparse.dll.lib · sdp.exe
cargo test --release -p sdp-core   # 단위 시험 53개
```

### 3. 런타임 · 모델을 옆에 두기

라이브러리는 **자기 옆 폴더**에서 런타임과 모델을 찾는다. 처음 한 번 `target/release/` 에 둔다.

```powershell
Copy-Item .\third_party\pdfium\windows-x64\pdfium.dll, .\third_party\onnxruntime\windows-x64\*.dll .\target\release\
New-Item -ItemType Directory -Force .\target\release\models\ocr | Out-Null
Copy-Item .\models\ocr\* .\target\release\models\ocr\
```

다른 곳에 두었다면 `sdp_open` 옵션(`runtime.pdfium` · `runtime.onnxruntime` · `ocr.model_dir`)으로 알려 준다.

### 4. 시험

```powershell
.\target\release\sdp.exe version
.\target\release\sdp.exe parse .\tests\samples\보고서_예시.docx
.\target\release\sdp.exe parse .\tests\samples\스캔_예시.jpg        # OCR
.\target\release\sdp.exe dump .\tests\samples.txt .\work\out          # 시험 문서 10건 한꺼번에
```

### 5. 배포 묶음

```powershell
python scripts\package.py windows-x64 target\release\struct_docparse.dll 0.1.1 target\release\sdp.exe
# → dist\struct_docparse-0.1.1-windows-x64.zip (lib\ 에 DLL · sdp · 런타임 · 모델, include · bindings · examples · doc)
```

`package.py` 는 런타임을 `third_party/`, 모델을 `models/ocr/` 에서 가져가고 라이선스 문서를 묶음의 `licenses/` 에 넣는다.

## 빌드 (리눅스)

리눅스 런타임(`third_party/`)과 모델(`models/ocr/`)이 저장소에 들어 있어 따로 받을 것이 없다.

**리눅스에서 직접** (Rust stable 필요):

```bash
cargo build --release -p sdp-ffi -p sdp-cli      # → target/release/libstruct_docparse.so · sdp
cp third_party/pdfium/linux-x64/libpdfium.so third_party/onnxruntime/linux-x64/*.so target/release/
mkdir -p target/release/models/ocr && cp models/ocr/* target/release/models/ocr/
target/release/sdp parse tests/samples/보고서_예시.docx
```

**Windows 에서 빌드서버로** (Git Bash · PuTTY plink/pscp):

- `bash scripts/linux-build.sh` — 빌드서버(CentOS 7)에 올려 빌드 · 단위 시험 · 시험 문서 비교(Windows 결과와 같아야 함) 후 `dist/…-linux-x64.tgz` 로 묶는다. 접속 정보는 `scripts/server.env`(견본 `server.env.sample`, git 에 안 올라감).
- `bash scripts/linux-examples.sh` — C · Python · Rust · Java 예제를 리눅스에서 실행

라이브러리는 glibc 2.17 이상이면 돈다(0.2.x 는 2.15 — 0.3.0 에서 청킹용 토크나이저가 들어가며 올랐다). OCR(onnxruntime 1.22)은 glibc 2.27 이상 — CentOS 7 에서는 OCR 만 안 된다(빌드서버에서는 컨테이너 rockylinux:8 로 시험).

## 설명서

- `doc/빌드_CLI시험_설명서.html` — 빌드하고 `sdp.exe` 로 시험하기
- `doc/struct_docparse-라이브러리-연동설명서.html` — Python · Rust · C/C++ · Java · PHP 등에서 붙이기, 리눅스, 함수 목록

## OCR 켜기·끄기

- 열 때: `{"ocr": {"enabled": false}}`
- 실행 중: `sdp_set_option(h, "ocr.enabled", "false")` — 끄면 엔진도 내린다
- 호출마다: `sdp_parse(h, path, "{\"ocr\":{\"enabled\":false}}", &out)`
- 모델 파일이 없으면 켜져 있어도 꺼진 것처럼 동작하고 `sdp_get_status` 의 `ocr.reason` 이 `"models_missing"`.
- `mode`: `inline`(기본 — 파싱 안에서 바로 OCR) / `deferred`(캐시만 보고 남은 쪽을 `ocr.pending` 으로 알려 줌 → `sdp_ocr_pages` 로 처리. SimpleRAG 가 쓰는 방식)
- `layout`: OCR 줄 구조 분석(기본 `true`, 블록 버전 6부터). 켜면 OCR 상자 좌표로 다단 순서 · 여러 줄 문단 · 제목 · 목록 · 목차 · 그림/표 설명 · 머리말/꼬리말/쪽번호를 가린다. 끄면 이전처럼 화면 한 줄이 `paragraph` 하나. 끄기: 열 때 `{"ocr":{"layout":false}}` · 실행 중 `sdp_set_option(h, "ocr.layout", "false")` · `sdp.exe … --no-layout`(끄면 블록 버전 5 출력과 같다). 설계 · 측정은 `plan/OCR구조분석_1단계_설계서_20261116.html`

## 본문 밖 글 읽기 (0.2.0, 기본 켬)

사이냅(snf)이 읽는 본문 밖 글을 같이 읽어 두 엔진의 본문을 맞춘다. 블록의 `source` 가 `"extra"` 다.

| 형식 | 읽는 것 | 자리 |
|---|---|---|
| PDF | 글상자(FreeText) 주석 `/Contents` — 숨긴 것도 · 서식 칸(글 칸 · 콤보 · 목록) 값 | 그 쪽의 글자층 줄 뒤, OCR 줄 앞 |
| docx | 메모(comment) | 문서 끝 합성 제목 "메모" 아래 `note` |
| pptx · docx | SmartArt 글(`*/diagrams/dataN.xml`) | 그 그림이 나온 자리 |

- OCR 하는 스캔 쪽은 보이는 주석이 그림에 그려져 OCR 이 읽으므로 숨긴 주석만 글로 붙인다(같은 글이 두 번 들어가지 않게).
- 쪽지(Text) 주석 · 도장 · 체크 상자 값은 읽지 않는다. 스캔 쪽 판정(OCR 할 쪽)은 옵션과 상관없이 같다.
- 끄기: 열 때 `{"extra_text": false}` · 실행 중 `sdp_set_option(h, "extra_text", "false")` · `sdp.exe … --no-extra-text`(끄면 0.1.x 본문과 같다).
- 옵션과 상관없이 고친 것: xlsx 셀 메모를 시트 관계로도 찾는다(openpyxl 이 저장한 `xl/comments/comment1.xml` 꼴).
- 설계 · 사이냅 비교: `plan/본문밖글읽기_설계서_20261118.html`

## 블록 단위 청킹 (0.3.0)

블록을 검색(RAG)용 청크로 잘라 준다. SimpleRAG(Rust 판) `chunk.rs` 의 규칙을 그대로 옮겼다 — 같은 문서 · 같은 설정이면 SimpleRAG 와 글자 하나까지 같은 청크가 나온다.

- 제목 수준으로 절을 나누고(머리글 · 바닥글 · 쪽 번호 · 목차는 뺌), 작은 절은 크기 안에서 이어 묶고, 큰 절은 제목 · 블록 · 표 행 단위로 자른다
- 표는 행 경계에서 자르고 조각마다 머리글 행을 다시 붙인다. 작은 표(`table_whole` 이하)는 통째로
- 청크 앞에 `[문서 제목 > 절]` 접두, 청크가 시작하는 쪽 · 걸친 쪽 범위
- 510 토큰(`hard_max`)을 넘는 청크는 다시 자르고, `min_tokens` 미만은 버린다
- 평문(legacy) 형식도 조문 · 번호 절을 찾아 자른다

길이는 **e5 토크나이저 토큰 수**로 잰다 — `tokenizer.json` 경로를 옵션으로 준다(묶음에는 들어 있지 않다. SimpleRAG 의 `models/e5-small-ko/tokenizer.json`). 토크나이저가 없으면 `"unit": "chars"` 로 글자 수로 잰다.

```c
sdp_open("{\"chunk\":{\"tokenizer\":\"models/e5-small-ko/tokenizer.json\"}}", NULL, NULL, &h);  // 기본값(켜지지는 않음)
sdp_parse(h, path, "{\"chunk\":true}", &out);                    // 이 호출에서 켜기 → 결과에 chunks
sdp_parse(h, path, "{\"chunk\":{\"tokens\":256}}", &out);         // 이 호출만 크기 256
sdp_chunk(h, saved_parse_json, "{\"tokens\":64}", &out);          // 저장한 파싱 결과를 다시 자르기(파싱 · OCR 없이)
```

- 결과: 청킹을 켠 호출에만 `chunk_version` · `chunking`(실제 쓴 설정) · `chunks`(`no` · `tokens` · `section_id` · `path` · `source` · `page` · `page_from` · `page_to` · `hash` · `body_hash` · `text`)가 붙는다. 끈 호출은 0.2.0 과 같다
- 중복 지문(0.3.1~): `hash` = `text` 그대로의 SHA-256 앞 16자리(임베딩 재사용 · 같은 사본), `body_hash` = 접두 줄을 떼고 공백을 정리한 본문의 지문(다른 문서에 든 같은 본문). 자세한 규칙은 연동설명서 3-11
- 옵션(`chunk.*`): `tokenizer` · `unit`(tokens | chars) · `mode`(structured | fixed) · `tokens`(128) · `overlap`(주지 않으면 max(8, tokens/8)) · `min_tokens`(16) · `table_header_max_tokens`(64) · `table_whole`(384) · `overflow_ratio`(1.0) · `sentence_split`(false) · `hard_max`(510) · `prefix`(true) · `prefix_max_chars`(80). 실행 중 `sdp_set_option(h, "chunk.tokens", "256")`
- 바인딩: Python `parse(path, chunk=True)` · `chunk(doc, opts)`, Rust `parse(path, Some(r#"{"chunk":true}"#))` · `chunk(json, opts)`, Java `parse(path, "{\"chunk\":true}")` · `chunk(json, opts)`, PHP `parse($path, ['chunk' => true])` · `chunk($doc, $opts)`
- CLI: `sdp parse 문서.docx --chunk --tokenizer <tokenizer.json>` · `sdp chunk <목록.txt> <폴더> --tokenizer <…> [--chunk-tokens N] [--chars]` → `<이름>.chunks.json`
- 지원 확인: `sdp_version()` 에 `chunk=` 가 있으면 된다. ABI 는 1 그대로
- 설계 · 시험: `plan/블록청킹_DLL옵션_설계서_20261127.md`

## 같음 검증 (분리 전 SimpleRAG 와 글자 하나까지 같은지)

분리 전 SimpleRAG 는 OCR 구조 분석 · 본문 밖 글이 없으므로 `--no-layout --no-extra-text` 로 비교한다(블록 버전 5 출력).

```bash
python tools/baseline.py <분리 전 simplerag-rs.exe> <문서 목록.txt> work/base <SIMPLERAG_HOME>
target/release/sdp.exe dump <문서 목록.txt> work/new --mode deferred --no-layout --no-extra-text [--cache <HOME>/ocr_cache]
target/release/sdp.exe compare work/base work/new
```

문서 목록은 한 줄에 경로 하나(UTF-8). 공개 저장소에는 `tests/samples.txt`(합성 10건)만 있고, 실제 문서 목록은 `tests/private/` 에 둔다.

2026-11-16 결과: 실제 문서 84건(공개 저장소에는 없음) — 캐시 없음 84/84 · 캐시 있음 84/84 · inline 84/84 같음, OCR 줄(스캔 PDF 5쪽 193줄 · 그림 10줄) 같음.

## SimpleRAG 연결

SimpleRAG `rust/simplerag/src/docparse.rs` 가 exe 옆 `struct_docparse.dll`(또는 `SIMPLERAG_DOCPARSE`)을 적재한다.
`rust/build_dist.ps1` 이 이 저장소의 `target/release/struct_docparse.dll` 을 배포 폴더로 복사한다(`SDP_ROOT` 로 위치 변경).
그래서 SimpleRAG 를 빌드하기 전에 이 저장소에서 `cargo build --release -p sdp-ffi` 를 먼저 한다. 두 저장소를 같은 상위 폴더에 두면 따로 설정할 것이 없다.

## 버전

- DLL: `Cargo.toml` 의 workspace 버전 (0.3.1)
  - Windows: `struct_docparse.dll` · `sdp.exe` 파일 속성 '자세히' 탭에 파일 버전 · 제품 이름 · 저작권 · 언어(빌드 때 `crates/winres.rs` 가 VERSIONINFO 를 넣는다 — Windows SDK 의 `rc.exe` 필요, 없으면 경고만)
  - 리눅스: `readelf -n libstruct_docparse.so` 의 `struct_docparse` 노트, 또는 `strings libstruct_docparse.so | grep struct_docparse`
- ABI: `sdp_abi_version()` = 1 — 함수를 빼거나 인자를 바꿀 때만 올린다
- 추출 결과: `blocks_version` = 7 — 블록 결과가 바뀌는 변경마다 올린다(SimpleRAG 가 재인덱싱 판정에 쓴다). 7 = 본문 밖 글(0.2.0)
- 청크 결과: `chunk_version` = 1 — 청크 결과가 바뀌는 변경마다 올린다(0.3.0~). 청크를 저장하는 쪽은 `blocks_version` 과 함께 본다
- OCR 엔진: `ppocrv6-tiny-det+ko-ppocrv5-mobile-rec/1` — 바뀌면 옛 OCR 캐시를 버린다

## 라이선스

이 저장소의 코드(라이브러리 · CLI · 바인딩 · 예제 · 스크립트 · 설명서)는 [MIT 라이선스](LICENSE)를 따른다.

함께 든 바깥 파일은 각자의 라이선스를 따른다 — 자세한 것은 `third_party/README.md` · `models/ocr/README.md`.

| 파일 | 라이선스 |
|---|---|
| `third_party/pdfium/` | BSD-3-Clause · Apache-2.0 |
| `third_party/onnxruntime/` | MIT (Microsoft) |
| `third_party/java/jna-5.14.0.jar` | Apache-2.0 / LGPL-2.1 |
| `models/ocr/` (PaddlePaddle PP-OCR) | Apache-2.0 |
