#!/usr/bin/env bash
#------------------------------------------------------------------
# 빌드서버 접속 함수 — linux-build.sh 와 server-run.sh 가 함께 쓴다(source 로 읽는다)
#=> scripts/server.env → 접속 정보 파일(SSH_ENV_FILE)에서 서버 · 계정 · 비밀번호를 읽어
#   rsh(원격 명령) · put(올리기) · get(받기) 함수를 만든다. 비밀번호는 화면에 내지 않는다.
#------------------------------------------------------------------
[ -f scripts/server.env ] || die "scripts/server.env 가 없습니다 — scripts/server.env.sample 을 복사해 채우세요"
# KEY=VALUE 한 줄씩 읽어 환경변수로(주석·빈 줄 건너뜀, 윈도우 줄끝 제거)
while IFS='=' read -r k v; do
    case "$k" in ''|\#*) continue ;; esac
    export "$k=${v%$'\r'}"
done < scripts/server.env
: "${SSH_ENV_FILE:?server.env 에 SSH_ENV_FILE 이 없습니다}"
: "${SDP_BUILD_DIR:?server.env 에 SDP_BUILD_DIR 이 없습니다}"
: "${CARGO_BIN_DIR:?server.env 에 CARGO_BIN_DIR 이 없습니다}"
command -v plink >/dev/null || die "plink 를 찾을 수 없습니다 (PuTTY 설치·PATH 확인)"
command -v pscp  >/dev/null || die "pscp 를 찾을 수 없습니다"
[ -f "$SSH_ENV_FILE" ] || die "접속 정보 파일이 없습니다: $SSH_ENV_FILE"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

step "접속 정보 읽기"
python - "$SSH_ENV_FILE" "$TMP" <<'PY' || die "접속 정보를 읽지 못했습니다"
import io, sys
d = {}
for enc in ("utf-8", "cp949"):
    try:
        lines = io.open(sys.argv[1], encoding=enc).read().splitlines()
        break
    except UnicodeDecodeError:
        continue
for line in lines:
    line = line.strip()
    if "=" in line and not line.startswith("#"):
        k, v = line.split("=", 1)
        d[k.strip()] = v.strip().strip('"').strip("'")
for k in ("SSH_SERVER", "SSH_USER_ID", "SSH_USER_PWD"):
    if not d.get(k):
        raise SystemExit("%s 가 없습니다" % k)
io.open(sys.argv[2] + "/.pw", "w", encoding="utf-8", newline="").write(d["SSH_USER_PWD"])
io.open(sys.argv[2] + "/.host", "w", encoding="utf-8", newline="").write(
    "%s %s\n" % (d["SSH_SERVER"].split(":")[0], d["SSH_USER_ID"]))
PY
read -r HOST USER_ID < "$TMP/.host" || true
HOST="${HOST%$'\r'}"; USER_ID="${USER_ID%$'\r'}"
PW="$(cat "$TMP/.pw")"; PW="${PW%$'\r'}"
echo "  $USER_ID@$HOST (비밀번호는 출력하지 않습니다)"

# 원격 명령 · 파일 보내기 · 받기 (비밀번호를 화면에 남기지 않는다)
rsh() { plink -batch -ssh -l "$USER_ID" -pw "$PW" "$HOST" "$@"; }
put() { pscp -batch -q -pw "$PW" "$1" "$USER_ID@$HOST:$2"; }
get() { pscp -batch -q -pw "$PW" "$USER_ID@$HOST:$1" "$2"; }

