#!/usr/bin/env bash
#------------------------------------------------------------------
# 리눅스 빌드 — 빌드서버(CentOS 7 계열)에서 libstruct_docparse.so 를 만들고 시험한 뒤 받아 온다
#=> 이 PC(Windows · Git Bash)에서 실행한다. 방식은 DocParse-rust/scripts/linux-build.sh 와 같다.
#    1) scripts/server.env → 접속 정보 파일(SSH_ENV_FILE)에서 서버 · 계정 읽기(비밀번호는 출력 안 함)
#    2) 소스(Cargo.toml · Cargo.lock · crates) + 시험 묶음(tests/samples · 리눅스 런타임 · OCR 모델) 올리기
#    3) 빌드서버에서 cargo test -p sdp-core · cargo build --release -p sdp-ffi -p sdp-cli
#       요구 glibc 확인, 같은 폴더에 런타임을 두고 sdp 로 시험(version · parse · OCR · OCR 끔 · dump)
#    4) libstruct_docparse.so · sdp · 리눅스 dump 결과를 target/linux-x64 로 받아 온다
#    5) Windows 에서 같은 목록을 dump 해 리눅스 결과와 비교(글자 하나까지 같아야 한다)
#    6) scripts/package.py 로 dist/struct_docparse-<버전>-linux-x64.tgz
#
#   사용: bash scripts/linux-build.sh   (Git Bash · PuTTY 의 plink/pscp 필요)
#   설정: scripts/server.env (scripts/server.env.sample 참고, .gitignore 로 막혀 있다)
#------------------------------------------------------------------
set -euo pipefail
cd "$(dirname "$0")/.."

# 실패하면 사유를 남기고 멈춘다
die() { echo "[실패] $*" >&2; exit 1; }
# 단계 머리줄
step() { echo ""; echo "── $* ──"; }

# 빌드서버 접속 정보 · rsh/put/get 함수
source scripts/ssh-env.sh
for f in third_party/pdfium/linux-x64/libpdfium.so third_party/onnxruntime/linux-x64/libonnxruntime.so; do
    [ -f "$f" ] || die "리눅스 런타임이 없습니다: $f (doc/struct_docparse-라이브러리-연동설명서.html 의 '리눅스 런타임' 참고)"
done
OCR_MODELS="${OCR_MODELS:-../SimpleRAG/models/ocr}"
[ -f "$OCR_MODELS/PP-OCRv6_tiny_det.onnx" ] || die "OCR 모델 폴더가 없습니다: $OCR_MODELS (OCR_MODELS 로 지정)"

# 버전 = 크레이트 버전 + 이 PC 의 git 해시
VER_CRATE="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)"/\1/')"
HASH="$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
if [ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ]; then HASH="$HASH-dirty"; fi
VERSION="$VER_CRATE+$HASH"

step "소스 · 시험 묶음 올리기 ($VERSION)"
mkdir -p "$TMP/rt/models/ocr"
cp third_party/pdfium/linux-x64/libpdfium.so third_party/onnxruntime/linux-x64/libonnxruntime.so \
   third_party/onnxruntime/linux-x64/libonnxruntime_providers_shared.so "$TMP/rt/"
cp "$OCR_MODELS"/* "$TMP/rt/models/ocr/"
tar -czf "$TMP/sdpsrc.tgz" Cargo.toml Cargo.lock crates tests/samples tests/samples.txt -C "$TMP" rt \
    || die "소스를 묶지 못했습니다"
rsh "mkdir -p $SDP_BUILD_DIR" || die "빌드 폴더를 만들지 못했습니다"
put "$TMP/sdpsrc.tgz" "$SDP_BUILD_DIR/" || die "소스 전송 실패"

step "빌드서버에서 빌드 · 시험"
rsh "bash -s" <<EOF || die "빌드서버에서 실패했습니다 (위 출력 참고)"
set -e
export PATH="$CARGO_BIN_DIR:\$PATH"
cd "$SDP_BUILD_DIR"
rm -rf crates Cargo.toml Cargo.lock tests rt out
tar xzf sdpsrc.tgz
echo "  \$(uname -sr) · \$(ldd --version | head -1)"
cargo test --release -p sdp-core > test.log 2>&1 || { tail -30 test.log; echo "cargo test 실패 — $SDP_BUILD_DIR/test.log"; exit 1; }
grep 'test result' test.log | head -1 | sed 's/^/  단위 시험: /'
cargo build --release -p sdp-ffi -p sdp-cli > build.log 2>&1 || { tail -30 build.log; echo "cargo build 실패 — $SDP_BUILD_DIR/build.log"; exit 1; }
R=target/release
[ -f \$R/libstruct_docparse.so ] || { echo "빌드 결과가 없습니다: \$R/libstruct_docparse.so"; exit 1; }
# 요구하는 glibc 기호 버전 중 가장 높은 것 — 낮을수록 오래된 리눅스에서도 뜬다
g=\$(objdump -T \$R/libstruct_docparse.so | grep -o 'GLIBC_[0-9.]*' | sort -t. -k2 -n -u | tail -1)
echo "  libstruct_docparse.so \$(stat -c %s \$R/libstruct_docparse.so) B · 최고 요구 glibc: \$g"
# 시험 — 라이브러리 옆에 런타임 · 모델을 둔다(DLL 옆에서 찾는다)
cp -r rt/* \$R/
\$R/sdp version
\$R/sdp parse "tests/samples/보고서_예시.docx" --out out_docx.json && echo "  parse docx (서버 glibc): 성공"
# OCR — onnxruntime 1.22 는 glibc 2.27 이상이 필요하다. 서버가 더 낮으면 rockylinux:8(glibc 2.28) 컨테이너에서 돌린다
RUN=""
NOOCR=""
GV=\$(ldd --version | head -1 | grep -o '[0-9]*\.[0-9]*\$')
if [ "\$(printf '%s\n' 2.27 "\$GV" | sort -V | head -1)" != "2.27" ]; then
    if command -v docker >/dev/null && docker image inspect rockylinux:8 >/dev/null 2>&1; then
        RUN="docker run --rm --user \$(id -u):\$(id -g) -v \$PWD:/w -w /w rockylinux:8"
        echo "  서버 glibc \$GV < 2.27 → OCR 시험은 컨테이너 rockylinux:8 (glibc 2.28)에서"
    else
        NOOCR="--no-ocr"
        echo "  서버 glibc \$GV < 2.27 이고 컨테이너도 없음 → OCR 시험 생략(--no-ocr 로 비교)"
    fi
fi
if [ -z "\$NOOCR" ]; then
    \$RUN \$R/sdp parse tests/samples/스캔_예시.jpg --out out_img.json 2>/dev/null && grep -c '"source": "ocr"' out_img.json | sed 's/^/  OCR 블록 수: /'
fi
\$RUN \$R/sdp parse tests/samples/스캔_예시.jpg --no-ocr > /dev/null 2>&1 && echo "  OCR 끔: 성공(뜻밖)" || echo "  OCR 끔: 그림 거절(정상)"
mkdir -p out
\$RUN \$R/sdp dump tests/samples.txt out/samples \$NOOCR 2>&1 | tail -1 | sed 's/^/  dump: /'
echo "\$NOOCR" > out/NOOCR
tar czf out.tgz out
EOF

step "받아 오기"
mkdir -p target/linux-x64
get "$SDP_BUILD_DIR/target/release/libstruct_docparse.so" target/linux-x64/ || die "libstruct_docparse.so 내려받기 실패"
get "$SDP_BUILD_DIR/target/release/sdp" target/linux-x64/ || die "sdp 내려받기 실패"
get "$SDP_BUILD_DIR/out.tgz" "$TMP/" || die "리눅스 dump 결과 내려받기 실패"
rm -rf work/samples_linux && mkdir -p work && tar xzf "$TMP/out.tgz" -C "$TMP" && mv "$TMP/out/samples" work/samples_linux \
    && cp "$TMP/out/NOOCR" work/samples_linux_NOOCR

step "Windows 결과와 비교"
[ -f target/release/sdp.exe ] || cargo build --release -p sdp-cli
rm -rf work/samples_win
# 리눅스에서 OCR 을 못 돌렸으면 Windows 도 OCR 끔으로 맞춰 비교한다
NOOCR="$(tr -d '\r\n' < work/samples_linux_NOOCR 2>/dev/null || true)"
./target/release/sdp.exe dump tests/samples.txt work/samples_win $NOOCR 2>&1 | tail -1
./target/release/sdp.exe compare work/samples_win work/samples_linux || die "리눅스 결과가 Windows 와 다릅니다"

step "묶기"
python scripts/package.py linux-x64 target/linux-x64/libstruct_docparse.so "$VERSION" target/linux-x64/sdp
