#!/bin/sh
# Release build of an .rpm on Fedora / RHEL and its clones (run on the target release, e.g. Fedora or AlmaLinux 10,
# so the package links against its libraries). Copies blankey-<version>-1.<dist>.<arch>.rpm to dist/.
#   --install-deps   dnf install the build dependencies first (needs root, as in CI containers)
# Needs: rustup, cargo install tauri-cli --version "^2", cargo install cargo-about.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
. /etc/os-release

if [ "${1:-}" = "--install-deps" ]; then
    shift
    case " $ID ${ID_LIKE:-} " in
        *" rhel "*|*" centos "*)
            dnf install -y 'dnf-command(config-manager)' epel-release
            dnf config-manager --set-enabled crb
            ;;
    esac
    dnf install -y gcc gcc-c++ make git curl wget file pkgconf-pkg-config \
        webkit2gtk4.1-devel gtk3-devel libayatana-appindicator-gtk3-devel librsvg2-devel \
        openssl-devel dbus-devel
fi

# the dist tag goes into the release field, as rpmbuild would: blankey-0.1.0-1.fc43.x86_64.rpm
DIST="$(rpm --eval '%{?dist}')"
[ -n "$DIST" ] || DIST=".${ID}${VERSION_ID%%.*}"
[ -f src-tauri/pdfium/libpdfium.so ] || scripts/fetch-pdfium.sh
scripts/licenses.sh
scripts/stage-ui.sh
cd src-tauri
cargo tauri build --config tauri.release.conf.json \
    --config "{\"bundle\":{\"linux\":{\"rpm\":{\"release\":\"1$DIST\"}}}}" --bundles rpm "$@"
mkdir -p "$ROOT/dist"
cp "${CARGO_TARGET_DIR:-target}"/release/bundle/rpm/*.rpm "$ROOT/dist/"
ls -lh "$ROOT/dist"
