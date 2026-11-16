# OCR 비교(우리 sdp.exe vs Newploy) 스크립트가 함께 쓰는 경로·설정
import os
from pathlib import Path

# 저장소 루트 (tools/ocr_compare/ 의 두 단계 위)
REPO = Path(__file__).resolve().parents[2]
# 실제 문서가 들어가므로 git 에 올리지 않는 tests/private 아래에 둔다
DATA = REPO / "tests" / "private" / "ocr_compare_newploy"
SET_DIR = DATA / "set"          # 그림만 든 '스캔' PDF + truth.json
OURS_DIR = DATA / "ours"        # sdp.exe parse 결과
NP_DIR = DATA / "newploy"       # Newploy document_result.json 복사본
SDP_EXE = REPO / "target" / "release" / "sdp.exe"
# Newploy 앱이 결과를 쌓는 곳
NP_WORK = Path(os.path.expandvars(r"%LOCALAPPDATA%\Newploy\AI Desktop\work"))
# Newploy 를 --remote-debugging-port=9333 으로 띄웠을 때의 주소
CDP_URL = "http://127.0.0.1:9333"
