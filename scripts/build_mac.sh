#!/bin/sh
# Release build on macOS: Blankey.app (PDFium in Contents/Frameworks) and a DMG, copied to dist/.
# Needs: rustup, cargo install tauri-cli --version "^2", cargo install cargo-about.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[ -f src-tauri/pdfium/libpdfium.dylib ] || scripts/fetch-pdfium.sh
scripts/licenses.sh
cd src-tauri
cargo tauri build --config tauri.release.conf.json "$@"
mkdir -p "$ROOT/dist"
cp target/release/bundle/dmg/*.dmg "$ROOT/dist/"
cp THIRD_PARTY_LICENSES.html "$ROOT/dist/"
ls -lh "$ROOT/dist"
