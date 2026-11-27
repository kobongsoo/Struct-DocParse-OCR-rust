# struct_docparse 라이브러리 연동 요약

> 원본: `struct_docparse-라이브러리-연동설명서.html` (2026-11-16 · 라이브러리 0.1.0 · ABI 1 · 2026-11-27 라이브러리 0.3.0 — 블록 단위 청킹)

문서(docx · pptx · xlsx · hwpx · doc · hwp · pdf · html · md · 그림 등)를 넣으면 **블록 단위 글 + 라벨**(heading · paragraph · table · header · footer 등)을 JSON으로 돌려주는 라이브러리다. 함수가 C ABI라서 C 함수를 부를 수 있는 언어라면 어디서든 붙일 수 있다.

## 1. 언어별 연동 방식

| 언어 | 방식 | 준비물 | 시험 여부 |
|---|---|---|---|
| Python 3 | ctypes | `bindings/python/struct_docparse.py` | Windows · 리눅스(3.9) 시험함 |
| Rust | libloading (실행 중에 열기) | `bindings/rust/struct_docparse.rs` + `libloading` · `serde_json` | Windows · 리눅스 시험함 |
| C / C++ | 헤더 + 링크 | `include/struct_docparse.h`, Windows는 `.dll.lib` | MSVC · gcc 4.8 시험함 |
| Java 8+ | JNA 5 | `bindings/java/structdocparse/*.java` + `jna-5.14.0.jar` | Java 21 · 8 시험함 |
| PHP 7.4+ | FFI 확장 | `bindings/php/StructDocParse.php` | **미시험** |
| C# · Node.js · Go | P/Invoke · koffi · cgo | 원본 9장의 코드 조각 | **미시험** |

**공통 호출 흐름:** 라이브러리 열기 → `sdp_open(옵션)` → `sdp_parse(파일)` → 결과 JSON 읽기 → `sdp_free` → `sdp_close`

## 2. 배포 묶음 구성

```
struct_docparse-<버전>-{windows|linux}-x64/
├─ lib/        ← 폴더째 둔다 (파일끼리 같은 폴더에 있어야 함)
│  ├─ struct_docparse.dll / libstruct_docparse.so (+ Windows: .dll.lib)
│  ├─ sdp(.exe)              시험용 CLI
│  ├─ pdfium                 PDF
│  ├─ onnxruntime 1.22.0     OCR 계산
│  └─ models/ocr/            OCR 모델 3개
├─ include/struct_docparse.h
├─ bindings/   python · rust · java · php
├─ examples/   python · rust · c · java · php
└─ doc/ · README.md · VERSION · SHA256SUMS
```

먼저 CLI로 동작을 확인한다: `./lib/sdp version`, `./lib/sdp parse 문서.hwpx --out result.json`

## 3. 모든 언어에 공통인 규칙

- **`lib/` 폴더는 통째로 둔다.** 라이브러리가 pdfium · onnxruntime · 모델을 자기 옆에서 먼저 찾는다. 다른 곳에 두었다면 `runtime.pdfium`, `runtime.onnxruntime`, `ocr.model_dir` 옵션으로 경로를 알려 준다.
- **onnxruntime 버전:** 1.22 이상이어야 한다(1.22 미만은 거절, 더 새 버전은 경고만). Windows에서 시스템 폴더의 다른 버전이 먼저 잡히면 OCR이 실패한다. `ORT_DYLIB_PATH`가 있으면 그 값을 따른다.
- **문자열은 모두 UTF-8(끝에 0).**
  - Windows C/C++의 `argv`는 CP949라서 한글 경로를 그대로 넘기면 `-1`이 난다. `GetCommandLineW` + `WideCharToMultiByte(CP_UTF8)`로 바꿔 넘긴다.
  - Java 바인딩은 `byte[]`(UTF-8 + 0)로 넘긴다.
- **결과 문자열은 반드시 `sdp_free`로 해제한다.** 언어 쪽 free를 쓰면 안 된다. 단, `sdp_version()`과 `sdp_last_error()`의 반환값은 해제하지 않는다.
- **ABI 확인:** 열자마자 `sdp_abi_version() == 1`인지 확인한다.
- **스레드:** 핸들 하나를 여러 스레드가 함께 써도 된다. 옵션은 프로세스에 한 벌이라 핸들을 여럿 열면 마지막에 연 옵션이 적용된다.

### 반환 코드

| 코드 | 이름 | 뜻 |
|---|---|---|
| 0 | `SDP_OK` | 성공 (평문 폴백도 0, `format: "legacy"`) |
| -1 | `SDP_EINVAL` | 널 포인터 · 잘못된 JSON · 모르는 옵션 · UTF-8 아님 |
| -2 | `SDP_EIO` | 파일 없음 · 열 수 없음 |
| -3 | `SDP_EPARSE` | 구조 파싱과 평문 추출이 모두 실패 |
| -4 | `SDP_EUNSUPPORTED` | 지원 안 하는 형식, OCR 끈 상태의 그림 |
| -5 | `SDP_EOCR` | OCR 사용 불가 또는 OCR 실패 |
| -7 | `SDP_ERUNTIME` | pdfium 적재 실패 |
| -99 | `SDP_EPANIC` | 라이브러리 내부 오류를 잡음 (프로그램은 계속 돈다) |

0이 아니면 `sdp_last_error()`로 사유를 읽는다.

### 열 때 옵션 JSON (빠진 키는 기본값)

| 키 | 기본 | 설명 |
|---|---|---|
| `ocr.enabled` | true | OCR 켬/끔 |
| `ocr.images` | true | 그림 파일도 OCR |
| `ocr.mode` | inline | `inline` = 파싱 중 OCR / `deferred` = 캐시만 보고 남은 쪽은 `ocr.pending`으로 |
| `ocr.model_dir`, `ocr.cache_dir` | 라이브러리 옆 / 메모리 | 모델 · 캐시 폴더 |
| `ocr.threads`, `ocr.max_pages` | 4, 100 | |
| `ocr.layout` | true | OCR 줄 구조 분석 (다단 · 문단 · 제목 등) |
| `ocr.arena` (0.1.1~) | true | OCR 메모리 아레나 |
| `ocr.page_cap` (0.1.1~) | "a4" | A4보다 큰 쪽을 A4로 줄여 OCR (`"none"`이면 원래 크기) |
| `ocr.image_cap` (0.1.1~) | false | 그림 파일에도 page_cap 적용 |
| `ocr.page_up` (0.1.1~) | true | A4보다 작은 쪽을 최대 1.4배 해상도로 OCR |
| `ocr.orient` (0.1.1~) | true | 누운 · 뒤집힌 스캔 쪽 방향 보정 |
| `ocr.orient_flip` (0.1.1~) | false | 바로 선 쪽도 180° 뒤집힘 확인 (쪽마다 0.02~0.31초 추가) |
| `pdf.tables`, `pdf.infer_headings` | true, false | |
| `log.level` | warn | warn · info · off |
| `extra_text` (0.2.0~) | true | 본문 밖 글(PDF 글상자 · 서식 칸 · docx 메모 · SmartArt) 읽기 |
| `chunk.*` (0.3.0~) | 아래 "청크로 받기" | 청킹 기본값. 여기 적어도 켜지지 않는다(켜는 것은 호출마다) |

- 실제 JSON에는 주석을 넣으면 안 된다. **모르는 키는 `-1`로 거절**하므로, 새 옵션을 이전 버전 라이브러리에 넘기면 열기가 실패한다.

### OCR 켜기·끄기 (세 단계)

- 열 때: `{"ocr":{"enabled":false}}`
- 실행 중: `sdp_set_option(h, "ocr.enabled", "false")` (끄면 엔진도 내려 메모리를 돌려준다)
- 호출마다: `sdp_parse(h, path, "{\"ocr\":{\"enabled\":false}}", &out)`

모델 파일이 없으면 꺼진 것처럼 동작하고, `sdp_get_status`의 `ocr.reason`이 `"models_missing"`이 된다.

### 결과 JSON

```json
{
  "schema": 1, "blocks_version": 7, "parser": "struct_docparse 0.2.0",
  "format": "docx", "title": "보고서_예시", "pages": null, "warnings": [], "raw_text": null,
  "ocr": {"enabled": true, "reason": null, "pending": [], "done": [], "whole": false},
  "blocks": [
    {"type": "heading", "text": "1. 개요", "level": 2},
    {"type": "table", "text": "| 항목 | 금액 |…", "rows": [["항목","금액"]], "header_rows": 1, "page": 2},
    {"type": "paragraph", "text": "그림에서 읽은 글", "page": 3, "source": "ocr"}
  ]
}
```

- **블록 type:** heading(level 1~9) · paragraph · list_item · table(rows · header_rows) · caption · code · header · footer · page_number · toc · note
- **source:** 없음 = 파서가 읽은 글, `ocr`, `inferred`(글자 크기로 짐작한 제목), `extra`(본문 밖 글)
- **legacy 형식**(hwp · ppt · xls · txt · csv 등)은 `blocks`가 빈 배열이고 `raw_text`에 평문이 온다. 실패가 아니다.
- `blocks_version`이 바뀌면 결과 모양이 달라진 것이다(저장한 결과를 다시 만들지 판단할 때 쓴다).

### 청크로 받기 — 블록 단위 청킹 (0.3.0~)

> **0.3.0부터** 쓸 수 있다. 0.2.x 라이브러리에 `chunk` 옵션을 넘기면 `-1`이 나고 `sdp_chunk` 함수도 없다 — 라이브러리와 바인딩을 함께 올린다. 설계 · 시험: `plan/블록청킹_DLL옵션_설계서_20261127.md`

블록을 검색(RAG)용 청크로 자르는 일을 라이브러리가 대신 한다. 규칙은 SimpleRAG와 같다.

- 제목 수준으로 절을 나눈다(머리글 · 바닥글 · 쪽 번호 · 목차는 뺌).
- 작은 절은 크기 안에서 이어 묶고, 큰 절은 제목 · 블록 · 표 행 단위로 자른다.
- 표는 행 경계에서 자르고 조각마다 머리글 행을 다시 붙인다. 작은 표(`table_whole` 이하)는 통째로 둔다.
- 청크 앞에 `[문서 제목 > 절]` 접두를 붙이고, 청크가 시작하는 쪽을 찾는다.
- 510토큰(`hard_max`)을 넘는 청크는 다시 자르고, `min_tokens`보다 짧은 청크는 버린다.

**켜기 — 호출마다.** 열 때 옵션의 `chunk`는 기본값일 뿐이다.

```c
sdp_open("{\"chunk\":{\"tokenizer\":\"models/e5-small-ko/tokenizer.json\"}}", NULL, NULL, &h);
sdp_parse(h, path, "{\"chunk\":true}", &out);                              /* 켜기만 */
sdp_parse(h, path, "{\"chunk\":{\"tokens\":256}}", &out);                 /* 이 호출만 크기 256 */
sdp_set_option(h, "chunk.tokens", "256");                                   /* 실행 중 기본값 바꾸기 */
```

- 호출 옵션 `chunk`: `true`(켬) · 객체(켬 + 이 호출만 덮어쓰기, `"enabled": false`면 끔) · `false`(끔).
- `tokens`만 바꾸고 `overlap`을 안 주면 `overlap` = max(8, tokens/8)로 따라간다(SimpleRAG 규칙).
- 어긋난 값(`overlap` ≥ `tokens`, `hard_max` < `tokens` 등) · 모르는 키는 `-1`.

| `chunk` 키 | 기본 | 설명 |
|---|---|---|
| `tokenizer` | 없음 | e5 `tokenizer.json` 경로. `unit`이 tokens면 필요 (못 읽으면 첫 청킹 호출이 `-1`) |
| `unit` | tokens | tokens = 토큰 수 / chars = 글자 수(이때 크기 값은 모두 글자 수 뜻) |
| `mode` | structured | structured = 절 단위 / fixed = 고정 크기 |
| `tokens`, `overlap`, `min_tokens` | 128, 16, 16 | 청크 크기 · 긴 줄을 자를 때 겹침 · 이보다 짧으면 버림 |
| `table_header_max_tokens` | 64 | 이보다 긴 표 머리글은 다시 붙이지 않음 |
| `table_whole` | 384 | 표 전체가 이 이하면 행으로 나누지 않음(0 = 끔) |
| `overflow_ratio` | 1.0 | 크기 × 이 값 안이면 단위를 통째로 |
| `sentence_split` | false | 넘치는 줄을 문장 경계로도 나눔 |
| `hard_max` | 510 | 절대 상한(e5 입력 한계) |
| `prefix`, `prefix_max_chars` | true, 80 | 청크 앞 `[문서 > 절]` 접두 · 그 길이 상한 |

**결과** — 청킹을 켠 호출에만 붙는다. `blocks`는 그대로다.

```json
{
  "chunk_version": 1,
  "chunking": {"unit": "tokens", "mode": "structured", "tokens": 128, "...": "..."},
  "chunks": [
    {"no": 0, "tokens": 121, "section_id": 0, "path": ["제1장 총칙"], "source": "parser",
     "page": 1, "page_from": 1, "page_to": 2, "text": "[인사규정 > 제1장 총칙]\n제1조(목적) …"}
  ]
}
```

- `text`는 접두 포함 글이다. 임베딩 · 검색 · LLM에 그대로 넣는다.
- 평문(legacy) · 고정 크기 청크는 `section_id`가 `null`, `path`가 `[]`이고, `source`는 각각 `"legacy"` · `""`이다.
- 청크 0개는 오류가 아니다. OCR이 `deferred`면 아직 OCR 안 한 쪽의 글은 청크에 없다.
- **다시 자르기:** `sdp_chunk(h, doc_json, chunk_opts_json, &out)` — 저장한 `sdp_parse` 결과로 파싱 · OCR 없이 크기만 바꿔 자른다.
- **지원 확인:** `sdp_version()`에 `chunk=`가 있으면 된다(예: `"0.3.0 blocks=7 chunk=1 ocr=…"`). ABI는 1 그대로다. `sdp_get_status`의 `chunk`에 기본값 · 토크나이저가 나온다.
- **재사용 판단:** 청크를 저장하면 `blocks_version`과 `chunk_version`을 함께 적어 두고, 하나라도 바뀌면 다시 자른다.

| 언어 | 청크까지 파싱 | 다시 자르기 |
|---|---|---|
| Python | `p.parse(path, chunk=True)` · `p.parse(path, chunk={"tokens": 256})` | `p.chunk(doc, {"tokens": 64})` |
| Rust | `sdp.parse(path, Some(r#"{"chunk":true}"#))` | `sdp.chunk(&json_text, Some(r#"{"tokens":64}"#))` |
| Java | `sdp.parse(path, "{\"chunk\":true}")` | `sdp.chunk(jsonText, "{\"tokens\":64}")` |
| PHP | `$sdp->parse($path, ['chunk' => true])` | `$sdp->chunk($doc, ['tokens' => 64])` |

```python
opts = {"chunk": {"tokenizer": r"D:\models\e5-small-ko\tokenizer.json"}}
with DocParser(r"lib\struct_docparse.dll", opts) as p:
    doc = p.parse("인사규정.docx", chunk=True)
    for c in doc["chunks"]:
        print(c["no"], c["tokens"], c["page"], c["text"][:40])
```

**CLI:** `sdp parse 문서.docx --chunk --tokenizer <tokenizer.json>` (결과에 청크), `sdp chunk <목록.txt> <폴더> --tokenizer <…> [--chunk-tokens N] [--chars]` → 문서마다 `<이름>.chunks.json`. 토크나이저 경로는 `SDP_TOKENIZER` 환경변수로도 준다.

## 4. 언어별 핵심 사용법

### Python

```python
from struct_docparse import DocParser, DocParseError

with DocParser("lib/libstruct_docparse.so", {"ocr": {"enabled": True}}) as p:   # Windows: r"lib\struct_docparse.dll"
    doc = p.parse("보고서.hwpx")                       # dict
    for b in doc["blocks"]:
        print(b["type"], b["text"][:40])
```

메서드: `version()` · `status()` · `parse(path, title=, keep_pages=, ocr=, chunk=)` · `chunk(doc, opts)` · `extract_text` · `set_option` · `ocr_pages` · `ocr_file` · `pdf_info` · `close()`

### Rust

- 필요한 것: `bindings/rust/struct_docparse.rs`(src/에 복사), `lib/` 폴더 통째, 크레이트 `libloading = "0.8"`, `serde_json = "1"`
- 헤더와 `.dll.lib`는 필요 없다(실행 중에 DLL을 연다).
- 가장 빠른 확인: `cd examples/rust` 후 `cargo run --release -- "문서 경로"`

```rust
let sdp = StructDocParse::open(dll_path, r#"{"ocr":{"enabled":true}}"#)?;
let doc = sdp.parse(path, None)?;                 // serde_json::Value
// blocks가 비어 있으면 raw_text를 본다 (legacy 형식)
```

메서드: `open` · `version` · `parse(path, call_opts)` · `chunk(doc_json, opts)` · `extract_text` · `status` · `pdf_info` · `set_option`. 오류 `SdpError`는 `code`와 `msg`를 가진다. Drop될 때 자동으로 닫힌다. Rust 프로젝트는 `sdp-core` crate를 직접 의존해 DLL 없이 쓸 수도 있다.

**자주 막히는 곳:** 인자에 `parse`를 넣으면 파일로 보고 `-2`가 난다. legacy 형식에서 블록이 0개인 것은 정상이다. lib 안 파일을 흩어 놓으면 pdfium · OCR이 실패한다.

### C / C++

- Windows(MSVC): `cl /utf-8 /I include example.c lib\struct_docparse.dll.lib shell32.lib`. 실행 시 DLL이 보이도록 `lib\`에 두거나 PATH에 넣는다.
- 리눅스(gcc): `gcc -std=c99 -I include example.c -L lib -lstruct_docparse -Wl,-rpath,'$ORIGIN/lib'`
- 링크 없이 `LoadLibraryW`/`dlopen`으로 열어도 된다.

### Java

JNA 5.x(Java 8+)를 쓴다. `bindings/java/structdocparse/`를 소스에 넣는다(Maven: `net.java.dev.jna:jna:5.14.0`).

```java
try (StructDocParse sdp = StructDocParse.load("lib/libstruct_docparse.so", "{}")) {
    Map<String, Object> doc = sdp.parse("보고서.hwpx", null);
}
```

Java 22 이상은 JNA 없이 표준 FFM으로도 붙일 수 있다.

### PHP (미시험)

PHP 7.4+ FFI 확장이 필요하다(`extension=ffi`, FPM에서는 `ffi.enable=true`). `StructDocParse::load(경로, 옵션배열)` → `parse()` → 연관 배열.

### C# · Node.js · Go (미시험)

각각 P/Invoke(`LPUTF8Str` 마샬링), koffi, cgo로 같은 C 함수를 부른다. 원본 9장에 시작용 코드 조각이 있다.

## 5. 리눅스

| 부분 | 필요 glibc | 되는 배포판 예 |
|---|---|---|
| 문서 파싱 (`libstruct_docparse.so` · `libpdfium.so`) | 2.15 이상 | CentOS 7 / RHEL 7 이상, Ubuntu 14.04 이상 |
| OCR (`libonnxruntime.so` 1.22) | 2.27 이상 | RHEL/Rocky/Alma 8 이상, Ubuntu 18.04 이상, Debian 10 이상 |

- **0.3.0부터 문서 파싱 요구 glibc가 2.15 → 2.17**로 오른다(청킹용 토크나이저). CentOS 7 / RHEL 7 이상은 그대로 돈다.
- **CentOS 7에서는 OCR이 안 된다** (`-5`). glibc 2.27 이상 서버나 rockylinux:8 같은 컨테이너를 쓴다. OCR을 안 쓰면 `{"ocr":{"enabled":false}}`로 연다.
- 직접 빌드: `bash scripts/linux-build.sh` (빌드서버에서 빌드 · 시험 · Windows 결과와 비교 · 묶기까지), `bash scripts/linux-examples.sh` (예제 실행)

## 6. 문제 해결

| 증상 | 원인 | 해결 |
|---|---|---|
| 라이브러리를 못 연다 | 경로 오류 또는 32비트 | 절대 경로 사용, x64만 지원 |
| 한글 경로가 모두 `-1` | UTF-8이 아님 | UTF-8로 바꿔 넘긴다 |
| 그림 · 스캔 PDF가 `-5` | onnxruntime 버전 · glibc 2.27 미만 · 모델 없음 | `ocr.reason`과 오류문 확인 |
| 그림이 `-4` | OCR 꺼짐 | 정상 동작, OCR을 켠다 |
| PDF가 `-3` + "pdfium 적재 실패" | pdfium이 옆에 없음 | lib/ 통째로 두거나 `runtime.pdfium` 지정 |
| 메모리가 계속 늘어 보임 | OCR 엔진이 올라가 있음 | 다 쓴 뒤 `sdp_ocr_unload` |
| `-99` | 라이브러리 내부 오류 | 해당 파일을 보내 수정 요청 (다른 파일은 계속 처리됨) |

## 7. C 함수 목록 (`include/struct_docparse.h`)

| 함수 | 하는 일 |
|---|---|
| `sdp_abi_version()` | 함수 모양 버전 (1) |
| `sdp_version()` | 버전 문자열 (해제 안 함) |
| `sdp_open(opts, log_cb, user, &h)` | 옵션 설정 · 핸들 생성 |
| `sdp_close(h)` | 핸들 닫기 (OCR 엔진도 내림) |
| `sdp_set_option(h, key, value)` | 실행 중 옵션 변경 |
| `sdp_get_status(h, &out)` | OCR 상태 · 이유 · 모델 위치 · pdfium 적재 여부 |
| `sdp_supports(h, path)` | 1 블록 / 2 평문 / 0 미지원 (확장자 기준) |
| `sdp_parse(h, path, call_opts, &out)` | 문서 파싱 → 결과 JSON (호출 옵션 `chunk`로 청크도 — 0.3.0~) |
| `sdp_chunk(h, doc_json, chunk_opts, &out)` | 0.3.0~ 저장한 파싱 결과를 다시 청킹 |
| `sdp_extract_text(h, path, fix_tables, &out)` | 블록 없이 평문만 |
| `sdp_ocr_pages(...)` | 쪽 OCR → 캐시 (deferred 방식) |
| `sdp_ocr_file(h, path, pages, &out)` | OCR 결과만 (캐시 안 씀) |
| `sdp_ocr_unload(h)` | OCR 엔진 내리기 |
| `sdp_ocr_cache_remove(h, hash)` | OCR 캐시 한 건 삭제 |
| `sdp_pdf_info(h, path, &out)` | PDF 속성 · 쪽 수 · 책갈피 수 |
| `sdp_pdf_page_texts(h, path, &out)` | PDF 쪽별 글자층 글 |
| `sdp_last_error()` | 이 스레드의 마지막 오류문 (해제 안 함) |
| `sdp_free(p)` | 결과 문자열 해제 |

## 8. 시험 기록 (2026-11-16)

- 리눅스 빌드(CentOS 7.9): Rust 단위 시험 15개 통과, `.so` 5.2MB, 요구 glibc 2.15
- 리눅스와 Windows 결과 비교: 합성 문서 10/10, 실제 업무 문서 13/13 일치
- Windows와 분리 전 SimpleRAG 결과 비교: 실제 문서 84/84 일치
- 예제 실행: Python · Rust · C · Java는 시험함, PHP · C# · Node.js · Go는 돌려 보지 않음
- CentOS 7에서 OCR은 안 됨 (glibc 2.27 필요)

**청킹 시험 (2026-11-27 · 0.3.0)**

- 청킹 단위 시험 11개 통과 (sdp-core 전체 49개 통과)
- SimpleRAG 청크와 비교: 실제 문서 62건 × 설정 4가지(기본 · 크기 256 · 통째 표 끔 · 약한 초과 1.2) 모두 청크 모든 필드가 같음
- 토크나이저 리눅스 빌드(CentOS 7 · gcc 4.8.5): 빌드됨, 토큰 결과 Windows와 같음, `.so` +2.1MB, 요구 glibc 2.17
- DLL을 거쳐(`sdp chunk`) 같은 62건 × 4가지 설정도 SimpleRAG 청크와 모두 같음
- 청킹을 켜지 않은 결과 = 0.2.0 결과 (시험 문서 16/16, `parser` 버전 글만 다름)
- Python 바인딩(`parse(chunk=True)` · `chunk()` · 글자 단위 · 오류 `-1`) · Rust 바인딩 예제 빌드 · Java 바인딩 `parse` · `chunk()` 확인 (Windows). PHP는 돌려 보지 않음
