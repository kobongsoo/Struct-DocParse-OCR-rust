# third_party — 실행에 필요한 바깥 라이브러리

저장소만 받아도 빌드 · 시험 · 묶기가 되도록, 라이브러리가 실행 중에 여는 바깥 라이브러리를 여기에 둔다.
**받은 그대로**이며 고치지 않았다. 묶음을 만들 때(`scripts/package.py`) 플랫폼에 맞는 것이 `lib/` 로 들어간다.

| 파일 | 하는 일 | 판 · 출처 | 라이선스 |
|---|---|---|---|
| `pdfium/windows-x64/pdfium.dll` | PDF 읽기 · 쪽 그리기 | pdfium-binaries chromium/7999 — Python `pypdfium2` 5.13.0 휠에 든 것(SimpleRAG 배포본과 같은 파일) | BSD-3-Clause · Apache-2.0 (`pdfium/LICENSES/`) |
| `pdfium/linux-x64/libpdfium.so` | 〃 (리눅스) | pdfium-binaries 리눅스 빌드 — DocParse-rust 저장소의 `third_party/pdfium` 과 같은 파일 | 〃 |
| `onnxruntime/windows-x64/onnxruntime.dll` · `onnxruntime_providers_shared.dll` | OCR 모델 계산 | ONNX Runtime **1.29.0** — Python `onnxruntime` 1.29.0 휠에 든 것(SimpleRAG 배포본과 같은 파일) | MIT (`onnxruntime/windows-x64/LICENSE` · `ThirdPartyNotices.txt`) |
| `onnxruntime/linux-x64/libonnxruntime.so` · `libonnxruntime_providers_shared.so` | 〃 (리눅스) | ONNX Runtime **1.22.0** — [공식 릴리스](https://github.com/microsoft/onnxruntime/releases/tag/v1.22.0) `onnxruntime-linux-x64-1.22.0.tgz` 의 `lib/libonnxruntime.so.1.22.0` 을 이 이름으로 | MIT (`onnxruntime/linux-x64/LICENSE` · `ThirdPartyNotices.txt`) |
| `java/jna-5.14.0.jar` | 자바 예제 · 바인딩(JNA) | Maven Central `net.java.dev.jna:jna:5.14.0` (SHA-1 `67bf3eaea4f0718cb376a181a629e5f88fa1c9dd` 대조) | Apache-2.0 / LGPL-2.1 |

- onnxruntime 은 **1.22 이상**이면 된다(ort 2.0.0-rc.10 은 1.22 미만만 거절하고 더 새것은 경고만 낸다).
  리눅스 1.22.0 은 glibc 2.27 이상이 필요하다 — CentOS 7 에서는 OCR 이 안 된다.
- Windows 에서는 Visual C++ 런타임(`vcruntime140.dll` 등)이 있어야 한다 — 대부분의 PC 에 이미 있고, 없으면
  [Microsoft Visual C++ 재배포 패키지](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) 를 설치한다.

SHA-256

```
fb898a1f5ace57805834f390407500bdb6ef93eff326a252ad334a8aae809d8e  pdfium/windows-x64/pdfium.dll
687dce861f959c7097d47c5864509d51a926a71b38322596a8ee3e7a99c6b96e  pdfium/linux-x64/libpdfium.so
f9013e824c1dbc5b1874ffdabeedfd1cad4e2dabd9d93528f6ec6d21c2980d00  onnxruntime/windows-x64/onnxruntime.dll
e1d2b0678ad4eafedf6472b544d7d632c365ca48673269a174c817597a0e04a4  onnxruntime/windows-x64/onnxruntime_providers_shared.dll
3da6146e14e7b8aaec625dde11d6114c7457c87a5f93d744897da8781e35c673  onnxruntime/linux-x64/libonnxruntime.so
e4706ea02be3999167f000bb1ff7391c86e95d2d96f78eb33225c14b172a8451  onnxruntime/linux-x64/libonnxruntime_providers_shared.so
```
