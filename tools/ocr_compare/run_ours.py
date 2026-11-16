"""시험셋 PDF 를 우리 OCR(sdp.exe parse)로 돌려 ours/ 에 JSON 저장.

사용: python tools/ocr_compare/run_ours.py
"""
import subprocess, time

from common import OURS_DIR, SDP_EXE, SET_DIR


#------------------------------------------------------------------
# 우리 OCR 로 시험셋 전부 파싱
#=> scanNN.pdf 마다 sdp.exe parse 를 한 번씩 실행하고 걸린 시간을 찍는다.
#    1) ours/ 폴더를 만든다
#    2) PDF 하나씩 sdp.exe parse <pdf> --out ours/<이름>.json 실행
#    3) 문서별 누적 시간을 출력 (exe 시작·모델 올리기 시간 포함)
#
# -in: 없음
#
# -out: 없음 (ours/*.json 생성)
# -out: error = sdp.exe 가 0 이 아닌 코드로 끝나면 CalledProcessError
#------------------------------------------------------------------
def main():
    OURS_DIR.mkdir(parents=True, exist_ok=True)
    t0 = time.time()
    for pdf in sorted(SET_DIR.glob("*.pdf")):
        subprocess.run([str(SDP_EXE), "parse", str(pdf), "--out", str(OURS_DIR / f"{pdf.stem}.json")],
                       check=True, stdout=subprocess.DEVNULL)
        print(f"{pdf.stem} {time.time() - t0:.0f}s")


if __name__ == "__main__":
    main()
