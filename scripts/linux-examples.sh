#!/usr/bin/env bash
#------------------------------------------------------------------
# 언어별 예제를 리눅스(빌드서버)에서 돌려 보기 — linux-build.sh 다음에 실행
#=> 빌드서버의 $SDP_BUILD_DIR/target/release 에 이미 libstruct_docparse.so · 런타임 · 모델이 있어야 한다.
#    1) 바인딩 · 예제 · 헤더 · (Windows 에서 --release 8 로 컴파일한) 자바 클래스 · JNA 올리기
#    2) C(gcc) · Python · Rust · Java 예제를 서버에서 실행(서버 glibc 2.17 — OCR 없는 문서)
#    3) C 예제를 rockylinux:8 컨테이너에서 한 번 더(OCR 포함 — 그림 파일)
#
#   사용: bash scripts/linux-examples.sh
#------------------------------------------------------------------
set -euo pipefail
cd "$(dirname "$0")/.."
die() { echo "[실패] $*" >&2; exit 1; }
step() { echo ""; echo "── $* ──"; }
source scripts/ssh-env.sh

step "자바 예제를 자바 8 용으로 컴파일"
J=third_party/java/jna-5.14.0.jar
rm -rf "$TMP/jout" && mkdir -p "$TMP/jout"
javac --release 8 -encoding UTF-8 -cp "$J" -d "$TMP/jout" bindings/java/structdocparse/*.java examples/java/Example.java \
    || die "javac 실패"

step "예제 올리기"
tar -czf "$TMP/ex.tgz" include bindings examples/c examples/python examples/rust/Cargo.toml examples/rust/src "$J" \
    -C "$TMP" jout || die "묶지 못했습니다"
put "$TMP/ex.tgz" "$SDP_BUILD_DIR/" || die "전송 실패"

step "빌드서버에서 실행"
rsh "bash -s" <<EOF || die "예제 실행 실패 (위 출력 참고)"
set -e
export PATH="$CARGO_BIN_DIR:\$PATH"
cd "$SDP_BUILD_DIR"
rm -rf include bindings examples jout third_party
tar xzf ex.tgz
R=\$PWD/target/release
export SDP_LIB=\$R/libstruct_docparse.so
DOCX="tests/samples/보고서_예시.docx"
PPTX="tests/samples/발표_예시.pptx"
echo "[C]"
# rpath 는 실행 파일 기준 상대 위치(ORIGIN) — 컨테이너 안(/w)에서도 같은 자리를 찾는다
gcc -std=c99 -I include examples/c/example.c -L \$R -lstruct_docparse -Wl,-rpath,'\$ORIGIN/target/release' -o ex_c
./ex_c "\$DOCX" | cut -c1-150
echo "[Python]"
python3 examples/python/example.py "\$DOCX" "\$PPTX" 2>&1
echo "[Rust]"
(cd examples/rust && cargo run --release --quiet -- "../../\$DOCX" 2>&1 | tail -5)
echo "[Java 8 + JNA]"
java -cp "third_party/java/jna-5.14.0.jar:jout" -Dsdp.lib=\$SDP_LIB Example "\$PPTX" 2>&1 | head -6
echo "[C · rockylinux:8 컨테이너 — OCR]"
docker run --rm --user \$(id -u):\$(id -g) -v \$PWD:/w -w /w rockylinux:8 ./ex_c tests/samples/스캔_예시.jpg | cut -c1-150
EOF
