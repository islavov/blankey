#!/bin/sh
# Copy the Python app's data (vault + templates) into the preview data dir, so the mock-up shows real data
# without ever writing to the original.
set -eu
SRC="$HOME/Library/Application Support/Blankey"
DST="${BLANKEY_DATA_DIR:-$HOME/Library/Application Support/Blankey Preview}"
if [ -e "$DST/vault.db" ]; then
  echo "$DST already has a vault; remove it first" >&2
  exit 1
fi
mkdir -p "$DST"
cp -R "$SRC/templates" "$DST/"
cp "$SRC"/vault.db* "$DST/"
cp "$SRC/config.toml" "$DST/" 2>/dev/null || true
echo "Copied to $DST"
