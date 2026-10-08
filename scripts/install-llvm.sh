#!/usr/bin/env bash
# Installs the pinned LLVM release into .llvm/22, the prefix that
# .cargo/config.toml hands to llvm-sys. The system LLVM is left alone.
# Safe to re-run: it does nothing when the pinned version is already there.
set -euo pipefail

version=22.1.8
sha256=df0e1ecf16caf3489a272a5eea4eec9b0d82878f6477fa309504f918a0006384
url="https://github.com/llvm/llvm-project/releases/download/llvmorg-${version}/LLVM-${version}-Linux-X64.tar.xz"

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/.llvm/22"

if [[ "$(cat "$dest/VERSION" 2>/dev/null)" == "$version" ]]; then
    echo "LLVM $version is already in $dest"
    exit 0
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl --fail --location --retry 3 --output "$tmp/llvm.tar.xz" "$url"
echo "$sha256  $tmp/llvm.tar.xz" | sha256sum --check --quiet

rm -rf "$dest"
mkdir -p "$dest"
tar -xJf "$tmp/llvm.tar.xz" -C "$dest" --strip-components=1
# Written last, so an interrupted run is redone on the next call.
echo "$version" >"$dest/VERSION"
echo "Installed LLVM $version in $dest"
