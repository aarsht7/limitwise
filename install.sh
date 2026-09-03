#!/bin/sh
set -eu

repository="aarsht7/limitwise"
release_base="https://github.com/$repository/releases/latest/download"

fail() {
  printf 'LimitWise install failed: %s\n' "$*" >&2
  exit 1
}

confirm() {
  question=$1
  printf '%s [y/N] ' "$question" >/dev/tty 2>/dev/null || return 1
  IFS= read -r answer </dev/tty 2>/dev/null || return 1
  case $answer in
    y|Y|yes|YES|Yes) return 0 ;;
    *) return 1 ;;
  esac
}

download() {
  url=$1
  destination=$2
  curl -fL --retry 3 --proto '=https' --tlsv1.2 -o "$destination" "$url"
}

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    fail "sha256sum or shasum is required"
  fi
}

command -v codex >/dev/null 2>&1 || fail "Codex is not installed or is not on PATH"
codex login status >/dev/null 2>&1 || fail "Codex is not signed in; run 'codex login' first"
command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

os=$(uname -s)
arch=$(uname -m)
tested=false

case "$os:$arch" in
  Linux:x86_64|Linux:amd64)
    target="x86_64-unknown-linux-gnu"
    install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/limitwise/bin"
    tested=true
    ;;
  Linux:aarch64|Linux:arm64)
    target="aarch64-unknown-linux-gnu"
    install_dir="${XDG_DATA_HOME:-$HOME/.local/share}/limitwise/bin"
    ;;
  Darwin:x86_64|Darwin:amd64)
    target="x86_64-apple-darwin"
    install_dir="$HOME/Library/Application Support/LimitWise/bin"
    ;;
  Darwin:arm64|Darwin:aarch64)
    target="aarch64-apple-darwin"
    install_dir="$HOME/Library/Application Support/LimitWise/bin"
    ;;
  *)
    printf 'WARNING: LimitWise has only been tested on Linux x86-64. macOS, Apple Silicon, and other architectures are untested.\n' >&2
    confirm "Continue platform detection for $os $arch?" || fail "cancelled"
    fail "no prebuilt binary is published for $os $arch"
    ;;
esac

if [ "$tested" != true ]; then
  printf 'WARNING: LimitWise has only been tested on Linux x86-64. macOS, Apple Silicon, and other architectures are untested.\n' >&2
  confirm "Install untested build for $os $arch?" || fail "cancelled"
fi

archive_name="limitwise-$target.tar.gz"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/limitwise-install.XXXXXX")
staged_binary=""
cleanup() {
  rm -rf "$temporary_dir"
  if [ -n "$staged_binary" ]; then
    rm -f "$staged_binary"
  fi
}
trap cleanup EXIT HUP INT TERM
archive="$temporary_dir/$archive_name"
checksums="$temporary_dir/SHA256SUMS"

printf 'Downloading LimitWise for %s...\n' "$target"
download "$release_base/$archive_name" "$archive"
download "$release_base/SHA256SUMS" "$checksums"

expected=$(awk -v name="$archive_name" '$2 == name || $2 == ("*" name) { print $1; exit }' "$checksums")
[ -n "$expected" ] || fail "checksum entry for $archive_name is missing"
actual=$(sha256_file "$archive")
[ "$actual" = "$expected" ] || fail "SHA-256 checksum mismatch for $archive_name"

tar -xzf "$archive" -C "$temporary_dir" "limitwise/bin/limitwise"
[ -f "$temporary_dir/limitwise/bin/limitwise" ] || fail "release archive does not contain the LimitWise binary"

mkdir -p "$install_dir"
chmod 700 "$install_dir"
installed_binary="$install_dir/limitwise"
staged_binary=$(mktemp "$install_dir/.limitwise-install.XXXXXX")
cp "$temporary_dir/limitwise/bin/limitwise" "$staged_binary"
chmod 700 "$staged_binary"
mv -f "$staged_binary" "$installed_binary"
staged_binary=""

printf 'Adding LimitWise marketplace...\n'
if ! codex plugin marketplace add "$repository"; then
  printf 'Marketplace may already exist; refreshing it...\n'
  codex plugin marketplace upgrade limitwise
fi
codex plugin add limitwise@limitwise

if confirm "Install and start the LimitWise background service?"; then
  "$installed_binary" setup
else
  printf 'Background service not installed. Run "%s setup" later to enable scheduled tasks.\n' "$installed_binary"
fi

printf 'LimitWise installed. Open a new Codex conversation before using it.\n'
