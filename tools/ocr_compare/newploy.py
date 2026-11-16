"""Newploy AI Desktop 에 시험셋을 넣고, 진행을 보고, 결과를 가져온다.

먼저 앱을 디버그 포트로 띄운다(앱이 이미 떠 있으면 끄고 다시):
  & "$env:LOCALAPPDATA\\Programs\\Newploy AI Desktop\\Newploy AI Desktop.exe" --remote-debugging-port=9333

사용:
  python tools/ocr_compare/newploy.py add      # 'OCR비교' 폴더에 scanNN.pdf 넣기 (대기열에 올라감)
  python tools/ocr_compare/newploy.py status   # 문서별 상태 (모두 끝나면 종료 코드 0)
  python tools/ocr_compare/newploy.py fetch    # work\\<id>\\result\\document_result.json → newploy/ 복사

앱 화면에서 끌어다 놓기를 할 때 앱이 부르는 것과 같은 window.newploy.library.addPaths 를 쓴다.
암호화된 모델을 앱 밖에서 돌리지 않는다.
"""
import json, os, shutil, sys

from playwright.sync_api import sync_playwright

from common import CDP_URL, NP_DIR, NP_WORK, SET_DIR

FOLDER = "OCR비교"


#------------------------------------------------------------------
# 시험셋 PDF 절대 경로 목록
#=> Newploy 는 원본 경로(sourcePath)로 파일을 구분하므로 절대 경로로 넘긴다.
#
# -in: 없음
#
# -out: paths = scanNN.pdf 절대 경로 목록(이름순)
# -out: error = 없음 (없으면 빈 목록)
#------------------------------------------------------------------
def set_paths():
    return [str(p.resolve()) for p in sorted(SET_DIR.glob("*.pdf"))]


#------------------------------------------------------------------
# Newploy 화면(렌더러)에서 JS 실행
#=> CDP 로 앱 창에 붙어 window.newploy.* 다리를 부른다.
#    1) playwright 로 CDP 주소에 연결
#    2) 첫 창에서 js 를 arg 와 함께 실행하고 결과를 돌려준다
#
# -in: js  = 실행할 JS 함수 문자열 (async 가능)
# -in: arg = JS 함수에 넘길 값 (기본 None)
#
# -out: result = JS 함수 반환값 (JSON 으로 바뀐 값)
# -out: error = 앱이 디버그 포트로 안 떠 있으면 playwright 연결 예외
#------------------------------------------------------------------
def run_js(js, arg=None):
    with sync_playwright() as p:
        b = p.chromium.connect_over_cdp(CDP_URL)
        return b.contexts[0].pages[0].evaluate(js, arg)


#------------------------------------------------------------------
# 시험셋을 Newploy 라이브러리에 넣기
#=> 'OCR비교' 폴더가 없으면 만들고 scanNN.pdf 를 넣는다. 넣으면 바로 OCR 대기열에 오른다.
#
# -in: 없음
#
# -out: 없음 (넣은 파일 id·상태 출력)
# -out: error = 거절된 파일은 rejected 로 출력, 연결 실패는 예외 전파
#------------------------------------------------------------------
def add():
    r = run_js("""async ([paths, name]) => {
      let st = await window.newploy.library.get();
      let f = st.folders.find(x => x.name === name);
      if (!f) { st = await window.newploy.library.createFolder(name); f = st.folders.find(x => x.name === name); }
      const res = await window.newploy.library.addPaths(f.id, paths);
      return {rejected: res.rejected, files: res.state.files.filter(x => paths.includes(x.sourcePath)).map(x => [x.id, x.status])};
    }""", [set_paths(), FOLDER])
    print(json.dumps(r, ensure_ascii=False, indent=1))


#------------------------------------------------------------------
# 시험셋 처리 상태 보기
#=> 라이브러리에서 시험셋 파일의 status(queued/processing/completed/completed_with_errors)를 찍는다.
#
# -in: 없음
#
# -out: code = 0 이면 모두 끝남, 1 이면 아직 처리 중 (sys.exit 로 넘김)
# -out: error = 연결 실패는 예외 전파
#------------------------------------------------------------------
def status():
    paths = set_paths()
    r = run_js("async (paths) => { const st = await window.newploy.library.get();"
               " return st.files.filter(x => paths.includes(x.sourcePath)).map(x => [x.sourcePath, x.status]); }", paths)
    for sp, st in r:
        print(os.path.basename(sp), st)
    # 대기·처리 중인 파일이 하나도 없으면 끝난 것
    return 0 if r and all(st not in ("queued", "processing") for _, st in r) else 1


#------------------------------------------------------------------
# Newploy 결과 가져오기
#=> work 폴더의 document_result.json 중 원본 경로가 시험셋인 것을 newploy/scanNN.json 으로 복사한다.
#   (앱 데이터가 지워져도 채점을 다시 할 수 있게)
#
# -in: 없음
#
# -out: 없음 (복사한 파일 이름 출력)
# -out: error = 깨진 JSON 은 건너뜀
#------------------------------------------------------------------
def fetch():
    NP_DIR.mkdir(parents=True, exist_ok=True)
    want = {os.path.normcase(p) for p in set_paths()}
    for f in NP_WORK.glob("*/result/document_result.json"):
        try:
            d = json.load(open(f, encoding="utf-8"))
        except Exception:
            continue
        sp = d.get("source_path") or ""
        if os.path.normcase(sp) in want:
            shutil.copy(f, NP_DIR / (os.path.splitext(os.path.basename(sp))[0] + ".json"))
            print(os.path.basename(sp), d.get("status"))


if __name__ == "__main__":
    cmd = sys.argv[1] if len(sys.argv) > 1 else "status"
    if cmd == "add":
        add()
    elif cmd == "fetch":
        fetch()
    else:
        sys.exit(status())
