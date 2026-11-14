#!/bin/sh
# Download the OCR models (PaddleOCR PP-OCRv5: text detection and Cyrillic recognition, as ONNX) into src-tauri/ocr/.
# Blankey runs them itself (src-tauri/src/doctypes/ocr.rs); a file is kept only when its checksum is the pinned one.
set -eu
BASE="https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.10.0/onnx/PP-OCRv5"
DEST="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/ocr"
mkdir -p "$DEST"
sum() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
fetch() {
    path="$1"; expected="$2"; file="$DEST/$(basename "$path")"
    if [ -f "$file" ] && [ "$(sum "$file")" = "$expected" ]; then return; fi
    curl -sSfL "$BASE/$path" -o "$file.part"
    if [ "$(sum "$file.part")" != "$expected" ]; then
        rm -f "$file.part"
        echo "checksum mismatch for $path" >&2
        exit 1
    fi
    mv "$file.part" "$file"
}
fetch det/ch_PP-OCRv5_det_mobile.onnx 4d97c44a20d30a81aad087d6a396b08f786c4635742afc391f6621f5c6ae78ae
fetch rec/cyrillic_PP-OCRv5_rec_mobile.onnx 90f761b4bfcce0c8c561c0cb5c887b0971d3ec01c32164bdf7374a35b0982711
ls -la "$DEST"
