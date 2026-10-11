#!/bin/sh
# Writes src-tauri/THIRD_PARTY_LICENSES.html: every Rust dependency (cargo-about), plus PDFium, the fonts and the
# UI's vendored Preact and htm.
# Needs cargo-about (cargo install cargo-about) and the PDFium download (scripts/fetch-pdfium.sh).
set -eu
cd "$(dirname "$0")/../src-tauri"
OUT=THIRD_PARTY_LICENSES.html
cargo about generate --fail about.hbs -o "$OUT.tmp"

escape() { sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g' "$1"; }
{
    echo '<h2 id="bundled">Bundled libraries and fonts</h2>'
    echo '<h3>PDFium (BSD-3-Clause / Apache-2.0), https://pdfium.googlesource.com/pdfium/</h3>'
    if [ -f pdfium/LICENSE ]; then echo '<pre>'; escape pdfium/LICENSE; echo '</pre>'; fi
    echo '<h3>Noto Sans, Noto Sans Mono (SIL Open Font License 1.1)</h3><pre>'
    escape assets/fonts/OFL.txt
    echo '</pre>'
    echo '<h3>Typst fonts: Libertinus Serif, Libertinus Math (OFL 1.1), New Computer Modern (GUST Font License),'
    echo 'DejaVu Sans Mono (Bitstream Vera / DejaVu license)</h3>'
    echo '<p>Embedded through the typst-assets crate; see https://github.com/typst/typst-assets for the full texts.</p>'
    echo '<h3>Preact (MIT), https://preactjs.com</h3><pre>'
    escape ../ui/vendor/LICENSE-preact
    echo '</pre>'
    echo '<h3>htm (Apache-2.0), https://github.com/developit/htm</h3><pre>'
    escape ../ui/vendor/LICENSE-htm
    echo '</pre>'
} > bundled.tmp
# insert the bundled section where the template marks it
awk -v file=bundled.tmp '/<!-- BUNDLED -->/ { while ((getline line < file) > 0) print line; next } { print }' "$OUT.tmp" > "$OUT"
rm -f "$OUT.tmp" bundled.tmp
echo "Wrote src-tauri/$OUT"
