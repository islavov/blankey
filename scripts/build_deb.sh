#!/bin/sh
# Release build of a .deb on Debian / Ubuntu (run on the target release, e.g. Debian 13 trixie or Ubuntu 26.04,
# so the package links against its libraries). Copies blankey_<version>_<arch>_<distro>.deb to dist/.
#   --install-deps   apt-get the build dependencies first (needs root, as in CI containers)
# Needs: rustup, cargo install tauri-cli --version "^2", cargo install cargo-about.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

if [ "${1:-}" = "--install-deps" ]; then
    shift
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends \
        build-essential curl wget file pkg-config ca-certificates \
        libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
        libxdo-dev libssl-dev libdbus-1-dev
fi

. /etc/os-release
DISTRO="${VERSION_CODENAME:-$ID}"
[ -f src-tauri/pdfium/libpdfium.so ] || scripts/fetch-pdfium.sh
scripts/licenses.sh
cd src-tauri
cargo tauri build --config tauri.release.conf.json "$@"
mkdir -p "$ROOT/dist"
for deb in "${CARGO_TARGET_DIR:-target}"/release/bundle/deb/*.deb; do
    name="$(basename "$deb" .deb)"
    cp "$deb" "$ROOT/dist/${name}_${DISTRO}.deb"
done
ls -lh "$ROOT/dist"
