"""텍스트층 PDF → 그림만 든 '스캔' PDF 시험셋 + 정답(truth.json) 만들기.

사용: python tools/ocr_compare/make_set.py [원본 폴더=D:\\분류함] [문서 수=8]
"""
import glob, json, random, re, sys

import pymupdf

from common import SET_DIR


#------------------------------------------------------------------
# 시험에 쓸 만한 쪽인지 판단
#=> 정답(텍스트층)이 믿을 만하고 OCR 로 읽을 글이 충분한 쪽만 고른다.
#    1) 공백 뺀 글자 수가 150~2,500자인지 본다 (너무 적거나 많은 쪽 제외)
#    2) 한글 비율이 50% 넘는지 본다 (한국어 OCR 비교가 목적)
#    3) 깨진 글자(U+FFFD)가 없는지 본다 (정답이 깨지면 채점이 틀어짐)
#    4) 그림 면적이 쪽의 20% 미만인지 본다 (그림 속 글자는 정답에 없으므로)
#
# -in: page = pymupdf 쪽 객체
#
# -out: ok = True 면 시험에 쓸 쪽
# -out: error = 없음 (그림 위치를 못 구하는 경우 예외 전파)
#------------------------------------------------------------------
def page_ok(page):
    t = page.get_text()
    s = re.sub(r"\s", "", t)
    han = len(re.findall(r"[가-힣]", s))
    if not (150 <= len(s) <= 2500 and han / max(1, len(s)) > 0.5 and "\ufffd" not in t):
        return False
    # 쪽을 덮는 큰 그림이 있으면 그 안의 글자는 정답에 빠져 있어 공정하지 않다
    area = sum(r.width * r.height for im in page.get_images() for r in page.get_image_rects(im[0]))
    return area / (page.rect.width * page.rect.height) < 0.2


#------------------------------------------------------------------
# 시험셋 만들기 (본체)
#=> 원본 폴더에서 PDF 를 고정 시드로 섞어 고르고, 문서마다 최대 5쪽을 뽑아
#   200dpi JPEG(품질 90) 그림만 든 PDF 로 다시 만든다. 원래 텍스트층은 정답으로 저장한다.
#    1) 원본 PDF 목록을 시드 20261116 으로 섞는다 (다시 돌려도 같은 셋)
#    2) page_ok 를 통과한 쪽이 3쪽 이상인 문서만 쓴다
#    3) 뽑은 쪽을 그림으로 렌더해 새 PDF 의 쪽 크기 그대로 붙인다
#    4) truth.json 에 원본 경로·쪽 번호·정답 글자를 적는다
#
# -in: src_dir = 원본 PDF 폴더 (기본 D:\분류함)
# -in: n_docs  = 만들 문서 수 (기본 8)
#
# -out: 없음 (SET_DIR 에 scanNN.pdf · truth.json 생성)
# -out: error = 못 여는 PDF 는 건너뜀, 쓰기 실패는 예외 전파
#------------------------------------------------------------------
def main(src_dir=r"D:\분류함", n_docs=8):
    random.seed(20261116)
    files = sorted(glob.glob(src_dir + r"\**\*.pdf", recursive=True))
    random.shuffle(files)
    SET_DIR.mkdir(parents=True, exist_ok=True)
    truth = {}
    for f in files:
        if len(truth) >= n_docs:
            break
        try:
            d = pymupdf.open(f)
        except Exception:
            continue
        good = [i for i, p in enumerate(d) if page_ok(p)]
        if len(good) < 3:
            continue
        pick = sorted(random.sample(good, min(5, len(good))))
        name = f"scan{len(truth) + 1:02d}"
        nd = pymupdf.open()
        for i in pick:
            p = d[i]
            # 실제 스캔본처럼 JPEG 손실 압축을 거친 그림으로 만든다
            jpg = p.get_pixmap(dpi=200).tobytes("jpeg", jpg_quality=90)
            np_ = nd.new_page(width=p.rect.width, height=p.rect.height)
            np_.insert_image(np_.rect, stream=jpg)
        nd.save(str(SET_DIR / f"{name}.pdf"))
        truth[name] = {"src": f, "pages": pick, "text": [d[i].get_text() for i in pick]}
        print(name, len(pick), f)
    json.dump(truth, open(SET_DIR / "truth.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print("pages", sum(len(v["pages"]) for v in truth.values()))


if __name__ == "__main__":
    main(*(sys.argv[1:2] or [r"D:\분류함"]), *[int(x) for x in sys.argv[2:3]])
