#!/bin/sh
# Release build on macOS: Blankey.app (PDFium in Contents/Frameworks) and a DMG, copied to dist/. With
# TAURI_SIGNING_PRIVATE_KEY set, also the signed Blankey.app.tar.gz the updater installs (scripts/updater-manifest.sh).
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
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    set -- --config '{"bundle":{"createUpdaterArtifacts":true}}' "$@"
fi
cargo tauri build --config tauri.release.conf.json "$@"
mkdir -p "$ROOT/dist"
cp target/release/bundle/dmg/*.dmg "$ROOT/dist/"
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
    cp target/release/bundle/macos/Blankey.app.tar.gz "$ROOT/dist/Blankey_aarch64.app.tar.gz"
    cp target/release/bundle/macos/Blankey.app.tar.gz.sig "$ROOT/dist/Blankey_aarch64.app.tar.gz.sig"
fi
cp THIRD_PARTY_LICENSES.html "$ROOT/dist/"
ls -lh "$ROOT/dist"
