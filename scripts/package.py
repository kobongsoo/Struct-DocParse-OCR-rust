# -*- coding: utf-8 -*-
#------------------------------------------------------------------
# 배포 묶음 만들기 — dist/struct_docparse-<버전>-<플랫폼>.(zip|tgz)
#=> 다른 프로그램이 받아 바로 쓸 수 있게 라이브러리 · 런타임 · OCR 모델 · 헤더 · 바인딩 · 예제 · 설명서를 한 폴더로 묶는다.
#    1) lib/ 에 라이브러리 + sdp(시험 CLI) + pdfium + onnxruntime + models/ocr — 라이브러리는 "자기 옆"에서 런타임을 찾는다
#    2) include/ · bindings/ · examples/ · doc/ · README.md
#    3) VERSION · SHA256SUMS
#    4) windows-x64 는 zip, linux-x64 는 tgz(실행 권한 유지)
#
#   사용: python scripts/package.py <windows-x64|linux-x64> <라이브러리 파일> <버전> [sdp 실행 파일]
#------------------------------------------------------------------
import hashlib
import os
import shutil
import sys
import tarfile
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OCR_MODELS = os.environ.get("OCR_MODELS", os.path.join(ROOT, "models", "ocr"))  # 저장소에 든 OCR 모델(Apache-2.0)

# 플랫폼별 런타임 파일(원본 경로, 묶음 안 이름)
RUNTIME = {
    "windows-x64": [
        (os.path.join(ROOT, "target", "release", "pdfium.dll"), "pdfium.dll"),
        (os.path.join(ROOT, "target", "release", "onnxruntime.dll"), "onnxruntime.dll"),
        (os.path.join(ROOT, "target", "release", "onnxruntime_providers_shared.dll"), "onnxruntime_providers_shared.dll"),
    ],
    "linux-x64": [
        (os.path.join(ROOT, "third_party", "pdfium", "linux-x64", "libpdfium.so"), "libpdfium.so"),
        (os.path.join(ROOT, "third_party", "onnxruntime", "linux-x64", "libonnxruntime.so"), "libonnxruntime.so"),
        (os.path.join(ROOT, "third_party", "onnxruntime", "linux-x64", "libonnxruntime_providers_shared.so"),
         "libonnxruntime_providers_shared.so"),
    ],
}


#------------------------------------------------------------------
# 폴더 통째 복사(있으면 지우고) — __pycache__ · 빌드 찌꺼기는 뺀다
#
# -in: src = 원본 폴더 / dst = 대상 폴더
#
# -out: 없음
# -out: error = 원본이 없으면 조용히 넘어간다
#------------------------------------------------------------------
def copy_tree(src, dst):
    if not os.path.isdir(src):
        return
    shutil.copytree(src, dst, ignore=shutil.ignore_patterns("__pycache__", "*.pyc", "target", "out", "*.class", "*.exe", "*.obj", "ex_c"))


#------------------------------------------------------------------
# 파일 SHA-256 (16진수)
#
# -in: path = 파일
#
# -out: 64자 16진수
# -out: error = 읽기 실패 시 예외 전파
#------------------------------------------------------------------
def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for b in iter(lambda: f.read(1 << 20), b""):
            h.update(b)
    return h.hexdigest()


#------------------------------------------------------------------
# 묶음 만들기 (핵심)
#
# -in: plat    = "windows-x64" | "linux-x64"
# -in: libfile = 빌드한 라이브러리(struct_docparse.dll · libstruct_docparse.so)
# -in: version = "0.1.0+해시"
# -in: sdp     = 시험 CLI 실행 파일(없으면 None)
#
# -out: 만든 압축 파일 경로
# -out: error = 런타임 · 모델이 없으면 SystemExit
#------------------------------------------------------------------
def package(plat, libfile, version, sdp=None):
    if plat not in RUNTIME:
        raise SystemExit("모르는 플랫폼: %s (windows-x64 · linux-x64)" % plat)
    tag = version.replace("+", "-")
    name = "struct_docparse-%s-%s" % (tag, plat)
    out = os.path.join(ROOT, "dist", name)
    if os.path.isdir(out):
        shutil.rmtree(out)
    lib = os.path.join(out, "lib")
    os.makedirs(os.path.join(lib, "models", "ocr"))
    # 1) 라이브러리 · CLI · 런타임 · 모델 — 모두 lib/ 한곳에(라이브러리가 자기 옆에서 찾는다)
    shutil.copy2(libfile, lib)
    # Windows C/C++ 에서 링크할 때 쓰는 가져오기 라이브러리(struct_docparse.dll.lib)
    if plat == "windows-x64" and os.path.isfile(libfile + ".lib"):
        shutil.copy2(libfile + ".lib", lib)
    if sdp:
        shutil.copy2(sdp, lib)
    for src, dst in RUNTIME[plat]:
        if not os.path.isfile(src):
            raise SystemExit("런타임 파일이 없습니다: %s" % src)
        shutil.copy2(src, os.path.join(lib, dst))
    # 모델 세 파일 + 라이선스 · 출처 안내(Apache-2.0 은 라이선스 문서를 함께 배포해야 한다)
    for f in ("PP-OCRv6_tiny_det.onnx", "korean_PP-OCRv5_mobile_rec.onnx", "korean_PP-OCRv5_mobile_rec.dict.txt", "LICENSE", "README.md"):
        src = os.path.join(OCR_MODELS, f)
        if not os.path.isfile(src):
            raise SystemExit("OCR 모델이 없습니다: %s (OCR_MODELS 로 지정)" % src)
        shutil.copy2(src, os.path.join(lib, "models", "ocr", f))
    # 2) 헤더 · 바인딩 · 예제 · 설명서
    for d in ("include", "bindings", "examples", "doc"):
        copy_tree(os.path.join(ROOT, d), os.path.join(out, d))
    shutil.copy2(os.path.join(ROOT, "README.md"), out)
    # 3) 판 · 체크섬
    with open(os.path.join(out, "VERSION"), "w", encoding="utf-8", newline="\n") as f:
        f.write(version + "\n")
    lines = []
    for base, _, files in os.walk(lib):
        for fn in sorted(files):
            p = os.path.join(base, fn)
            lines.append("%s  %s" % (sha256(p), os.path.relpath(p, out).replace("\\", "/")))
    with open(os.path.join(out, "SHA256SUMS"), "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(sorted(lines, key=lambda l: l[66:])) + "\n")
    # 4) 압축 — 리눅스는 실행 권한(sdp · .so)을 살린다
    if plat == "linux-x64":
        arc = out + ".tgz"
        with tarfile.open(arc, "w:gz") as t:
            # 실행 파일 · 공유 라이브러리는 755, 나머지 644
            def mode(ti):
                ti.mode = 0o755 if (ti.isdir() or ti.name.endswith(".so") or ti.name.endswith("/sdp")) else 0o644
                ti.uid = ti.gid = 0
                ti.uname = ti.gname = "root"
                return ti
            t.add(out, arcname=name, filter=mode)
    else:
        arc = out + ".zip"
        with zipfile.ZipFile(arc, "w", zipfile.ZIP_DEFLATED) as z:
            for base, _, files in os.walk(out):
                for fn in files:
                    p = os.path.join(base, fn)
                    z.write(p, os.path.join(name, os.path.relpath(p, out)))
    print("묶음: %s (%.1f MB)" % (arc, os.path.getsize(arc) / 1048576))
    return arc


if __name__ == "__main__":
    if len(sys.argv) < 4:
        raise SystemExit("사용: python scripts/package.py <windows-x64|linux-x64> <라이브러리 파일> <버전> [sdp 실행 파일]")
    package(sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4] if len(sys.argv) > 4 else None)
