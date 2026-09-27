#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  printf '%s\n' 'usage: doctor-nonmutation.sh /path/to/limitwise.sqlite3' >&2
  exit 2
fi

source_database=$1
[ -f "$source_database" ] || {
  printf 'database does not exist: %s\n' "$source_database" >&2
  exit 2
}

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
plugin_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
binary=${LIMITWISE_BINARY:-$plugin_dir/target/debug/limitwise}
fake_codex=$plugin_dir/tests/fixtures/fake-codex.sh
proof_root=$(mktemp -d "${TMPDIR:-/tmp}/limitwise-doctor-nonmutation.XXXXXX")
cleanup() {
  rm -rf "$proof_root"
}
trap cleanup EXIT HUP INT TERM

case $(uname -s) in
  Darwin) data_dir="$proof_root/home/Library/Application Support/LimitWise" ;;
  *) data_dir="$proof_root/data/limitwise" ;;
esac
mkdir -p "$data_dir/logs"
chmod 700 "$proof_root/home" "$data_dir" "$data_dir/logs" 2>/dev/null || true
cp -p "$source_database" "$data_dir/limitwise.sqlite3"
for suffix in -wal -shm; do
  if [ -f "$source_database$suffix" ]; then
    cp -p "$source_database$suffix" "$data_dir/limitwise.sqlite3$suffix"
  fi
done
chmod 600 "$data_dir"/limitwise.sqlite3*

snapshot_tree() {
  destination=$1
  : >"$destination"
  find "$data_dir" -print | LC_ALL=C sort | while IFS= read -r path; do
    if [ "$(uname -s)" = "Darwin" ]; then
      stat -f '%N|%z|%p|%m|%c' "$path"
    else
      stat -c '%n|%s|%f|%Y|%Z' "$path"
    fi
    if [ -f "$path" ]; then
      cksum "$path"
    fi
  done >>"$destination"
}

snapshot_tree "$proof_root/before"
set +e
env \
  LIMITWISE_HOME="$proof_root/home" \
  XDG_DATA_HOME="$proof_root/data" \
  LIMITWISE_CODEX_PATH="$fake_codex" \
  LIMITWISE_DOCTOR_SCENARIO=success \
  "$binary" doctor --json >"$proof_root/report.json"
doctor_status=$?
set -e
[ "$doctor_status" -le 1 ] || exit "$doctor_status"
snapshot_tree "$proof_root/after"
cmp "$proof_root/before" "$proof_root/after"

printf '%s\n' 'LimitWise doctor copied-database non-mutation proof passed'
