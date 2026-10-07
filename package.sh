#!/usr/bin/env bash
# Build the Doorbell widget from source and package it for the Deck, into
# prebuilt/ — the package install.sh puts on the Deck, so that people can
# install without Rust or the SDK. Run after changing the widget, then commit
# prebuilt/.
#
# Usage: ./package.sh
set -euo pipefail

WIDGET=doorbell
HERE="$(cd "$(dirname "$0")" && pwd)"
SDK="$HERE/bmc-sdk"
# Homebrew's rustup is keg-only, so it may not be on PATH.
export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH"

if [ ! -f "$SDK/Cargo.toml" ]; then
	echo "The SDK is missing; run: git submodule update --init" >&2
	exit 1
fi

manifest="$HERE/$WIDGET/manifest.json"
field() { python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))[sys.argv[2]])' "$manifest" "$1"; }
version=$(field version)
icon=$(field icon)
wasm_name="$(tr - _ <<<"$WIDGET").wasm"

echo "==> Building $WIDGET $version"
cargo build --manifest-path "$HERE/Cargo.toml" -p "$WIDGET" --release --target wasm32-unknown-unknown
release="$HERE/target/wasm32-unknown-unknown/release"
assets_tool="$SDK/target/debug/bmc-wasm-assets"
if [ ! -x "$assets_tool" ]; then
	echo "==> Building the SDK's packaging tool (first run only)"
	cargo build --manifest-path "$SDK/Cargo.toml" -p bmc-wasm-assets
fi

echo "==> Packaging"
work="$HERE/.tmp/package/$WIDGET"
package="bmc-widget-$WIDGET"
base="$work/$package/lib/bmc-widgets/$WIDGET"
rm -rf "$work"
mkdir -p "$work/stage" "$base/bin" "$base/lib/wasm" "$base/lib/assets"
# Split compiled assets out of the wasm into their own files, as the firmware build does.
"$assets_tool" extract \
	--input "$release/$wasm_name" \
	--wasm-output "$work/stage/$wasm_name" \
	--asset-root "$work/stage/assets" \
	--artifact-root "$release/deps"
"$assets_tool" verify-stripped --input "$work/stage/$wasm_name"
cp "$work/stage/$wasm_name" "$base/lib/wasm/$WIDGET.wasm"
if [ -d "$work/stage/assets" ]; then
	cp -R "$work/stage/assets/." "$base/lib/assets/"
fi
cp "$manifest" "$base/manifest.json"
mkdir -p "$(dirname "$base/$icon")"
cp "$HERE/$WIDGET/$icon" "$base/$icon"
# The firmware's wrappers name their own store path; this one is added to the
# store by content, so it finds its files relative to itself instead.
cat >"$base/bin/$WIDGET" <<EOF
#!/bin/sh
here=\$(dirname "\$(readlink -f "\$0")")
exec /run/current-profile/bin/bmc-wasm-thin-v0 \\
  --wasm "\$here/../lib/wasm/$WIDGET.wasm" \\
  --asset-root "\$here/../lib/assets" \\
  "\$@"
EOF
chmod 755 "$base/bin/$WIDGET"

echo "==> Writing prebuilt/"
mkdir -p "$HERE/prebuilt"
rm -f "$HERE"/prebuilt/$WIDGET-widget-*.tar.gz
out="$HERE/prebuilt/$WIDGET-widget-$version.tar.gz"
# Reproducible: fixed owner and dates, sorted, no names from this computer.
python3 - "$work" "$package" "$out" <<'PY'
import gzip, io, os, sys, tarfile
work, package, out = sys.argv[1:]
buf = io.BytesIO()
with tarfile.open(fileobj=buf, mode='w', format=tarfile.USTAR_FORMAT) as tar:
    paths = [package] + sorted(os.path.join(dp, n)[len(work) + 1:]
                               for dp, dirs, files in os.walk(os.path.join(work, package))
                               for n in dirs + files)
    for rel in paths:
        info = tar.gettarinfo(os.path.join(work, rel), arcname=rel)
        info.uid = info.gid = 0
        info.uname = info.gname = 'root'
        info.mtime = 0
        if info.isfile():
            with open(os.path.join(work, rel), 'rb') as f:
                tar.addfile(info, f)
        else:
            tar.addfile(info)
with open(out, 'wb') as f, gzip.GzipFile(fileobj=f, mode='wb', mtime=0, filename='') as gz:
    gz.write(buf.getvalue())
PY
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
( cd "$HERE/prebuilt" && sha256 "$(basename "$out")" > SHA256SUMS )
echo "==> $(basename "$out") ($(wc -c < "$out" | tr -d ' ') bytes). Commit prebuilt/ to publish it."
