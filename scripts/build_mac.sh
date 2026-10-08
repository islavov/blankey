#!/bin/sh
# Build dist/Blankey-<version>.dmg with precompiled bytecode: the bundle cannot write .pyc files at runtime,
# so without this every launch compiles all Python source again.
set -e
cd "$(dirname "$0")/.."
contents=build/blankey/macos/app/Blankey.app/Contents

if [ -d "$contents" ]; then
    uv run briefcase update macOS
else
    uv run briefcase create macOS
fi
uv run briefcase build macOS
uv run python - "$contents" <<'PY'
import sys
from pathlib import Path

contents = Path(sys.argv[1])
version = next((contents / "Frameworks/Python.framework/Versions").glob("3.*")).name
if version != f"{sys.version_info.major}.{sys.version_info.minor}":
    sys.exit(f"Bundled Python {version} differs from the build Python {sys.version_info[:2]}: bytecode would not match")
PY
uv run python -m compileall -q -j0 --invalidation-mode unchecked-hash \
    "$contents/Resources/app" "$contents/Resources/app_packages" "$contents/Frameworks/Python.framework/Versions/Current/lib" \
    >/dev/null || true  # a few test files of third-party packages do not compile; they are never imported
rm -f dist/Blankey-*.dmg
uv run briefcase package macOS --adhoc-sign
codesign --verify --deep --strict "$contents/.."
ls -lh dist/
