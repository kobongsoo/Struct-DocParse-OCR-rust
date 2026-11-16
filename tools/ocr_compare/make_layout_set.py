"""OCR 구조 분석 시험용 합성 문서 — 2단 · 머리말 · 쪽번호 · 제목 · 목록이 든 텍스트층 PDF 와 그 '스캔본'.

사용: python tools/ocr_compare/make_layout_set.py [글꼴=C:\\Windows\\Fonts\\malgun.ttf]
결과: tests/private/ocr_layout/
        src_twocol.pdf · src_onecol.pdf  (정답용 텍스트층 PDF)
        set/scan_twocol.pdf · set/scan_onecol.pdf  (200dpi JPEG 그림만 든 PDF)
        set/truth.json  (make_set.py 와 같은 모양 — eval_layout.py --set 으로 채점)
글은 모두 지어낸 것이다.
"""
import json, sys
from pathlib import Path

import pymupdf

REPO = Path(__file__).resolve().parents[2]
OUT = REPO / "tests" / "private" / "ocr_layout"
FONT = sys.argv[1] if len(sys.argv) > 1 else r"C:\Windows\Fonts\malgun.ttf"
HEADER = "예시연구원 · 분기 산업 동향 보고서"

# 쪽마다 (큰 제목, [(소제목, [문단 또는 "- 목록"])])
PAGES = [
    ("1. 시장 동향 개요", [
        ("1.1 국내 시장", [
            "올해 국내 시장은 상반기 동안 완만한 회복세를 보였다. 소비 심리가 점차 나아지면서 유통과 서비스 부문의 매출이 함께 늘었고, 제조 부문도 수출 회복에 힘입어 가동률이 높아졌다.",
            "다만 지역별 차이는 여전히 크다. 수도권은 투자와 고용이 함께 늘었지만 일부 지방은 인구 감소와 산업 구조 변화로 회복 속도가 더디다.",
        ]),
        ("1.2 해외 시장", [
            "해외 시장은 주요 교역국의 금리 인하로 수요가 살아나는 흐름이다. 특히 동남아시아 지역의 전자 부품 수요가 크게 늘어 관련 기업의 수주가 이어지고 있다.",
            "- 동남아시아 전자 부품 수요 증가",
            "- 유럽 친환경 설비 투자 확대",
            "- 북미 소비재 재고 조정 마무리",
        ]),
    ]),
    ("2. 부문별 분석", [
        ("2.1 제조 부문", [
            "제조 부문은 반도체와 이차전지가 성장을 이끌었다. 설비 투자가 이어지면서 장비와 소재 기업의 실적도 함께 좋아졌으며 하반기에도 이 흐름이 이어질 것으로 본다.",
            "그러나 원자재 가격이 다시 오르고 있어 수익성에는 부담이 될 수 있다. 기업들은 공급처를 넓히고 장기 계약을 늘리는 방식으로 위험을 줄이고 있다.",
        ]),
        ("2.2 서비스 부문", [
            "서비스 부문은 여행과 공연 수요가 회복되면서 고용이 크게 늘었다. 온라인 플랫폼을 통한 예약이 늘어 중소 사업자의 판로도 넓어졌다.",
            "금융 서비스는 비대면 거래 비중이 계속 높아지고 있다. 이에 따라 보안과 개인정보 보호에 대한 투자가 늘고 관련 규제도 정비되고 있다.",
        ]),
    ]),
    ("3. 향후 전망", [
        ("3.1 기회 요인", [
            "새로운 기술의 확산은 생산성을 높일 기회가 된다. 자동화와 데이터 분석을 도입한 기업은 비용을 줄이고 고객 응대 속도를 높이고 있다.",
            "- 공정 자동화로 불량률 감소",
            "- 데이터 기반 수요 예측 정확도 향상",
        ]),
        ("3.2 위험 요인", [
            "대외 불확실성은 여전히 가장 큰 위험이다. 환율 변동과 교역 규제 강화가 겹치면 수출 기업의 계획이 흔들릴 수 있다.",
            "인력 수급도 과제로 남아 있다. 숙련 인력이 부족한 분야는 교육 과정을 늘리고 산학 협력을 강화하는 방안이 논의되고 있다.",
        ]),
    ]),
]


#------------------------------------------------------------------
# 텍스트층 PDF 한 벌 만들기 (1단 또는 2단)
#=> 쪽마다 머리말 · 큰 제목(쪽 폭) · 본문 · 쪽번호를 넣는다. 2단이면 소제목 묶음 둘을
#   왼쪽 · 오른쪽 단에 하나씩 넣는다. 글자층 순서가 곧 정답 읽는 순서가 된다
#   (머리말 → 제목 → 왼쪽 단 → 오른쪽 단 → 쪽번호).
#
# -in: path = 만들 파일 / two_col = True 면 2단
#
# -out: 없음 (파일 생성)
# -out: error = 글꼴 파일이 없으면 pymupdf 예외 전파
#------------------------------------------------------------------
def make_pdf(path, two_col):
    doc = pymupdf.open()
    for no, (title, sections) in enumerate(PAGES, start=1):
        pg = doc.new_page(width=595, height=842)
        # 글자층에 한글을 넣으려고 글꼴 파일을 쪽에 싣는다
        put = lambda rect, t, size: pg.insert_textbox(rect, t, fontsize=size, fontname="kr", fontfile=FONT, lineheight=1.45)
        put(pymupdf.Rect(50, 30, 545, 50), HEADER, 8)
        put(pymupdf.Rect(50, 70, 545, 100), title, 17)
        cols = [(50, 285), (310, 545)] if two_col else [(50, 545)]
        y = {c: 115.0 for c in range(len(cols))}
        for si, (sub, paras) in enumerate(sections):
            c = si % len(cols)
            x0, x1 = cols[c]
            # 소제목(본문 10pt 보다 큰 13pt)
            put(pymupdf.Rect(x0, y[c], x1, y[c] + 22), sub, 13)
            y[c] += 26
            for ptxt in paras:
                is_item = ptxt.startswith("- ")
                ix = x0 + (10 if is_item else 0)
                # 남은 높이가 음수면 다 들어간 것 — 실제 쓴 높이만큼 내려간다
                rect = pymupdf.Rect(ix, y[c], x1, y[c] + 300)
                left = put(rect, ptxt, 10)
                used = 300 - left
                y[c] += used + (2 if is_item else 9)
            y[c] += 10
        put(pymupdf.Rect(270, 800, 330, 815), "- %d -" % (no + 10), 9)
    doc.save(str(path))


#------------------------------------------------------------------
# 텍스트층 PDF → 그림만 든 '스캔' PDF (make_set.py 와 같은 방법)
#
# -in: src = 텍스트층 PDF / dst = 만들 스캔 PDF
#
# -out: 없음
# -out: error = 예외 전파
#------------------------------------------------------------------
def to_scan(src, dst):
    d = pymupdf.open(str(src))
    nd = pymupdf.open()
    for p in d:
        jpg = p.get_pixmap(dpi=200).tobytes("jpeg", jpg_quality=90)
        np_ = nd.new_page(width=p.rect.width, height=p.rect.height)
        np_.insert_image(np_.rect, stream=jpg)
    nd.save(str(dst))


#------------------------------------------------------------------
# 진입점 — 1단 · 2단 한 벌씩 만들고 truth.json 을 쓴다
#
# -in: 없음 (sys.argv[1] = 글꼴 경로, 선택)
#
# -out: 없음
# -out: error = 예외 전파
#------------------------------------------------------------------
def main():
    (OUT / "set").mkdir(parents=True, exist_ok=True)
    truth = {}
    for name, two in (("twocol", True), ("onecol", False)):
        src = OUT / f"src_{name}.pdf"
        make_pdf(src, two)
        to_scan(src, OUT / "set" / f"scan_{name}.pdf")
        d = pymupdf.open(str(src))
        truth[f"scan_{name}"] = {"src": str(src), "pages": list(range(len(d))), "text": [p.get_text() for p in d]}
    json.dump(truth, open(OUT / "set" / "truth.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print("만듦:", OUT)


if __name__ == "__main__":
    main()
