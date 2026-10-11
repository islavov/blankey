#!/bin/sh
# Copies ui/ without its tests to src-tauri/target/ui, the frontend that release builds package
# (tauri.release.conf.json). Debug builds serve ui/ directly.
set -eu
cd "$(dirname "$0")/.."
rm -rf src-tauri/target/ui
mkdir -p src-tauri/target/ui
tar -C ui --exclude ./tests -cf - . | tar -C src-tauri/target/ui -xf -
echo "Staged ui/ in src-tauri/target/ui"
