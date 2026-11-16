"""OCR 구조 분석(ocr.layout) 평가 — 원본 텍스트층 PDF 를 정답 삼아 라벨 · 순서 · 문단을 잰다.

사용:
  sdp dump <목록> <끈 폴더> --mode deferred --cache <캐시>
  sdp dump <목록> <켠 폴더> --mode deferred --cache <캐시> --layout
  python tools/ocr_compare/eval_layout.py <끈 폴더> <켠 폴더> [결과 json] [--set <truth.json 폴더>]

정답(자동, OCR 구조분석 1단계 설계서 §8):
  - 줄 · 읽는 순서 = 원본 PDF 의 pymupdf 줄 · 텍스트층 순서
  - 문단   = 텍스트층 줄 위치로 정한다(pymupdf 블록은 한글 PDF 에서 거의 줄마다 나뉘어 못 쓴다):
             앞 줄과 세로 틈이 글자 크기의 0.6배 넘게 벌어지거나, 글자 크기가 바뀌거나,
             앞 줄이 같은 쪽 줄들의 오른쪽 끝보다 글자 3개 넘게 일찍 끝나면 새 문단
  - 제목   = 문서 본문 글자 크기(글자 수 가중 최빈)의 1.15배 이상인 짧은 줄,
             또는 줄 전체가 굵고 블록에 혼자 있는 짧은 줄
  - 가장자리 = 쪽 위 · 아래 8% 안의 60자 이하 줄(머리말 · 꼬리말 · 쪽번호를 한데)
  - 표 줄  = pymupdf find_tables 영역 안의 줄
  - 그림 줄 = 그림(이미지) 또는 쪽의 10% 이상인 선 그림 묶음(cluster_drawings) 안의 줄 — 도식 속 글은
             제목 · 순서 · 문단 채점에서 뺀다(글자 F1 에는 남김)
  - 목차 줄 = '목차 · 차례'가 있는 쪽에서 쪽 번호로 끝나는 줄 — 제목 정답에서 뺀다
  정답도 추정이다 — 숫자는 켜기 전후 비교용으로 본다.
"""
import collections, itertools, json, re, sys
from pathlib import Path

import pymupdf

from common import SET_DIR
from evaluate import nospace, score

EDGE_ZONE = 0.08
HEAD_SIZE_RATIO = 1.15
HEAD_MAX_CHARS = 60
MATCH_MIN = 0.6
FIG_MIN_AREA = 0.10
EDGE_KINDS = {"header", "footer", "page_number"}


#------------------------------------------------------------------
# 글자 두 개씩 묶음(바이그램) 개수
#=> 공백을 뺀 글에서 이웃한 두 글자 묶음을 센다. 줄과 블록이 같은 글을 담았는지
#   순서와 상관없이 비교할 때 쓴다(한 글자짜리는 그 글자 하나).
#
# -in: s = 문자열
#
# -out: Counter = {두 글자: 개수}
# -out: error = 없음
#------------------------------------------------------------------
def bigrams(s):
    t = nospace(s)
    if len(t) < 2:
        return collections.Counter([t]) if t else collections.Counter()
    return collections.Counter(t[i:i + 2] for i in range(len(t) - 1))


#------------------------------------------------------------------
# a 의 글이 b 안에 얼마나 들어 있나 (0~1)
#=> a 의 바이그램 중 b 에도 있는 비율. 1 이면 a 가 b 에 통째로 들어 있다.
#
# -in: a, b = bigrams() 결과
#
# -out: ratio = 0~1
# -out: error = 없음 (a 가 비면 0)
#------------------------------------------------------------------
def covered(a, b):
    n = sum(a.values())
    return sum((a & b).values()) / n if n else 0.0


#------------------------------------------------------------------
# 원본 PDF 쪽 하나 → 정답 줄 목록
#=> pymupdf 줄마다 글 · 글자 크기 · 굵기 · 블록 번호 · 위치를 모으고,
#   표 영역 안 줄과 가장자리 줄을 표시한다.
#    1) get_text("dict") 의 블록 → 줄 → 조각(span)
#    2) find_tables 로 표 영역(실패하면 표 없음)
#    3) 쪽 위 · 아래 EDGE_ZONE 안의 짧은 줄은 가장자리
#
# -in: page = pymupdf 쪽
#
# -out: lines = [{text, size, bold, block, edge, table, solo}] — 텍스트층 순서
# -out: error = 없음 (표 찾기 실패는 무시)
#------------------------------------------------------------------
def truth_lines(page):
    try:
        tabs = [t.bbox for t in page.find_tables().tables]
    except Exception:
        tabs = []
    area = page.rect.width * page.rect.height
    # 그림 영역 — 이미지와 큰 선 그림 묶음(표 영역과 겹치는 묶음은 표로 본다)
    figs = [tuple(i["bbox"]) for i in page.get_image_info()]
    try:
        for r in page.cluster_drawings():
            if r.width * r.height >= area * FIG_MIN_AREA and not any(
                    abs(r.x0 - a) < 5 and abs(r.y0 - b) < 5 for a, b, _, _ in tabs):
                figs.append((r.x0, r.y0, r.x1, r.y1))
    except Exception:
        pass
    toc_page = bool(re.search(r"^\s*(목\s*차|차\s*례|CONTENTS)\s*$", page.get_text(), re.M | re.I))
    h = page.rect.height
    out = []
    for bi, blk in enumerate(page.get_text("dict")["blocks"]):
        lines = [l for l in blk.get("lines", []) if "".join(s["text"] for s in l["spans"]).strip()]
        for ln in lines:
            spans = [s for s in ln["spans"] if s["text"].strip()]
            text = "".join(s["text"] for s in ln["spans"]).strip()
            x0, y0, x1, y1 = ln["bbox"]
            # 줄 가운데가 표 영역 안이면 표 줄
            cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
            in_tab = any(a <= cx <= c and b <= cy <= d for a, b, c, d in tabs)
            in_fig = not in_tab and any(a <= cx <= c and b <= cy <= d for a, b, c, d in figs)
            out.append({
                "text": text,
                "size": max(s["size"] for s in spans),
                # pymupdf 글꼴 flags 의 16 = 굵게
                "bold": all(s["flags"] & 16 for s in spans),
                "block": bi,
                "edge": (y1 <= h * EDGE_ZONE or y0 >= h * (1 - EDGE_ZONE)) and len(nospace(text)) <= HEAD_MAX_CHARS,
                "table": in_tab,
                "figure": in_fig,
                "toc": toc_page and bool(re.search(r"\s\d{1,3}\s*$", text)),
                "solo": len(lines) == 1,
                "y0": y0, "y1": y1, "x0": x0, "x1": x1,
            })
    return para_ids(merge_rows(out))


#------------------------------------------------------------------
# 정답 문단 번호 매기기 (텍스트층 줄 위치로)
#=> 이웃한 두 줄이 같은 문단인지 텍스트층의 정확한 위치로 정한다. 다음 중 하나면 새 문단:
#    1) 세로 틈 > 글자 크기 × 0.6
#    2) 글자 크기가 0.5pt 넘게 다름
#    3) 앞 줄이 그 줄 무리의 오른쪽 끝보다 글자 3개 넘게 일찍 끝남(문단 마지막 줄)
#    4) 왼쪽 끝이 글자 2개 넘게 다름(들여쓰기 · 다른 단)
#
# -in: lines = merge_rows 결과
#
# -out: lines = 줄마다 "block"(문단 번호)을 다시 매긴 목록
# -out: error = 없음
#------------------------------------------------------------------
def para_ids(lines):
    if not lines:
        return lines
    right = max(l["x1"] for l in lines)
    pid = 0
    lines[0]["block"] = pid
    for p, l in zip(lines, lines[1:]):
        sz = max(p["size"], 1.0)
        # 단이 둘이면 오른쪽 끝도 단마다 다르다 — 앞 줄 기준 오른쪽 끝은 같은 왼쪽 끝 줄들의 최댓값
        col_right = max((x["x1"] for x in lines if abs(x["x0"] - p["x0"]) <= 2 * sz), default=right)
        new = (l["y0"] - p["y1"] > 0.6 * sz or abs(l["size"] - p["size"]) > 0.5
               or p["x1"] < col_right - 3 * sz or abs(l["x0"] - p["x0"]) > 2 * sz)
        pid += new
        l["block"] = pid
    return lines


#------------------------------------------------------------------
# 같은 높이의 이웃 줄을 한 화면 줄로 합치기
#=> pymupdf 는 탭으로 떨어진 "1.2.3" 과 "ISM 서비스 패턴" 을 다른 줄로 내기도 한다.
#   OCR 은 화면 한 줄을 하나로 묶으므로, 정답도 텍스트층 순서로 이웃하고 세로로 절반 넘게 겹치는 줄을 합친다.
#
# -in: lines = truth_lines 의 줄 목록(y0 · y1 포함)
#
# -out: lines = 합친 줄 목록
# -out: error = 없음
#------------------------------------------------------------------
def merge_rows(lines):
    out = []
    for l in lines:
        p = out[-1] if out else None
        if p is not None:
            ov = min(p["y1"], l["y1"]) - max(p["y0"], l["y0"])
            if ov >= 0.5 * min(p["y1"] - p["y0"], l["y1"] - l["y0"]):
                p["text"] = p["text"] + " " + l["text"]
                p["size"] = max(p["size"], l["size"])
                p["bold"] = p["bold"] and l["bold"]
                for k in ("edge", "table", "figure"):
                    p[k] = p[k] and l[k]
                p["toc"] = p["toc"] or l["toc"]
                p["solo"] = p["solo"] or l["solo"]
                continue
        out.append(dict(l))
    return out


#------------------------------------------------------------------
# 문서 정답 만들기 (제목 표시까지)
#=> 쪽마다 truth_lines 를 모은 뒤 문서 본문 글자 크기를 정하고 제목 줄을 표시한다.
#
# -in: src = 원본 PDF 경로 / pages = 쓸 쪽 번호(0부터, truth.json 그대로)
#
# -out: {쪽(1부터, 스캔본 기준): 정답 줄 목록}
# -out: error = 원본을 못 열면 예외 전파
#------------------------------------------------------------------
def doc_truth(src, pages):
    d = pymupdf.open(src)
    per = {k + 1: truth_lines(d[p]) for k, p in enumerate(pages)}
    # 본문 크기 = 글자 수 가중 최빈(0.5pt 단위)
    w = collections.Counter()
    for ls in per.values():
        for l in ls:
            if len(nospace(l["text"])) >= 15 and not l["table"] and not l["figure"]:
                w[round(l["size"] * 2) / 2] += len(nospace(l["text"]))
    body = w.most_common(1)[0][0] if w else 10.0
    for ls in per.values():
        for l in ls:
            short = len(nospace(l["text"])) <= HEAD_MAX_CHARS and re.search(r"[가-힣A-Za-z]", l["text"])
            big = l["size"] >= body * HEAD_SIZE_RATIO
            l["heading"] = bool(short and not l["edge"] and not l["table"] and not l["figure"] and not l["toc"] and (big or (l["bold"] and l["solo"])))
    return per


#------------------------------------------------------------------
# sdp dump 결과 → 문서별 블록 목록
#=> <폴더>/<번호>/<이름>.blocks.json 을 읽어 {scanNN: blocks} 로 돌려준다.
#
# -in: folder = dump 폴더
#
# -out: {문서 이름: [블록]}
# -out: error = 없음 (못 읽는 파일은 건너뜀)
#------------------------------------------------------------------
def load_dump(folder):
    out = {}
    for f in Path(folder).glob("*/*.blocks.json"):
        name = f.name.split(".")[0]
        try:
            out[name] = json.load(open(f, encoding="utf-8"))["blocks"]
        except Exception:
            continue
    return out


#------------------------------------------------------------------
# Kendall τ (순서 일치도, -1~1)
#=> 정답 순서대로 놓인 값들 중 순서가 맞는 쌍 - 틀린 쌍을 전체 쌍으로 나눈다.
#   같은 값(같은 블록 · 같은 자리) 쌍은 뺀다.
#
# -in: xs = OCR 쪽 자리 목록(정답 순서)
#
# -out: tau 또는 None(쌍이 없음)
# -out: error = 없음
#------------------------------------------------------------------
def kendall(xs):
    c = d = 0
    for a, b in itertools.combinations(xs, 2):
        if a < b:
            c += 1
        elif a > b:
            d += 1
    return (c - d) / (c + d) if c + d else None


#------------------------------------------------------------------
# 한 판(끈 · 켠) 채점
#=> 정답 줄마다 그 글을 가장 많이 담은 OCR 블록을 찾아(같은 쪽) 라벨 · 순서 · 문단 경계를 잰다.
#    1) 줄 ↔ 블록 짝짓기: covered(줄, 블록) 최대, MATCH_MIN 미만이면 짝 없음
#    2) 제목 · 가장자리 · 목록: 재현율 = 정답 줄이 그 라벨 블록에 들어갔나,
#       정밀도 = 그 라벨 블록이 가장 많이 담은 정답 줄이 그 라벨인가
#    3) 순서: 본문 줄의 (블록 번호, 블록 안 위치) 로 Kendall τ
#    4) 문단 경계: 이웃한 정답 줄이 다른 pymupdf 블록인가 vs 다른 OCR 블록인가
#    5) 표 줄이 제목이 된 수
#    6) 글자 F1 (evaluate.score)
#
# -in: truth = {문서: {쪽: 정답 줄}} / dump = {문서: 블록}
#
# -out: dict = 지표 묶음
# -out: error = 없음
#------------------------------------------------------------------
def evaluate_run(truth, dump):
    cnt = collections.Counter()
    taus, f1s = [], []
    for name, pages in truth.items():
        blocks = dump.get(name, [])
        for pno, lines in pages.items():
            bl = [b for b in blocks if b.get("page") == pno and b.get("type") != "table"]
            bg = [bigrams(b.get("text") or "") for b in bl]
            gt_text = "\n".join(l["text"] for l in lines)
            f1s.append((score(gt_text, "\n".join(b.get("text") or "" for b in bl)), len(nospace(gt_text))))
            # 1) 짝짓기
            match = []
            for l in lines:
                lb = bigrams(l["text"])
                best, bi = 0.0, None
                for k, g in enumerate(bg):
                    c = covered(lb, g)
                    if c > best:
                        best, bi = c, k
                match.append(bi if best >= MATCH_MIN and sum(lb.values()) >= 2 else None)
            # 2) 재현율
            for l, bi in zip(lines, match):
                kind = bl[bi]["type"] if bi is not None else None
                for lab, want in (("head", l["heading"]), ("edge", l["edge"])):
                    if want:
                        cnt[f"{lab}_gt"] += 1
                        hit = (kind == "heading") if lab == "head" else (kind in EDGE_KINDS)
                        cnt[f"{lab}_tp_r"] += hit
                if l["table"] and kind == "heading":
                    cnt["table_as_head"] += 1
            # 2) 정밀도 — 라벨 블록마다 가장 많이 담긴 정답 줄
            lbg = [bigrams(l["text"]) for l in lines]
            for b, g in zip(bl, bg):
                lab = "head" if b["type"] == "heading" else ("edge" if b["type"] in EDGE_KINDS else None)
                if lab is None:
                    continue
                cnt[f"{lab}_pred"] += 1
                best, li = 0.0, None
                for k, lg in enumerate(lbg):
                    c = covered(g, lg)
                    if c > best:
                        best, li = c, k
                if li is not None and best >= MATCH_MIN:
                    ok = lines[li]["heading"] if lab == "head" else lines[li]["edge"]
                    cnt[f"{lab}_tp_p"] += ok
                    if lab == "head" and not ok:
                        cnt["head_fp_fig"] += lines[li]["figure"]
                        cnt["head_fp_table"] += lines[li]["table"]
            # 3) 순서 — 본문 줄만, 블록 번호로 잰다(같은 블록 안 쌍은 kendall 이 같음으로 보고 뺀다)
            #    블록 안 위치까지 재면 OCR 오자로 글을 못 찾은 줄이 앞자리로 가 순서가 틀린 것처럼 나온다
            pos = []
            for l, bi in zip(lines, match):
                if bi is None or l["edge"] or l["table"] or l["figure"]:
                    continue
                pos.append(bi)
            tau = kendall(pos)
            if tau is not None:
                taus.append((tau, len(pos)))
            # 4) 문단 경계
            for (l1, b1), (l2, b2) in zip(zip(lines, match), zip(lines[1:], match[1:])):
                if b1 is None or b2 is None or any(l[k] for l in (l1, l2) for k in ("edge", "table", "figure")):
                    continue
                gt_b, pr_b = l1["block"] != l2["block"], b1 != b2
                cnt["para_tp"] += gt_b and pr_b
                cnt["para_gt"] += gt_b
                cnt["para_pred"] += pr_b

    def pr(tp_p, pred, tp_r, gt):
        p, r = cnt[tp_p] / max(1, cnt[pred]), cnt[tp_r] / max(1, cnt[gt])
        return {"p": round(p, 3), "r": round(r, 3), "f1": round(2 * p * r / max(1e-9, p + r), 3), "n_gt": cnt[gt], "n_pred": cnt[pred]}

    wsum = sum(n for _, n in taus) or 1
    nsum = sum(n for _, n in f1s) or 1
    return {
        "char_f1": round(sum(s["f1"] * n for s, n in f1s) / nsum, 4),
        "order_tau": round(sum(t * n for t, n in taus) / wsum, 3),
        "heading": pr("head_tp_p", "head_pred", "head_tp_r", "head_gt"),
        "edge": pr("edge_tp_p", "edge_pred", "edge_tp_r", "edge_gt"),
        "para_boundary": pr("para_tp", "para_pred", "para_tp", "para_gt"),
        "table_as_heading": cnt["table_as_head"],
        # 틀린 제목 중 그림 · 표 안 글이었던 수(정답이 그 줄을 제목으로 안 보는 곳)
        "head_fp_in_figure": cnt["head_fp_fig"],
        "head_fp_in_table": cnt["head_fp_table"],
    }


#------------------------------------------------------------------
# 진입점 — 끈 · 켠 결과를 같은 정답으로 채점해 나란히 찍는다
#
# -in: sys.argv = [끈 폴더, 켠 폴더, (결과 json)]
#
# -out: 없음 (화면 출력, 결과 json 이 있으면 저장)
# -out: error = 인자가 모자라면 사용법을 찍고 종료 코드 2
#------------------------------------------------------------------
def main():
    args = sys.argv[1:]
    # --set <폴더> 로 다른 시험셋(예: make_layout_set.py 결과)의 truth.json 을 쓴다
    set_dir = SET_DIR
    if "--set" in args:
        k = args.index("--set")
        set_dir = Path(args[k + 1])
        del args[k:k + 2]
    if len(args) < 2:
        print(__doc__)
        sys.exit(2)
    sys.argv = [sys.argv[0]] + args
    meta = json.load(open(set_dir / "truth.json", encoding="utf-8"))
    truth = {name: doc_truth(v["src"], v["pages"]) for name, v in meta.items()}
    res = {tag: evaluate_run(truth, load_dump(folder)) for tag, folder in (("off", sys.argv[1]), ("on", sys.argv[2]))}
    for k in ("char_f1", "order_tau", "table_as_heading", "head_fp_in_figure", "head_fp_in_table"):
        print(f"{k:18} off={res['off'][k]}  on={res['on'][k]}")
    for k in ("heading", "edge", "para_boundary"):
        for tag in ("off", "on"):
            print(f"{k:18} {tag:3} {res[tag][k]}")
    if len(sys.argv) > 3:
        json.dump(res, open(sys.argv[3], "w", encoding="utf-8"), ensure_ascii=False, indent=1)


if __name__ == "__main__":
    main()
