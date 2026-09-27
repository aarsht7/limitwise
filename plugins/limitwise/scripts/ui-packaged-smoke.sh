#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  printf 'usage: %s <packaged-limitwise-binary>\n' "$0" >&2
  exit 2
fi

binary=$1
case $binary in
  /*) ;;
  *) binary=$(CDPATH= cd -- "$(dirname -- "$binary")" && pwd)/$(basename -- "$binary") ;;
esac
[ -x "$binary" ] || { printf 'binary is not executable: %s\n' "$binary" >&2; exit 1; }

temporary=$(mktemp -d "${TMPDIR:-/tmp}/limitwise-ui-smoke.XXXXXX")
server_pid=""
cleanup() {
  if [ -n "$server_pid" ]; then
    kill -TERM "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$temporary"
}
trap cleanup EXIT HUP INT TERM
chmod 700 "$temporary"

LIMITWISE_HOME="$temporary/home" \
LIMITWISE_CODEX_PATH="$(CDPATH= cd -- "$(dirname -- "$0")/../tests/fixtures" && pwd)/fake-codex.sh" \
  "$binary" ui --no-open >"$temporary/url" 2>"$temporary/stderr" &
server_pid=$!

attempt=0
while [ ! -s "$temporary/url" ] && kill -0 "$server_pid" 2>/dev/null; do
  attempt=$((attempt + 1))
  [ "$attempt" -lt 100 ] || { printf '%s\n' 'UI did not publish its launch URL' >&2; exit 1; }
  sleep 0.05
done

launch_url=$(sed -n 's/^LimitWise UI: //p' "$temporary/url")
[ -n "$launch_url" ] || { printf '%s\n' 'UI launch URL is missing' >&2; exit 1; }
origin=${launch_url%%/#*}
bootstrap=${launch_url#*#bootstrap=}
[ "$origin" != "$launch_url" ] || { printf '%s\n' 'UI launch URL has no bootstrap fragment' >&2; exit 1; }
[ -n "$bootstrap" ] || { printf '%s\n' 'UI bootstrap fragment is empty' >&2; exit 1; }

curl -fsS "$origin/" >"$temporary/index.html"
grep -F '<div id="root">' "$temporary/index.html" >/dev/null
curl -fsS "$origin/assets/app.js" >"$temporary/app.js"
curl -fsS "$origin/assets/app.css" >"$temporary/app.css"

bootstrap_json=$(printf '{"bootstrap":"%s"}' "$bootstrap")
curl -fsS \
  -H "Origin: $origin" \
  -H 'Content-Type: application/json' \
  --data "$bootstrap_json" \
  "$origin/api/v1/bootstrap" >"$temporary/bootstrap.json"
bearer=$(sed -n 's/.*"token":"\([a-f0-9][a-f0-9]*\)".*/\1/p' "$temporary/bootstrap.json")
[ "${#bearer}" -eq 64 ] || { printf '%s\n' 'UI bootstrap did not return a 256-bit bearer' >&2; exit 1; }

status=$(curl -sS -o "$temporary/wrong-token.json" -w '%{http_code}' \
  -H "Origin: $origin" \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer wrong' \
  "$origin/api/v1/tasks")
[ "$status" = 401 ] || { printf 'wrong-token request returned HTTP %s\n' "$status" >&2; exit 1; }

status=$(curl -sS -o "$temporary/hostile-origin.json" -w '%{http_code}' \
  -H 'Origin: https://attacker.invalid' \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $bearer" \
  "$origin/api/v1/tasks")
[ "$status" = 403 ] || { printf 'hostile-origin request returned HTTP %s\n' "$status" >&2; exit 1; }

curl -fsS \
  -H "Origin: $origin" \
  -H 'Content-Type: application/json' \
  -H "Authorization: Bearer $bearer" \
  "$origin/api/v1/tasks" >"$temporary/tasks.json"
grep -F '[]' "$temporary/tasks.json" >/dev/null

kill -TERM "$server_pid"
wait "$server_pid"
server_pid=""
if grep -F "$bearer" "$temporary/url" "$temporary/stderr" >/dev/null; then
  printf '%s\n' 'bearer token leaked to UI stdout or stderr' >&2
  exit 1
fi
printf '%s\n' 'LimitWise packaged UI smoke test passed'
