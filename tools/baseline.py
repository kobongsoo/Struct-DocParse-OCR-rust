# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 분리 전 기준 블록 JSON 만들기
#=> 분리 전 SimpleRAG(simplerag-rs.exe)의 `extract --blocks --dump` 로 시험 문서마다 *.blocks.json 을 남긴다.
#   DLL 판 결과를 이것과 글자 하나까지 비교한다(설계서 §11).
#    1) 목록 파일(한 줄에 경로 하나)을 읽는다
#    2) 문서마다 exe 를 불러 <out>/<번호>/<이름>.blocks.json 을 만든다
#    3) 번호 ↔ 원본 경로 표(index.json)를 남긴다
#
# -in: argv[1] = simplerag-rs.exe 경로
# -in: argv[2] = 문서 목록 파일
# -in: argv[3] = 결과 폴더
# -in: argv[4] = SIMPLERAG_HOME 으로 쓸 폴더(OCR 캐시 위치)
#
# -out: 결과 폴더에 JSON 들 · index.json
# -out: error = exe 실패는 index.json 에 rc 로 남기고 계속
#------------------------------------------------------------------
import json, os, subprocess, sys

def main():
    exe, listfile, out, home = sys.argv[1:5]
    os.makedirs(out, exist_ok=True)
    paths = [l.strip() for l in open(listfile, encoding="utf-8") if l.strip()]
    env = dict(os.environ, SIMPLERAG_HOME=home, SIMPLERAG_OCR_DIR=os.environ.get("SDP_OCR_DIR", ""))
    index = {}
    for i, p in enumerate(paths):
        d = os.path.join(out, f"{i:04d}")
        os.makedirs(d, exist_ok=True)
        # extract --blocks 는 미리보기 뒤 청킹까지 하는데, 덤프는 그 전에 써진다
        r = subprocess.run([exe, "extract", p, "--blocks", "--dump", d, "--chars", "0"], env=env,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        index[f"{i:04d}"] = {"path": p, "rc": r.returncode, "err": r.stderr.decode("utf-8", "replace")[-300:]}
        print(f"[{i+1}/{len(paths)}] rc={r.returncode} {os.path.basename(p)}", flush=True)
    json.dump(index, open(os.path.join(out, "index.json"), "w", encoding="utf-8"), ensure_ascii=False, indent=1)

if __name__ == "__main__":
    main()
