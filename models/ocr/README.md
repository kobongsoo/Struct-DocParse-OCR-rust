# OCR 모델

스캔 PDF · 그림 파일 글자 읽기에 쓰는 모델 세 파일이다. 라이브러리는 기본으로 **자기 옆 `models/ocr/`** 에서 찾는다
(다른 곳에 두면 `sdp_open` 옵션 `ocr.model_dir` 로 알려 준다).

| 파일 | 하는 일 | 원본 |
|---|---|---|
| `PP-OCRv6_tiny_det.onnx` | 글자 상자 찾기(검출) | [PaddlePaddle/PP-OCRv6_tiny_det_onnx](https://huggingface.co/PaddlePaddle/PP-OCRv6_tiny_det_onnx) |
| `korean_PP-OCRv5_mobile_rec.onnx` | 상자 안 글자 읽기(한국어 인식) | [PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx](https://huggingface.co/PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx) |
| `korean_PP-OCRv5_mobile_rec.dict.txt` | 인식 모델의 글자 사전(한글 11,172자 포함) | 같은 저장소 |

- 개발: PaddlePaddle (Baidu) · 모음: https://huggingface.co/collections/PaddlePaddle/pp-ocrv6
- 라이선스: **Apache-2.0** — 전문은 같은 폴더의 `LICENSE`. 파일은 받은 그대로이며 바꾸지 않았다.
- 엔진 이름 `ppocrv6-tiny-det+ko-ppocrv5-mobile-rec/1` 이 이 조합을 가리킨다. 모델을 바꾸면 엔진 이름을 올려 옛 OCR 캐시를 버리게 한다
  (`crates/sdp-core/src/ocr/mod.rs` 의 `ENGINE`).
- PP-OCRv6 인식 모델은 글자 사전에 한글이 없어(2026-11-13 실측) 인식만 v5 한국어 모델을 쓴다.

SHA-256

```
193bab7a04fca699a6c82e6abb5b81bdb28177f0abd4062552b04908dafb19f8  PP-OCRv6_tiny_det.onnx
92f0b7785e64fc9090106a241cf4c1eb97472824558272751b88a2a4476d3a08  korean_PP-OCRv5_mobile_rec.onnx
a88071c68c01707489baa79ebe0405b7beb5cca229f4fc94cc3ef992328802d7  korean_PP-OCRv5_mobile_rec.dict.txt
```
