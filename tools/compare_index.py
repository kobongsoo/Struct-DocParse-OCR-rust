# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 두 SimpleRAG 데이터 폴더의 색인 비교 (설계서 §11-4 끝단 검증)
#=> 분리 전 exe 와 DLL 을 쓰는 exe 로 같은 폴더를 색인한 뒤, 문서별 청크 수 · 청크 글 · 실패 목록 · OCR 대기열이 같은지 본다.
#    1) index_state.json 의 docs(청크 수 · 내용 해시 · ocr_pending) · failed(사유) · ocr_queue 비교
#    2) rag_index/points.sqlite 의 청크 글(payload)을 문서 · 순서별로 비교
#
# -in: argv[1] = 기준 데이터 폴더 / argv[2] = 새 데이터 폴더
#
# -out: 화면에 차이 · 요약. 종료 코드 0 = 같음, 1 = 다름
# -out: error = 파일이 없으면 예외 전파
#------------------------------------------------------------------
import json, os, sqlite3, sys

def load_state(home):
    return json.load(open(os.path.join(home, "index_state.json"), encoding="utf-8"))

def chunks(home):
    # 문서 경로 → [청크 글...] (id 순서)
    con = sqlite3.connect(os.path.join(home, "rag_index", "points.sqlite"))
    cols = [r[1] for r in con.execute("PRAGMA table_info(points)")]
    out = {}
    for row in con.execute("SELECT * FROM points ORDER BY id"):
        d = dict(zip(cols, row))
        p = json.loads(d.get("payload") or "{}")
        # 문서별로 (청크 글, 절 번호, 청크 해시, 쪽, OCR 표시, 벡터 바이트) — 벡터까지 같아야 검색 결과가 같다
        out.setdefault(p.get("doc_path") or "?", []).append((p.get("text"), p.get("section_id"), p.get("chunk_hash"),
                                                             p.get("page"), p.get("ocr"), d.get("vec")))
    return out

def main():
    a, b = sys.argv[1:3]
    sa, sb = load_state(a), load_state(b)
    diffs = 0
    keys = ["chunks", "content_hash", "ocr_pending", "fingerprint"]
    da, db = sa.get("docs", {}), sb.get("docs", {})
    for k in sorted(set(da) | set(db)):
        x, y = da.get(k), db.get(k)
        if x is None or y is None:
            print("문서 한쪽만:", k); diffs += 1; continue
        for f in keys:
            if x.get(f) != y.get(f):
                print("다름", os.path.basename(k), f, x.get(f), y.get(f)); diffs += 1
    fa = {k: v.get("why") for k, v in sa.get("failed", {}).items()}
    fb = {k: v.get("why") for k, v in sb.get("failed", {}).items()}
    if fa != fb:
        print("실패 목록 다름:", fa, fb); diffs += 1
    qa = {k: v.get("pages") for k, v in sa.get("ocr_queue", {}).items()}
    qb = {k: v.get("pages") for k, v in sb.get("ocr_queue", {}).items()}
    if qa != qb:
        print("OCR 대기열 다름:", qa, qb); diffs += 1
    ca, cb = chunks(a), chunks(b)
    n_same = sum(1 for k in ca if ca[k] == cb.get(k))
    for k in sorted(set(ca) | set(cb)):
        if ca.get(k) != cb.get(k):
            print("청크 다름:", os.path.basename(k), len(ca.get(k, [])), len(cb.get(k, []))); diffs += 1
    print(f"문서 {len(da)}/{len(db)} · 청크 문서 같음 {n_same}/{len(ca)} · 청크 {sum(map(len, ca.values()))}/{sum(map(len, cb.values()))} · 실패 {len(fa)}/{len(fb)} · OCR 대기 {len(qa)}/{len(qb)} · 차이 {diffs}")
    sys.exit(0 if diffs == 0 else 1)

main()
