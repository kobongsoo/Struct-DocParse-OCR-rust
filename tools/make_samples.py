# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 시험용 합성 문서 만들기 — tests/samples/
#=> 공개 저장소에 실제 문서를 넣지 않으려고, 형식별 시험 문서를 내용까지 지어내 만든다.
#   docx · xlsx · pptx · md · txt · 글자층 PDF · 스캔 그림(png · jpg) · 스캔 PDF · 작은 아이콘
#   hwp · hwpx · doc 는 만들 도구가 없어 빠진다 — 그 형식은 tests/private/ (git 에 안 올라감)에 직접 넣어 시험한다.
#
#   필요: python-docx · openpyxl · python-pptx · PyMuPDF · Pillow, 한글 글꼴(맑은 고딕 등)
#   사용: python tools/make_samples.py [글꼴 경로]
#------------------------------------------------------------------
import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests", "samples")
FONT = sys.argv[1] if len(sys.argv) > 1 else r"C:\Windows\Fonts\malgun.ttf"

TITLE = "문서 파서 시험용 보고서"
SECTIONS = [
    ("1. 개요", ["이 문서는 구조화 문서 파서를 시험하려고 지어낸 예시 문서다.",
                "제목, 문단, 목록, 표, 머리글과 꼬리글이 모두 들어 있다."]),
    ("2. 처리 절차", ["문서를 읽으면 블록 단위로 나누고 블록마다 라벨을 붙인다.",
                    "스캔된 쪽은 OCR 로 글자를 읽어 본문 문단으로 넣는다."]),
    ("3. 결과", ["결과는 JSON 으로 돌려주며 블록 순서는 문서 읽는 순서와 같다."]),
]
TABLE = [["항목", "값", "비고"], ["형식", "docx", "시험"], ["블록 수", "12", "예시"], ["작성일", "2026-11-16", "지어냄"]]
ITEMS = ["첫째 목록 항목", "둘째 목록 항목", "셋째 목록 항목"]


#------------------------------------------------------------------
# docx — 제목 수준 · 문단 · 목록 · 표 · 머리글 · 꼬리글
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = python-docx 가 없으면 ImportError
#------------------------------------------------------------------
def make_docx(path):
    import docx
    d = docx.Document()
    sec = d.sections[0]
    sec.header.paragraphs[0].text = "예시 기관 · 내부 시험 문서"
    sec.footer.paragraphs[0].text = "지어낸 문서 — 실제 내용 아님"
    d.add_heading(TITLE, level=1)
    for head, paras in SECTIONS:
        d.add_heading(head, level=2)
        for p in paras:
            d.add_paragraph(p)
        if head.startswith("2."):
            for it in ITEMS:
                d.add_paragraph(it, style="List Bullet")
            t = d.add_table(rows=len(TABLE), cols=3)
            t.style = "Table Grid"
            for r, row in enumerate(TABLE):
                for c, v in enumerate(row):
                    t.cell(r, c).text = v
            d.add_paragraph("표 1. 처리 결과 요약", style="Caption")
    d.save(path)


#------------------------------------------------------------------
# xlsx — 시트 두 개(표 · 날짜 칸)
#------------------------------------------------------------------
def make_xlsx(path):
    import datetime
    import openpyxl
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "점검표"
    for row in [["점검 항목", "상태", "점검일"], ["출입문 잠금", "양호", datetime.date(2026, 11, 16)],
                ["소화기 위치", "양호", datetime.date(2026, 11, 16)], ["비상구 표시", "보완 필요", datetime.date(2026, 11, 17)]]:
        ws.append(row)
    ws2 = wb.create_sheet("요약")
    ws2.append(["구분", "건수"])
    ws2.append(["양호", 2])
    ws2.append(["보완 필요", 1])
    wb.save(path)


#------------------------------------------------------------------
# pptx — 제목 슬라이드 · 목록 슬라이드 · 표 슬라이드 · 발표자 노트
#------------------------------------------------------------------
def make_pptx(path):
    from pptx import Presentation
    from pptx.util import Inches
    p = Presentation()
    s = p.slides.add_slide(p.slide_layouts[0])
    s.shapes.title.text = TITLE
    s.placeholders[1].text = "지어낸 발표 자료"
    s = p.slides.add_slide(p.slide_layouts[1])
    s.shapes.title.text = "처리 절차"
    body = s.placeholders[1].text_frame
    body.text = ITEMS[0]
    for it in ITEMS[1:]:
        body.add_paragraph().text = it
    s.notes_slide.notes_text_frame.text = "발표자 노트 — 시험용 메모"
    s = p.slides.add_slide(p.slide_layouts[5])
    s.shapes.title.text = "결과 표"
    tb = s.shapes.add_table(len(TABLE), 3, Inches(1), Inches(2), Inches(8), Inches(2)).table
    for r, row in enumerate(TABLE):
        for c, v in enumerate(row):
            tb.cell(r, c).text = v
    p.save(path)


#------------------------------------------------------------------
# 글자층 있는 PDF — 3쪽, 쪽마다 머리글 · 쪽 번호, 제목 · 문단 · 선으로 그린 표 (PyMuPDF)
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = PyMuPDF 가 없으면 ImportError
#------------------------------------------------------------------
def make_text_pdf(path):
    import fitz
    doc = fitz.open()
    for no, (head, paras) in enumerate(SECTIONS, start=1):
        pg = doc.new_page(width=595, height=842)
        # 글자층에 한글을 넣으려고 글꼴 파일을 쪽에 싣는다
        put = lambda xy, t, size: pg.insert_text(xy, t, fontsize=size, fontname="kr", fontfile=FONT)
        put((50, 40), "예시 기관 · 내부 시험 문서", 9)
        put((285, 815), "- %d -" % no, 9)
        put((50, 90), head, 16)
        y = 125
        for ptxt in paras:
            put((50, y), ptxt, 11)
            y += 22
        if no == 2:
            # 테두리 표 — 선으로 칸을 그린다(표 검출 시험)
            y += 10
            cw, rh = 150, 22
            for r, row in enumerate(TABLE):
                for col, v in enumerate(row):
                    x0, y0 = 50 + col * cw, y + r * rh
                    pg.draw_rect(fitz.Rect(x0, y0, x0 + cw, y0 + rh), color=(0, 0, 0), width=0.8)
                    put((x0 + 6, y0 + 15), v, 10)
    doc.save(path)


#------------------------------------------------------------------
# 스캔처럼 보이는 그림 한 장 — 글자를 그림으로 그리고 살짝 흐리게
#
# -in: lines = 줄 목록 / size = (가로, 세로)
#
# -out: PIL 그림
#------------------------------------------------------------------
def scan_image(lines, size=(1240, 1754)):
    img = Image.new("RGB", size, "white")
    d = ImageDraw.Draw(img)
    big, small = ImageFont.truetype(FONT, 44), ImageFont.truetype(FONT, 30)
    y = 120
    for i, ln in enumerate(lines):
        d.text((110, y), ln, fill="black", font=big if i == 0 else small)
        y += 90 if i == 0 else 60
    # 표 하나 — 선과 글
    y += 30
    for r, row in enumerate(TABLE):
        for c, v in enumerate(row):
            x0, y0 = 110 + c * 330, y + r * 60
            d.rectangle([x0, y0, x0 + 330, y0 + 60], outline="black", width=2)
            d.text((x0 + 15, y0 + 12), v, fill="black", font=small)
    return img.filter(ImageFilter.GaussianBlur(0.6))


def main():
    os.makedirs(OUT, exist_ok=True)
    make_docx(os.path.join(OUT, "보고서_예시.docx"))
    make_xlsx(os.path.join(OUT, "점검표_예시.xlsx"))
    make_pptx(os.path.join(OUT, "발표_예시.pptx"))
    make_text_pdf(os.path.join(OUT, "글자층_예시.pdf"))
    with open(os.path.join(OUT, "문서_예시.md"), "w", encoding="utf-8", newline="\n") as f:
        f.write("# %s\n\n## 1. 개요\n\n%s\n\n- %s\n- %s\n\n| 항목 | 값 |\n|---|---|\n| 형식 | md |\n\n```\n코드 조각 예시\n```\n"
                % (TITLE, SECTIONS[0][1][0], ITEMS[0], ITEMS[1]))
    with open(os.path.join(OUT, "평문_예시.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("평문 시험 파일입니다.\n둘째 줄\t탭 구분\t값\n")
    p1 = scan_image(["스캔 시험 문서", "이 쪽은 글자층이 없는 그림이다.", "OCR 로 읽어야 글이 나온다."])
    p2 = scan_image(["둘째 쪽", "처리 절차와 결과를 적은 쪽이다.", "표의 글도 함께 읽힌다."])
    p1.save(os.path.join(OUT, "스캔_예시.png"))
    p2.convert("RGB").save(os.path.join(OUT, "스캔_예시.jpg"), quality=85)
    # 글자층 없는 스캔 PDF (2쪽)
    p1.save(os.path.join(OUT, "스캔_예시.pdf"), save_all=True, append_images=[p2], resolution=150)
    Image.new("RGB", (24, 24), (200, 30, 30)).save(os.path.join(OUT, "아이콘_작음.png"))
    names = sorted(os.listdir(OUT))
    with open(os.path.join(ROOT, "tests", "samples.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("".join("tests/samples/%s\n" % n for n in names))
    print("만듦: %d개 → tests/samples, 목록 tests/samples.txt" % len(names))


if __name__ == "__main__":
    main()
