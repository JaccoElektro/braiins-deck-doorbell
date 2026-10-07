#!/usr/bin/env bash
# Check that prebuilt/ matches the source, so what people install without a
# build setup is what this repository says it is.
#
#   ./check.sh           versions agree (manifest, Cargo.toml, prebuilt/, SHA256SUMS)
#   ./check.sh --build   also rebuild the package: it must come out byte for byte the same
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
cd "$HERE"
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
fail() { echo "check: $*" >&2; exit 1; }

manifest=$(python3 -c 'import json; print(json.load(open("doorbell/manifest.json"))["version"])')
cargo=$(sed -n 's/^version = "\(.*\)"/\1/p' doorbell/Cargo.toml | head -n 1)
pkgs=(prebuilt/doorbell-widget-*.tar.gz)
[ "${#pkgs[@]}" = 1 ] && [ -f "${pkgs[0]}" ] || fail "expected exactly one package in prebuilt/"
pkg=$(basename "${pkgs[0]}" .tar.gz); pkg=${pkg#doorbell-widget-}
[ "$manifest" = "$cargo" ] || fail "doorbell/manifest.json says $manifest, doorbell/Cargo.toml says $cargo"
[ "$manifest" = "$pkg" ] || fail "the source is $manifest but prebuilt/ holds $pkg — run ./package.sh"
( cd prebuilt && sha256 -c --quiet SHA256SUMS ) || fail "prebuilt/ does not match prebuilt/SHA256SUMS"
grep -q "doorbell-widget-$pkg.tar.gz" prebuilt/SHA256SUMS || fail "SHA256SUMS does not list the package"
echo "check: versions agree ($manifest)"

if [ "${1:-}" = --build ]; then
	before=$(cat prebuilt/SHA256SUMS)
	./package.sh > /dev/null
	[ "$(cat prebuilt/SHA256SUMS)" = "$before" ] || fail "a fresh build differs from prebuilt/ — commit the new prebuilt/"
	echo "check: a fresh build gives the same package"
fi
