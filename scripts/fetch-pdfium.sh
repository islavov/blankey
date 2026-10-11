#!/bin/sh
# Download the PDFium shared library (rendering and text extraction) into src-tauri/pdfium/.
set -eu
RELEASE="${PDFIUM_RELEASE:-chromium/8086}"
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) ASSET=pdfium-mac-arm64.tgz ;;
  Darwin-x86_64) ASSET=pdfium-mac-x64.tgz ;;
  Linux-x86_64) ASSET=pdfium-linux-x64.tgz ;;
  Linux-aarch64) ASSET=pdfium-linux-arm64.tgz ;;
  *) echo "unsupported platform" >&2; exit 1 ;;
esac
DEST="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/pdfium"
mkdir -p "$DEST"
TMP="$(mktemp -d)"
curl -sSfL "https://github.com/bblanchon/pdfium-binaries/releases/download/$RELEASE/$ASSET" | tar -xz -C "$TMP"
cp "$TMP"/lib/libpdfium.* "$DEST/"
cp "$TMP"/LICENSE "$DEST/LICENSE" 2>/dev/null || true
rm -rf "$TMP"
ls -la "$DEST"
