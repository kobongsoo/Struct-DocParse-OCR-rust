# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# struct_docparse 파이썬 예제 — 문서마다 라벨별 블록 수와 앞 블록 몇 개를 보여 준다
#=> 1) 라이브러리 찾기(SDP_LIB 환경변수 → 묶음의 lib/ → 저장소 target/release)
#    2) 열기(OCR 켬 · inline) → 문서마다 parse
#    3) 같은 그림을 OCR 끔으로 한 번 더 — 실패 코드를 확인
#
#   python examples/python/example.py 문서1 [문서2 ...]
#------------------------------------------------------------------
import collections
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
# 바인딩(bindings/python/struct_docparse.py)을 불러올 수 있게 경로를 더한다
sys.path.insert(0, os.path.join(HERE, "..", "..", "bindings", "python"))
from struct_docparse import DocParser, DocParseError  # noqa: E402

LIB = "struct_docparse.dll" if os.name == "nt" else "libstruct_docparse.so"


#------------------------------------------------------------------
# 라이브러리 경로 찾기
#=> SDP_LIB 환경변수 → 배포 묶음(examples 옆 lib/) → 저장소 빌드 결과(target/release)
#
# -in: 없음
#
# -out: 라이브러리 파일 경로
# -out: error = 못 찾으면 SystemExit
#------------------------------------------------------------------
def find_lib():
    cands = [os.environ.get("SDP_LIB", ""),
             os.path.join(HERE, "..", "..", "lib", LIB),
             os.path.join(HERE, "..", "..", "target", "release", LIB)]
    for c in cands:
        if c and os.path.isfile(c):
            return os.path.abspath(c)
    raise SystemExit("라이브러리를 찾지 못했습니다 — SDP_LIB 에 %s 경로를 주세요" % LIB)


#------------------------------------------------------------------
# 문서 하나 파싱해 요약 출력
#
# -in: p = DocParser / path = 문서 / ocr = None(설정대로) · False(이번만 끔)
#
# -out: 없음
# -out: error = 파싱 실패는 잡아서 한 줄로 출력
#------------------------------------------------------------------
def show(p, path, ocr=None):
    try:
        d = p.parse(path, ocr=ocr)
    except DocParseError as e:
        print("  %s : 실패(%d) %s" % (os.path.basename(path), e.code, e.msg))
        return
    kinds = collections.Counter(b["type"] for b in d["blocks"])
    print("  %s : %s · 블록 %d · %s" % (os.path.basename(path), d["format"], len(d["blocks"]), dict(kinds)))
    if d["raw_text"]:
        # 라벨 없이 평문만 나오는 형식(hwp · ppt · xls · txt …)
        print("      평문 %d자: %s" % (len(d["raw_text"]), d["raw_text"][:40].replace("\n", " ")))
    for b in d["blocks"][:3]:
        print("      [%s%s] %s" % (b["type"], ":" + str(b["level"]) if "level" in b else "", b["text"][:50].replace("\n", " ")))


def main():
    files = sys.argv[1:]
    if not files:
        raise SystemExit("사용: python example.py 문서1 [문서2 ...]")
    lib = find_lib()
    # 로그(경고)는 표준 오류로
    with DocParser(lib, {"ocr": {"enabled": True, "mode": "inline"}},
                   log=lambda lv, m: print("[sdp] " + m, file=sys.stderr)) as p:
        st = p.status()
        print("struct_docparse %s · OCR %s · pdfium %s" % (p.version(), st["ocr"]["enabled"], st["pdfium"]["ok"]))
        for f in files:
            show(p, f)
        # 그림 파일이 있으면 이번 호출만 OCR 을 꺼서 다시 — 그림은 -4(지원 안 함)가 정상
        imgs = [f for f in files if f.lower().endswith((".png", ".jpg", ".jpeg", ".tif", ".tiff", ".bmp"))]
        if imgs:
            print("OCR 끔으로 다시:")
            show(p, imgs[0], ocr=False)


if __name__ == "__main__":
    main()
