#!/bin/sh
set -eu

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <target-triple> <output-directory>" >&2
  exit 2
fi

target=$1
output_dir=$2
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
binary="$root/target/$target/release/limitwise"

test -x "$binary"
mkdir -p "$output_dir"
output_abs=$(CDPATH= cd -- "$output_dir" && pwd)
staging=$(mktemp -d "$output_abs/.limitwise-package.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM

mkdir -p "$staging/limitwise/bin" "$staging/limitwise/scripts"
cp "$binary" "$staging/limitwise/bin/limitwise"
cp "$root/scripts/launch-limitwise" "$staging/limitwise/scripts/launch-limitwise"
cp -R "$root/.codex-plugin" "$root/.mcp.json" "$root/skills" "$root/assets" "$root/docs" "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$staging/limitwise/"
chmod 0755 "$staging/limitwise/bin/limitwise" "$staging/limitwise/scripts/launch-limitwise"
tar -C "$staging" -czf "$output_abs/limitwise-$target.tar.gz" limitwise
