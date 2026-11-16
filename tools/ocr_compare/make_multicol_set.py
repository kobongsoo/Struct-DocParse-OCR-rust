"""실제 다단(2단 이상) 텍스트층 PDF 쪽을 골라 '스캔본' 시험셋 만들기 — OCR 구조 분석 다단 검증용.

사용: python tools/ocr_compare/make_multicol_set.py [원본 폴더=D:\\분류함] [문서 수=10]
결과: tests/private/ocr_multicol/set/ (mcNN.pdf + truth.json — make_set.py 와 같은 모양)
      → eval_layout.py --set tests/private/ocr_multicol/set 으로 채점
"""
import glob, json, random, re, sys
from pathlib import Path

import pymupdf

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "tests" / "private" / "ocr_multicol" / "set"
# 쪽 하나의 다단 판정 문턱
MIN_SIDE_LINES = 8      # 왼쪽 · 오른쪽 단 각각 최소 줄 수
MAX_CROSS_SHARE = 0.12  # 가운데를 가로지르는 줄 비율 상한(제목 · 쪽 폭 문단) — 1단 문서의 긴 줄을 거른다
MIN_FILL = 0.6          # 단 안 줄 너비 중앙값 ≥ 단 너비 × 이 값(글줄이 단을 채움)
MIN_ALIGN = 0.5         # 오른쪽 단 줄의 이 비율 이상이 같은 왼쪽 끝(±5pt)에서 시작
MIN_V_OVERLAP = 0.5     # 두 단 세로 범위 겹침(짧은 쪽 기준)
MAX_SCAN_PAGES = 200    # 문서마다 앞에서 볼 최대 쪽 수
# 다단 문서가 드물어(2026-11-17 D:\분류함 937개 중 7개) 문서당 1쪽부터 쓴다
MIN_DOC_PAGES = 1
MAX_DOC_PAGES = 5


#------------------------------------------------------------------
# 이 쪽이 다단인가
#=> 텍스트층 줄 위치로 본다.
#    1) 한글이 충분하고 깨진 글자가 없는 쪽만
#    2) 쪽 가운데(글 범위 기준)를 넘지 않는 왼쪽 줄 · 오른쪽 줄이 각각 MIN_SIDE_LINES 이상
#    3) 가운데를 가로지르는 줄이 전체의 MAX_CROSS_SHARE 이하
#    4) 왼쪽 · 오른쪽 줄의 세로 범위가 MIN_V_OVERLAP 이상 겹침(나란한 단)
#    5) 큰 그림이 쪽의 20% 이상이면 뺀다(그림 속 글은 정답에 없다)
#    6) 양쪽 줄 너비 중앙값이 단 너비의 MIN_FILL 이상, 오른쪽 단 줄의 MIN_ALIGN 이상이 같은 x 에서 시작
#
# -in: page = pymupdf 쪽
#
# -out: True = 다단 쪽
# -out: error = 없음
#------------------------------------------------------------------
def is_multicol(page):
    t = page.get_text()
    s = re.sub(r"\s", "", t)
    han = len(re.findall(r"[가-힣]", s))
    if not (300 <= len(s) <= 4000 and han / max(1, len(s)) > 0.5 and "\ufffd" not in t):
        return False
    area = sum(r.width * r.height for im in page.get_images() for r in page.get_image_rects(im[0]))
    if area / (page.rect.width * page.rect.height) >= 0.2:
        return False
    lines = []
    for b in page.get_text("dict")["blocks"]:
        for l in b.get("lines", []):
            if "".join(sp["text"] for sp in l["spans"]).strip():
                lines.append(l["bbox"])
    if len(lines) < 2 * MIN_SIDE_LINES:
        return False
    x0 = min(b[0] for b in lines)
    x1 = max(b[2] for b in lines)
    mid = (x0 + x1) / 2
    left = [b for b in lines if b[2] < mid]
    right = [b for b in lines if b[0] > mid]
    cross = len(lines) - len(left) - len(right)
    if len(left) < MIN_SIDE_LINES or len(right) < MIN_SIDE_LINES or cross > len(lines) * MAX_CROSS_SHARE:
        return False
    ly = (min(b[1] for b in left), max(b[3] for b in left))
    ry = (min(b[1] for b in right), max(b[3] for b in right))
    ov = min(ly[1], ry[1]) - max(ly[0], ry[0])
    if ov < MIN_V_OVERLAP * min(ly[1] - ly[0], ry[1] - ry[0]):
        return False
    # 1단 문서의 짧은 줄 · 오른쪽 도식 글과 가르기 — 양쪽 줄이 단을 채우고, 오른쪽 단 줄이 한 줄로 맞춰 시작
    def fill(side, a, b):
        ws = sorted(x[2] - x[0] for x in side)
        return ws[len(ws) // 2] >= MIN_FILL * (b - a)
    if not (fill(left, x0, mid) and fill(right, mid, x1)):
        return False
    rx0 = sorted(b[0] for b in right)
    best = max(sum(1 for v in rx0 if abs(v - c) <= 5) for c in rx0)
    return best >= MIN_ALIGN * len(right)


#------------------------------------------------------------------
# 진입점 — 다단 쪽이 있는 문서를 골라 스캔본으로
#   (다단 쪽이 MIN_DOC_PAGES 이상인 문서, 문서당 최대 MAX_DOC_PAGES 쪽)
#=> make_set.py 와 같은 방법(200dpi JPEG 품질 90, 같은 쪽 크기)으로 그림만 든 PDF 를 만들고
#   truth.json 에 원본 경로 · 쪽 번호를 적는다. 고정 시드라 다시 돌려도 같은 셋.
#
# -in: src_dir = 원본 PDF 폴더 / n_docs = 만들 문서 수
#
# -out: 없음 (OUT 에 mcNN.pdf · truth.json)
# -out: error = 못 여는 PDF 는 건너뜀, 쓰기 실패는 예외 전파
#------------------------------------------------------------------
def main(src_dir=r"D:\분류함", n_docs=10):
    # 다시 만들 때 옛 결과가 섞이지 않게 비운다
    for old in OUT.glob("mc*.pdf"):
        old.unlink()
    random.seed(20261117)
    files = sorted(glob.glob(src_dir + r"\**\*.pdf", recursive=True))
    random.shuffle(files)
    OUT.mkdir(parents=True, exist_ok=True)
    truth = {}
    seen = set()
    for f in files:
        # 같은 파일이 여러 폴더에 복사돼 있다 — 파일 이름으로 한 번만
        base = Path(f).name
        if base in seen:
            continue
        if len(truth) >= n_docs:
            break
        try:
            d = pymupdf.open(f)
            good = [i for i in range(min(len(d), MAX_SCAN_PAGES)) if is_multicol(d[i])]
        except Exception:
            continue
        if len(good) < MIN_DOC_PAGES:
            continue
        seen.add(base)
        pick = sorted(random.sample(good, min(MAX_DOC_PAGES, len(good))))
        name = f"mc{len(truth) + 1:02d}"
        nd = pymupdf.open()
        for i in pick:
            p = d[i]
            jpg = p.get_pixmap(dpi=200).tobytes("jpeg", jpg_quality=90)
            np_ = nd.new_page(width=p.rect.width, height=p.rect.height)
            np_.insert_image(np_.rect, stream=jpg)
        nd.save(str(OUT / f"{name}.pdf"))
        truth[name] = {"src": f, "pages": pick, "text": [d[i].get_text() for i in pick]}
        print(name, len(pick), f, flush=True)
    json.dump(truth, open(OUT / "truth.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print("pages", sum(len(v["pages"]) for v in truth.values()))


if __name__ == "__main__":
    main(*(sys.argv[1:2] or [r"D:\분류함"]), *[int(x) for x in sys.argv[2:3]])
