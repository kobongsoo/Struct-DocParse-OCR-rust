"""정답(텍스트층) 대비 우리 OCR · Newploy OCR 글자 정확도 채점.

사용: python tools/ocr_compare/evaluate.py
출력: 화면 요약 + tests/private/ocr_compare_newploy/scores.json(쪽별) · summary.json(묶음별)
"""
import collections, json, re, unicodedata

from rapidfuzz.distance import Levenshtein

from common import DATA, NP_DIR, OURS_DIR, SET_DIR

SYSTEMS = ("ours", "np")
METRICS = (("f1", "글자 F1"), ("han", "한글 F1"), ("aln", "영문·숫자 F1"), ("wr", "단어 재현율"),
           ("sp", "띄어쓰기 F1"), ("rec", "글자 재현율"), ("prec", "글자 정밀도"), ("cer", "CER(순서 포함)"))


#------------------------------------------------------------------
# 글자 정규화
#=> 같은 글자인데 코드만 다른 것(전각·호환 문자, 굽은 따옴표, 가운뎃점 변형)을 하나로 맞춘다.
#   맞추지 않으면 OCR 이 맞게 읽어도 틀린 것으로 셈해진다.
#
# -in: s = 원본 문자열 (None 가능)
#
# -out: s = 정규화한 문자열
# -out: error = 없음
#------------------------------------------------------------------
def norm(s):
    s = unicodedata.normalize("NFKC", s or "")
    for a, b in (("․", "."), ("ㆍ", "·"), ("‧", "·"), ("“", '"'), ("”", '"'),
                 ("‘", "'"), ("’", "'"), ("・", "·"), ("ㆍ", "·")):
        s = s.replace(a, b)
    return s


#------------------------------------------------------------------
# 공백을 모두 뺀 정규화 글자열
#=> 띄어쓰기·줄바꿈 차이를 글자 채점에서 빼려고 쓴다(띄어쓰기는 따로 잰다).
#
# -in: s = 문자열
#
# -out: s = 공백 없는 정규화 문자열
# -out: error = 없음
#------------------------------------------------------------------
def nospace(s):
    return re.sub(r"\s", "", norm(s))


#------------------------------------------------------------------
# 글자 주머니 맞은 수 세기
#=> 읽는 순서와 상관없이 글자 개수만 맞춰 본다(정답과 OCR 의 읽기 순서가 달라도 공정).
#    1) 공백 뺀 글자열에서 (pat 이 있으면 그 글자만) 남긴다
#    2) 글자별 개수의 교집합 = 맞은 글자 수
#
# -in: gt  = 정답 글자
# -in: hy  = OCR 결과 글자
# -in: pat = 남길 글자 정규식 (예: 한글만 "[가-힣]"), 기본 None = 모든 글자
#
# -out: (tp, n_gt, n_hy) = 맞은 수, 정답 글자 수, OCR 글자 수
# -out: error = 없음
#------------------------------------------------------------------
def bag(gt, hy, pat=None):
    g, h = nospace(gt), nospace(hy)
    if pat:
        g, h = "".join(re.findall(pat, g)), "".join(re.findall(pat, h))
    cg, ch = collections.Counter(g), collections.Counter(h)
    return sum((cg & ch).values()), len(g), len(h)


#------------------------------------------------------------------
# F1 계산
#=> 정밀도(읽은 것 중 맞은 비율)와 재현율(정답 중 읽은 비율)의 조화 평균.
#
# -in: tp = 맞은 수
# -in: ng = 정답 수
# -in: nh = 예측 수
#
# -out: f1 = 0~1
# -out: error = 없음 (0 나누기는 0 으로 처리)
#------------------------------------------------------------------
def f1_of(tp, ng, nh):
    p, r = tp / max(1, nh), tp / max(1, ng)
    return 2 * p * r / max(1e-9, p + r)


#------------------------------------------------------------------
# 단어 재현율
#=> 정답의 2글자 이상 단어가 OCR 결과(공백 뺀 글자열) 안에 그대로 들어 있는 비율.
#   검색에서 '그 단어로 찾아지는가'에 가깝다.
#
# -in: gt = 정답 글자
# -in: hy = OCR 결과 글자
#
# -out: wr = 0~1
# -out: error = 없음
#------------------------------------------------------------------
def word_recall(gt, hy):
    h = nospace(hy)
    ws = [w for w in (nospace(x) for x in norm(gt).split()) if len(w) >= 2]
    return sum(w in h for w in ws) / max(1, len(ws))


#------------------------------------------------------------------
# 띄어쓰기 F1
#=> 정답과 OCR 의 '공백이 들어간 자리'가 얼마나 겹치는지 잰다.
#    1) 각 글자열에서 공백을 빼고, 공백이 있던 자리(앞 글자 수)를 모은다
#    2) 공백 없는 두 글자열을 정렬(Levenshtein)해 같은 글자끼리 위치를 잇는다
#    3) OCR 공백 자리를 정답 위치로 옮겨 겹친 수로 F1
#
# -in: gt = 정답 글자
# -in: hy = OCR 결과 글자
#
# -out: sp = 0~1
# -out: error = 없음
#------------------------------------------------------------------
def space_f1(gt, hy):
    # 공백 자리 집합과 공백 없는 글자열을 만든다
    def bounds(s):
        s = re.sub(r"\s+", " ", norm(s)).strip()
        out, seq = set(), []
        for c in s:
            if c == " ":
                out.add(len(seq))
            else:
                seq.append(c)
        return out, "".join(seq)
    gb, gs = bounds(gt)
    hb, hs = bounds(hy)
    m = {}
    for tag, a0, a1, b0, b1 in Levenshtein.opcodes(gs, hs):
        if tag == "equal":
            for k in range(a1 - a0):
                m[b0 + k] = a0 + k
    hb2 = {m[x] for x in hb if x in m}
    return f1_of(len(gb & hb2), len(gb), len(hb2))


#------------------------------------------------------------------
# 한 쪽 채점
#=> 정답과 OCR 결과로 모든 지표를 한 번에 계산한다.
#
# -in: gt = 정답 글자
# -in: hy = OCR 결과 글자
#
# -out: dict = f1·han·aln·wr·sp·rec·prec·cer·len
# -out: error = 없음
#------------------------------------------------------------------
def score(gt, hy):
    tp, ng, nh = bag(gt, hy)
    return {"f1": f1_of(tp, ng, nh), "han": f1_of(*bag(gt, hy, r"[가-힣]")), "aln": f1_of(*bag(gt, hy, r"[A-Za-z0-9]")),
            "wr": word_recall(gt, hy), "sp": space_f1(gt, hy), "rec": tp / max(1, ng), "prec": tp / max(1, nh),
            # CER 은 읽는 순서까지 따지므로 정답 텍스트층 순서와 다르면 크게 나빠진다 — 참고용
            "cer": Levenshtein.distance(nospace(gt), nospace(hy)) / max(1, ng), "len": nh}


#------------------------------------------------------------------
# 우리 OCR 결과 → 쪽별 글자
#=> sdp.exe parse JSON 의 blocks 를 쪽(page)별로 이어 붙인다. 표 블록은 행·칸 글자를 쓴다.
#
# -in: name = 문서 이름 (scanNN)
#
# -out: pages = {쪽번호: 글자}
# -out: error = 파일이 없으면 FileNotFoundError
#------------------------------------------------------------------
def ours_pages(name):
    d = json.load(open(OURS_DIR / f"{name}.json", encoding="utf-8"))
    pages = collections.defaultdict(list)
    for b in d["blocks"]:
        t = b.get("text") or ""
        if b.get("type") == "table" and b.get("rows"):
            t = "\n".join(" ".join(c or "" for c in r) for r in b["rows"])
        pages[b.get("page")].append(t)
    return {k: "\n".join(v) for k, v in pages.items()}


#------------------------------------------------------------------
# Newploy 영역 하나의 글자
#=> 보통은 text 를 쓰고, 표 영역은 text 가 비어 있으면 내보낸 HTML·TSV 에서 글자를 꺼낸다.
#
# -in: r = document_result.json 의 region
#
# -out: text = 영역 글자 (없으면 "")
# -out: error = 없음
#------------------------------------------------------------------
def region_text(r):
    t = r.get("text") or ""
    if not t and r.get("type") == "table":
        ex = r.get("exports") or {}
        t = re.sub(r"<[^>]+>", " ", ex.get("html_with_text") or "")
        if not t.strip():
            t = ex.get("tsv") or ""
    return t


#------------------------------------------------------------------
# Newploy 결과 → 쪽별 글자 · 쪽 정보
#=> newploy/scanNN.json(document_result 복사본)을 읽어 쪽별 글자와 영역 종류·오류 여부를 낸다.
#
# -in: name = 문서 이름 (scanNN)
#
# -out: (pages, info, job) = {쪽: 글자}, {쪽: (영역 종류 Counter, 오류 목록)}, job 정보
# -out: error = 파일이 없으면 FileNotFoundError
#------------------------------------------------------------------
def np_pages(name):
    d = json.load(open(NP_DIR / f"{name}.json", encoding="utf-8"))
    pages, info = {}, {}
    for p in d["pages"]:
        rs = p.get("regions", [])
        pages[p["page_index"]] = "\n".join(region_text(r) for r in rs)
        info[p["page_index"]] = (collections.Counter(r.get("type") for r in rs),
                                 [str(r.get("error")) for r in rs if r.get("error")])
    return pages, info, d.get("job", {})


#------------------------------------------------------------------
# 쪽 묶음 평균
#=> 정답 글자 수로 가중해 평균한다(짧은 쪽이 점수를 흔들지 않게).
#   재현율·정밀도는 쪽 평균이 아니라 전체 글자 합으로 다시 계산한다.
#
# -in: rows = 쪽 채점 목록
# -in: sys_ = "ours" 또는 "np"
#
# -out: dict = 지표별 평균 + 출력/정답 글자 비율
# -out: error = 없음 (빈 목록이면 0)
#------------------------------------------------------------------
def agg(rows, sys_):
    w = sum(r["gt_len"] for r in rows) or 1
    out = {k: sum(r[sys_][k] * r["gt_len"] for r in rows) / w for k, _ in METRICS}
    tp = sum(r[sys_]["rec"] * r["gt_len"] for r in rows)
    nh = sum(r[sys_]["len"] for r in rows)
    out["prec"] = tp / max(1, nh)
    out["ratio"] = nh / w
    return out


#------------------------------------------------------------------
# 채점 본체
#=> 모든 쪽을 채점하고 묶음(전체·본문만·그림·표 오류)별·문서별로 요약해 저장·출력한다.
#    1) truth.json 의 쪽마다 두 시스템 결과를 채점
#    2) Newploy 영역 정보로 쪽을 분류 (표 오류 / 그림·표 있음 / 본문만)
#    3) scores.json(쪽별) · summary.json(묶음·문서별) 저장
#
# -in: 없음
#
# -out: 없음 (화면 출력 + 파일 2개)
# -out: error = 결과 파일이 없으면 FileNotFoundError
#------------------------------------------------------------------
def main():
    truth = json.load(open(SET_DIR / "truth.json", encoding="utf-8"))
    rows, docs = [], {}
    for name, v in sorted(truth.items()):
        op = ours_pages(name)
        npp, info, job = np_pages(name)
        docs[name] = {"src": v["src"], "pages": len(v["pages"]), "np_job": job}
        for i, gt in enumerate(v["text"], 1):
            types, errs = info.get(i, (collections.Counter(), []))
            # 표 단계 오류 > 그림·표 있음 > 본문만 순으로 쪽을 나눈다
            kind = "table_error" if errs else ("figure_table" if {"table", "figure", "image", "chart"} & set(types) else "plain")
            rows.append({"doc": name, "page": i, "src_page": v["pages"][i - 1] + 1, "gt_len": len(nospace(gt)),
                         "kind": kind, "np_regions": dict(types), "np_errors": errs,
                         "ours": score(gt, op.get(i, "")), "np": score(gt, npp.get(i, ""))})
    json.dump(rows, open(DATA / "scores.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)

    groups = {"all": rows, "plain": [r for r in rows if r["kind"] == "plain"],
              "figure_table": [r for r in rows if r["kind"] == "figure_table"],
              "table_error": [r for r in rows if r["kind"] == "table_error"],
              "no_table_error": [r for r in rows if r["kind"] != "table_error"]}
    summary = {"groups": {}, "docs": {}}
    for g, rs in groups.items():
        summary["groups"][g] = {"pages": len(rs), "gt_chars": sum(r["gt_len"] for r in rs),
                                **{s: agg(rs, s) for s in SYSTEMS}}
    for name in docs:
        rs = [r for r in rows if r["doc"] == name]
        summary["docs"][name] = {**docs[name], **{s: agg(rs, s) for s in SYSTEMS}}
    win = sum(r["np"]["f1"] > r["ours"]["f1"] + 0.005 for r in rows)
    lose = sum(r["ours"]["f1"] > r["np"]["f1"] + 0.005 for r in rows)
    summary["page_wins"] = {"np": win, "ours": lose, "tie": len(rows) - win - lose}
    json.dump(summary, open(DATA / "summary.json", "w", encoding="utf-8"), ensure_ascii=False, indent=1)

    for g, s in summary["groups"].items():
        print(f"\n[{g}] {s['pages']}쪽 · 정답 {s['gt_chars']}자")
        for k, lab in METRICS:
            print(f"  {lab:12s} ours={s['ours'][k]:.3f}  newploy={s['np'][k]:.3f}")
    print("\n문서별 글자 F1")
    for name, s in summary["docs"].items():
        print(f"  {name} ours {s['ours']['f1']:.3f}  np {s['np']['f1']:.3f}  {s['src']}")
    print("\n쪽별 우세", summary["page_wins"])


if __name__ == "__main__":
    main()
