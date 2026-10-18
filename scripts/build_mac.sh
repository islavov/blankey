#!/bin/sh
# Release build on macOS: Blankey.app (PDFium in Contents/Frameworks) and a DMG, copied to dist/.
# Needs: rustup, cargo install tauri-cli --version "^2", cargo install cargo-about.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[ -f src-tauri/pdfium/libpdfium.dylib ] || scripts/fetch-pdfium.sh
if [ -z "${BLANKEY_TELEMETRY_KEY:-}" ]; then
    echo "warning: BLANKEY_TELEMETRY_KEY is not set; this build sends anonymous statistics with the development key, which the official relay ignores" >&2
fi
scripts/licenses.sh
scripts/stage-ui.sh
cd src-tauri
cargo tauri build --config tauri.release.conf.json "$@"
mkdir -p "$ROOT/dist"
cp target/release/bundle/dmg/*.dmg "$ROOT/dist/"
cp THIRD_PARTY_LICENSES.html "$ROOT/dist/"
ls -lh "$ROOT/dist"
