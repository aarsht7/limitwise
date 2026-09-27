#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
plugin_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
binary=${LIMITWISE_BINARY:-$plugin_dir/target/debug/limitwise}
fake_codex=$plugin_dir/tests/fixtures/fake-codex.sh
smoke_root=$(mktemp -d "${TMPDIR:-/tmp}/limitwise-doctor-smoke.XXXXXX")
cleanup() {
  rm -rf "$smoke_root"
}
trap cleanup EXIT HUP INT TERM

run_doctor() {
  scenario=${1:-success}
  if [ "$#" -eq 2 ]; then
    env \
      LIMITWISE_HOME="$smoke_root/home" \
      XDG_DATA_HOME="$smoke_root/data" \
      LIMITWISE_CODEX_PATH="$fake_codex" \
      LIMITWISE_DOCTOR_SCENARIO="$scenario" \
      "$binary" doctor "$2"
  else
    env \
      LIMITWISE_HOME="$smoke_root/home" \
      XDG_DATA_HOME="$smoke_root/data" \
      LIMITWISE_CODEX_PATH="$fake_codex" \
      LIMITWISE_DOCTOR_SCENARIO="$scenario" \
      "$binary" doctor
  fi
}

run_doctor success --json >"$smoke_root/first.json"
run_doctor success --json >"$smoke_root/second.json"
sed -E 's/("generated_at": )[0-9]+/\1<TIMESTAMP>/' "$smoke_root/first.json" >"$smoke_root/first.normalized"
sed -E 's/("generated_at": )[0-9]+/\1<TIMESTAMP>/' "$smoke_root/second.json" >"$smoke_root/second.normalized"
cmp "$smoke_root/first.normalized" "$smoke_root/second.normalized"
grep -q '"schema_version": "1"' "$smoke_root/first.json"
grep -q '"overall": "warn"' "$smoke_root/first.json"

run_doctor success >"$smoke_root/human.txt"
grep -q '^LimitWise doctor: WARN$' "$smoke_root/human.txt"
grep -q '^PASS codex.quota:' "$smoke_root/human.txt"

if run_doctor logged_out --json >"$smoke_root/failure.json"; then
  printf '%s\n' 'doctor failure fixture unexpectedly exited 0' >&2
  exit 1
else
  status=$?
  [ "$status" -eq 1 ] || exit "$status"
fi

if env \
  LIMITWISE_HOME="$smoke_root/home" \
  LIMITWISE_CODEX_PATH="$fake_codex" \
  "$binary" doctor --invalid >"$smoke_root/invalid.stdout" 2>"$smoke_root/invalid.stderr"; then
  printf '%s\n' 'doctor invalid-argument fixture unexpectedly exited 0' >&2
  exit 1
else
  status=$?
  [ "$status" -eq 2 ] || exit "$status"
fi

if env -u LIMITWISE_HOME -u HOME -u USERPROFILE \
  LIMITWISE_CODEX_PATH="$fake_codex" \
  "$binary" doctor --json >"$smoke_root/execution.stdout" 2>"$smoke_root/execution.stderr"; then
  printf '%s\n' 'doctor execution-failure fixture unexpectedly exited 0' >&2
  exit 1
else
  status=$?
  [ "$status" -eq 2 ] || exit "$status"
fi

printf '%s\n' 'LimitWise doctor smoke test passed'
