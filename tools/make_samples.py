# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 시험용 합성 문서 만들기 — tests/samples/
#=> 공개 저장소에 실제 문서를 넣지 않으려고, 형식별 시험 문서를 내용까지 지어내 만든다.
#   docx · xlsx · pptx · md · txt · 글자층 PDF · 스캔 그림(png · jpg) · 스캔 PDF · 작은 아이콘
#   hwp · hwpx · doc 는 만들 도구가 없어 빠진다 — 그 형식은 tests/private/ (git 에 안 올라감)에 직접 넣어 시험한다.
#
#   필요: python-docx · openpyxl · python-pptx · PyMuPDF · Pillow, 한글 글꼴(맑은 고딕 등)
#   사용: python tools/make_samples.py [글꼴 경로]           — 모든 시험 문서를 새로 만든다
#         python tools/make_samples.py --extra [글꼴 경로]   — 본문 밖 글 시험 문서만 만든다(본문밖글읽기 설계서 11장).
#                                                            기존 문서는 그대로 두어 바이트가 바뀌지 않게 한다
#------------------------------------------------------------------
import os
import sys
import zipfile

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "tests", "samples")
# 위치 인자(글꼴 경로)와 --extra 깃발을 나눈다
ARGS = [a for a in sys.argv[1:] if not a.startswith("--")]
ONLY_EXTRA = "--extra" in sys.argv[1:]
FONT = ARGS[0] if ARGS else r"C:\Windows\Fonts\malgun.ttf"

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


# ── 본문 밖 글 시험 문서 (본문밖글읽기 설계서 11-1) ─────────────────


#------------------------------------------------------------------
# PDF 를 작게 저장하기 — 시험 문서가 저장소를 키우지 않게
#=> 그냥 저장하면 한글 글꼴 파일 전체(약 13MB)와 압축 안 한 그림이 들어간다.
#    1) 쓴 글자만 남기게 글꼴을 줄인다(subset_fonts)
#    2) 안 쓰는 객체를 지우고(garbage=4) 스트림 · 그림 · 글꼴을 압축해 저장한다
#
# -in: doc  = PyMuPDF 문서
# -in: path = 저장할 파일
#
# -out: 없음
# -out: error = 저장 실패면 PyMuPDF 예외 전파
#------------------------------------------------------------------
def save_small(doc, path):
    # 글꼴 줄이기는 PyMuPDF 판에 따라 없을 수 있다 — 없으면 압축만 한다
    if hasattr(doc, "subset_fonts"):
        doc.subset_fonts()
    doc.save(path, garbage=4, deflate=True, deflate_images=True, deflate_fonts=True)
# 표지 낱말 — 결과에 이 낱말이 나오는지로 확인한다(영문: 글상자 겉모양이 기본 글꼴로 그려져도 OCR 이 읽게)
MK = {
    "freetext": "Visible note ALPHA",
    "hidden": "Hidden note BRAVO",
    "sticky": "Sticky note CHARLIE",
    "field": "Field value DELTA",
    "scan_vis": "Scan note ECHO",
    "scan_hid": "Scan hidden FOXTROT",
}


#------------------------------------------------------------------
# 글자층 쪽 PDF + 여러 주석 — 본문 밖 글 중 무엇을 읽고 무엇을 빼는지 시험
#=> 한 쪽에 본문 글과 아래 주석을 넣는다.
#    1) 보이는 글상자 · 숨긴 글상자(Hidden 깃발) — 둘 다 읽혀야 한다
#    2) 쪽지(Text) 주석 · 도장(Stamp) — 읽히면 안 된다
#    3) 글 칸(값 있음) — 값이 읽혀야 한다 / 체크 상자 — 값(Yes)이 읽히면 안 된다
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = PyMuPDF 가 없으면 ImportError
#------------------------------------------------------------------
def make_annot_text_pdf(path):
    import fitz
    doc = fitz.open()
    pg = doc.new_page(width=595, height=842)
    # 글자층 본문 — 이 쪽은 스캔 쪽이 아니어서 OCR 하지 않는다
    pg.insert_text((50, 90), "주석 시험 문서 — 글자층 쪽", fontsize=16, fontname="kr", fontfile=FONT)
    pg.insert_text((50, 125), "이 쪽의 본문 글은 글자층에 있다. 주석 글은 본문 밖 글이다.", fontsize=11, fontname="kr", fontfile=FONT)
    pg.add_freetext_annot(fitz.Rect(50, 200, 350, 225), MK["freetext"], fontsize=12)
    hid = pg.add_freetext_annot(fitz.Rect(50, 240, 350, 265), MK["hidden"], fontsize=12)
    # 숨김 깃발 — 화면에 안 보이지만 사이냅처럼 읽는다
    hid.set_flags(fitz.PDF_ANNOT_IS_HIDDEN)
    hid.update()
    pg.add_text_annot((420, 90), MK["sticky"])
    pg.add_stamp_annot(fitz.Rect(400, 200, 540, 250), stamp=fitz.STAMP_Approved)
    w = fitz.Widget()
    w.field_type = fitz.PDF_WIDGET_TYPE_TEXT
    w.field_name = "memo_field"
    w.field_value = MK["field"]
    w.rect = fitz.Rect(50, 300, 350, 325)
    pg.add_widget(w)
    cb = fitz.Widget()
    cb.field_type = fitz.PDF_WIDGET_TYPE_CHECKBOX
    cb.field_name = "agree_box"
    cb.field_value = True
    cb.rect = fitz.Rect(50, 340, 65, 355)
    pg.add_widget(cb)
    save_small(doc, path)


#------------------------------------------------------------------
# 스캔 쪽 PDF + 글상자 — OCR 쪽에서 같은 글이 두 번 들어가지 않는지 시험
#=> 쪽 전체를 그림으로 덮은 2쪽. 1쪽에 보이는 글상자 · 숨긴 글상자를 둔다.
#   OCR 켬: 보이는 글상자는 OCR 줄로만, 숨긴 글상자는 extra 블록으로만 나와야 한다.
#   OCR 끔: 둘 다 extra 블록으로 나와야 한다.
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = PyMuPDF 가 없으면 ImportError
#------------------------------------------------------------------
def make_annot_scan_pdf(path):
    import io
    import fitz
    doc = fitz.open()
    for lines in (["주석 스캔 시험", "이 쪽은 그림으로 덮인 스캔 쪽이다."], ["둘째 쪽", "글상자 없는 스캔 쪽이다."]):
        buf = io.BytesIO()
        scan_image(lines).save(buf, format="PNG")
        pg = doc.new_page(width=595, height=842)
        # 그림이 쪽 전체를 덮어야 스캔 쪽으로 판정된다
        pg.insert_image(pg.rect, stream=buf.getvalue())
    pg = doc[0]
    # 그림 아래쪽 빈 자리에 큰 글씨로 — OCR 이 또렷하게 읽게
    pg.add_freetext_annot(fitz.Rect(60, 700, 500, 735), MK["scan_vis"], fontsize=20)
    hid = pg.add_freetext_annot(fitz.Rect(60, 750, 500, 785), MK["scan_hid"], fontsize=20)
    hid.set_flags(fitz.PDF_ANNOT_IS_HIDDEN)
    hid.update()
    save_small(doc, path)


#------------------------------------------------------------------
# 메모 든 docx — 메모(comment)가 문서 끝 "메모" 아래 note 로 나오는지 시험
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = python-docx 1.2 미만이면 add_comment 가 없어 AttributeError
#------------------------------------------------------------------
def make_comment_docx(path):
    import docx
    d = docx.Document()
    d.add_heading("메모 시험 문서", level=1)
    p = d.add_paragraph("이 문단에 검토 메모가 달려 있다.")
    d.add_comment(p.runs, text="검토 메모 — 이 문장은 본문 밖 글이다.", author="시험", initials="T")
    d.save(path)


# SmartArt 데이터 부품(dgm:dataModel) — 글 있는 점 2개(node), 연결선 · 모양 · 뿌리 점(글 없음 · 되풀이)
DGM_DATA = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
<dgm:ptLst>
<dgm:pt modelId="{0}" type="doc"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:endParaRPr lang="ko-KR"/></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="{1}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>%s</a:t></a:r></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="{2}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>%s</a:t></a:r></a:p><a:p><a:r><a:t>%s</a:t></a:r></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="{3}" type="parTrans" cxnId="{9}"><dgm:prSet/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:endParaRPr lang="ko-KR"/></a:p></dgm:t></dgm:pt>
<dgm:pt modelId="{4}" type="pres"><dgm:prSet presName="node"/><dgm:spPr/><dgm:t><a:bodyPr/><a:p><a:r><a:t>%s</a:t></a:r></a:p></dgm:t></dgm:pt>
</dgm:ptLst>
<dgm:cxnLst/>
</dgm:dataModel>"""
SMART = ["기획 단계[▶요구 분석]", "개발 단계", "시험 · 배포", "모양 점 — 읽으면 안 됨"]
DGM_NS = "http://schemas.openxmlformats.org/drawingml/2006/diagram"
DGM_REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData"
DGM_CT = '<Override PartName="/%s" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/>'


#------------------------------------------------------------------
# ZIP 안 부품 몇 개를 바꾸거나 더해 새 파일로 쓰기
#=> python-docx · python-pptx 는 SmartArt 를 만들지 못하므로, 만든 파일을 ZIP 으로 다시 써서
#   데이터 부품 · 관계 · 가리킴을 직접 넣는다(PowerPoint · Word 가 고칠 수 있는 최소 모양 — 파서 시험용).
#
# -in: path    = 고칠 파일(제자리에서 다시 쓴다)
# -in: changes = {부품 이름: 새 바이트 또는 (옛 글 → 새 글) 함수}
#
# -out: 없음
# -out: error = ZIP 이 깨졌으면 zipfile.BadZipFile
#------------------------------------------------------------------
def rewrite_zip(path, changes):
    with zipfile.ZipFile(path) as zi:
        items = [(n, zi.read(n)) for n in zi.namelist()]
    names = {n for n, _ in items}
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as zo:
        for n, b in items:
            c = changes.get(n)
            # 함수면 옛 내용을 받아 고치고, 바이트면 통째로 바꾼다
            zo.writestr(n, c(b.decode("utf-8")).encode("utf-8") if callable(c) else (c if c is not None else b))
        for n, c in changes.items():
            if n not in names:
                zo.writestr(n, c)


#------------------------------------------------------------------
# SmartArt 든 pptx — SmartArt 글이 그 슬라이드의 그림 자리에 나오는지 시험
#=> 1쪽 제목 슬라이드에 graphicFrame(dgm:relIds r:dm) 하나를 넣고 ppt/diagrams/data1.xml 을 가리킨다.
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = python-pptx 가 없으면 ImportError
#------------------------------------------------------------------
def make_smartart_pptx(path):
    from pptx import Presentation
    p = Presentation()
    s = p.slides.add_slide(p.slide_layouts[5])
    s.shapes.title.text = "SmartArt 시험 슬라이드"
    p.save(path)
    frame = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="90" name="SmartArt 1"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
             '<p:xfrm><a:off x="914400" y="1828800"/><a:ext cx="6096000" cy="3048000"/></p:xfrm>'
             '<a:graphic><a:graphicData uri="%s"><dgm:relIds xmlns:dgm="%s" '
             'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:dm="rId90" r:lo="rId90" r:qs="rId90" r:cs="rId90"/>'
             '</a:graphicData></a:graphic></p:graphicFrame>') % (DGM_NS, DGM_NS)
    rewrite_zip(path, {
        "ppt/slides/slide1.xml": lambda x: x.replace("</p:spTree>", frame + "</p:spTree>", 1),
        "ppt/slides/_rels/slide1.xml.rels": lambda x: x.replace(
            "</Relationships>", '<Relationship Id="rId90" Type="%s" Target="../diagrams/data1.xml"/></Relationships>' % DGM_REL, 1),
        "[Content_Types].xml": lambda x: x.replace("</Types>", DGM_CT % "ppt/diagrams/data1.xml" + "</Types>", 1),
        "ppt/diagrams/data1.xml": (DGM_DATA % tuple(SMART)).encode("utf-8"),
    })


#------------------------------------------------------------------
# SmartArt 든 docx — SmartArt 글이 그 그림이 든 문단 뒤에 나오는지 시험
#=> 둘째 문단 안에 w:drawing(dgm:relIds r:dm) 을 넣고 word/diagrams/data1.xml 을 가리킨다.
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = python-docx 가 없으면 ImportError
#------------------------------------------------------------------
def make_smartart_docx(path):
    import docx
    d = docx.Document()
    d.add_heading("SmartArt 시험 문서", level=1)
    d.add_paragraph("SMARTART_HERE")
    d.add_paragraph("그림 뒤 문단")
    d.save(path)
    drawing = ('<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing">'
               '<wp:extent cx="5486400" cy="3200400"/><wp:docPr id="90" name="SmartArt 1"/>'
               '<a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="%s">'
               '<dgm:relIds xmlns:dgm="%s" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
               'r:dm="rId90" r:lo="rId90" r:qs="rId90" r:cs="rId90"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>') % (DGM_NS, DGM_NS)
    rewrite_zip(path, {
        # 자리표시 런("SMARTART_HERE")을 그림 런으로 바꾼다 — 그 문단에는 글이 없게 된다
        "word/document.xml": lambda x: x.replace("<w:r><w:t>SMARTART_HERE</w:t></w:r>", drawing, 1),
        "word/_rels/document.xml.rels": lambda x: x.replace(
            "</Relationships>", '<Relationship Id="rId90" Type="%s" Target="diagrams/data1.xml"/></Relationships>' % DGM_REL, 1),
        "[Content_Types].xml": lambda x: x.replace("</Types>", DGM_CT % "word/diagrams/data1.xml" + "</Types>", 1),
        "word/diagrams/data1.xml": (DGM_DATA % tuple(SMART)).encode("utf-8"),
    })


#------------------------------------------------------------------
# openpyxl 로 저장한 xlsx + 셀 메모 — 메모 부품이 xl/comments/comment1.xml 에 놓이는 꼴(설계서 7-3)
#
# -in: path = 만들 파일
#
# -out: 없음
# -out: error = openpyxl 이 없으면 ImportError
#------------------------------------------------------------------
def make_comment_xlsx(path):
    import openpyxl
    from openpyxl.comments import Comment
    wb = openpyxl.Workbook()
    ws = wb.active
    ws.title = "점검"
    ws["A1"], ws["B1"] = "항목", "결과"
    ws["A2"], ws["B2"] = "백업", "정상"
    ws["B2"].comment = Comment("셀 메모 — 다음 점검 때 다시 확인", "시험")
    wb.save(path)


#------------------------------------------------------------------
# 본문 밖 글 시험 문서 모두 만들기
#
# -in: 없음
#
# -out: 없음
# -out: error = 만들 도구가 없으면 ImportError
#------------------------------------------------------------------
def make_extra_samples():
    make_annot_text_pdf(os.path.join(OUT, "주석_글쪽.pdf"))
    make_annot_scan_pdf(os.path.join(OUT, "주석_스캔쪽.pdf"))
    make_comment_docx(os.path.join(OUT, "메모.docx"))
    make_smartart_pptx(os.path.join(OUT, "SmartArt.pptx"))
    make_smartart_docx(os.path.join(OUT, "SmartArt.docx"))
    make_comment_xlsx(os.path.join(OUT, "메모_openpyxl.xlsx"))


#------------------------------------------------------------------
# 시험 문서 목록 쓰기 — tests/samples 의 파일 이름 순
#
# -in: 없음
#
# -out: 파일 수
# -out: error = 쓰기 실패면 OSError
#------------------------------------------------------------------
def write_list():
    names = sorted(os.listdir(OUT))
    with open(os.path.join(ROOT, "tests", "samples.txt"), "w", encoding="utf-8", newline="\n") as f:
        f.write("".join("tests/samples/%s\n" % n for n in names))
    return len(names)


#------------------------------------------------------------------
# 진입점 — 모든 시험 문서(또는 --extra 면 본문 밖 글 문서만)를 만들고 목록을 쓴다
#
# -in: 없음 (명령줄은 위에서 읽었다)
#
# -out: 없음
# -out: error = 만들 도구가 없으면 ImportError
#------------------------------------------------------------------
def main():
    os.makedirs(OUT, exist_ok=True)
    # 본문 밖 글 문서만 — 기존 문서를 다시 만들면 바이트가 바뀌어 결과 비교가 흔들린다
    if ONLY_EXTRA:
        make_extra_samples()
        print("만듦: 본문 밖 글 시험 문서 → tests/samples, 목록 %d개" % write_list())
        return
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
    make_extra_samples()
    print("만듦: %d개 → tests/samples, 목록 tests/samples.txt" % write_list())


if __name__ == "__main__":
    main()
