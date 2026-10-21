#!/bin/sh
# Writes dist/latest.json for the updater (src-tauri/src/app/updates.rs) from the signed artifacts in dist/: the
# macOS app archive and the Windows NSIS and MSI installers, each with its .sig. Linux reads only the version.
#   scripts/updater-manifest.sh v0.2.6 [owner/repo]
# Needs: jq.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TAG="$1"
REPO="${2:-islavov/blankey}"
VERSION="${TAG#v}"
DIST="${DIST:-$ROOT/dist}"
BASE="https://github.com/$REPO/releases/download/$TAG"

one() {
    found=""
    for f in $DIST/$1; do
        [ -f "$f" ] || continue
        [ -z "$found" ] || { echo "more than one $1 in dist/" >&2; exit 1; }
        found="$f"
    done
    [ -n "$found" ] || { echo "no $1 in dist/" >&2; exit 1; }
    [ -f "$found.sig" ] || { echo "no signature for $(basename "$found")" >&2; exit 1; }
    echo "$found"
}

entry() {
    jq -n --arg url "$BASE/$(basename "$1")" --rawfile signature "$1.sig" '{url: $url, signature: $signature}'
}

MAC="$(one 'Blankey_aarch64.app.tar.gz')"
NSIS="$(one "Blankey_${VERSION}_x64-setup.exe")"
MSI="$(one "Blankey_${VERSION}_x64_*.msi")"

# this version's section of the changelog, without its heading
NOTES="$(awk -v v="## $VERSION" '$0 == v {on = 1; next} /^## / {on = 0} on' "$ROOT/CHANGELOG.md" | sed '/./,$!d')"

jq -n \
    --arg version "$VERSION" \
    --arg notes "$NOTES" \
    --arg date "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --argjson mac "$(entry "$MAC")" \
    --argjson nsis "$(entry "$NSIS")" \
    --argjson msi "$(entry "$MSI")" \
    '{version: $version, notes: $notes, pub_date: $date, platforms: {
        "darwin-aarch64": $mac,
        "darwin-aarch64-app": $mac,
        "windows-x86_64": $nsis,
        "windows-x86_64-nsis": $nsis,
        "windows-x86_64-msi": $msi
    }}' > "$DIST/latest.json"
echo "wrote $DIST/latest.json for $VERSION"
