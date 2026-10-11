# Blankey (Rust + Tauri)

Local, offline tray app that keeps personal data encrypted and fills document templates. An MCP server lets an AI
assistant (any MCP client) design templates, test them with invented data, and ask for real documents, without ever
seeing the real values.

Rewrite of the Python/Qt app with the same data format: `vault.db`, `templates/`, `config.toml` and the keychain
entry are shared, so an existing vault opens unchanged (see `vault::tests::reads_python_vault`).

## Run

```bash
scripts/fetch-pdfium.sh          # once: PDFium for page rendering and text extraction
cd src-tauri
cargo run                        # debug builds use ~/Library/Application Support/Blankey Preview
cargo test
cargo fmt && cargo clippy --all-targets
cd .. && npm install && npm test # UI tests (Node, happy-dom), no build step
```

`BLANKEY_DATA_DIR=/some/dir` picks another data dir. Release builds use the same folder as the Python app
(`~/Library/Application Support/Blankey`); `scripts/copy-python-data.sh` copies it into the preview folder.

## Release builds

Each script builds on its own platform and copies the installers and `THIRD_PARTY_LICENSES.html` to `dist/`.
They need `cargo install tauri-cli --version "^2"` and `cargo install cargo-about`; CI does the same in
`.github/workflows/build.yml` (manual run, or push a `v*` tag for a draft release).

| Platform | Script | Output |
|---|---|---|
| macOS (Apple Silicon) | `scripts/build_mac.sh` | `.dmg` |
| Windows x86-64 | `scripts/build_windows.ps1` | NSIS `.exe` and `.msi` |
| Debian 13 trixie, Ubuntu 26.04 | `scripts/build_deb.sh --install-deps` (on that release) | `.deb` per release |

`scripts/build_deb_docker.sh debian:trixie` (or `ubuntu:26.04`) runs the .deb build in a container locally.
PDFium is fetched for the platform and bundled next to the app. Builds are not code-signed: on macOS use
System Settings → Privacy & Security → "Open Anyway", on Windows SmartScreen → "Run anyway". `.docx` output is
converted to PDF when LibreOffice is installed.

## Licenses

Blankey is MIT. `scripts/licenses.sh` lists every Rust dependency with its license text (cargo-about, accepted
licenses in `src-tauri/about.toml`), plus PDFium (BSD-3-Clause), Noto Sans (OFL) and the Typst fonts, in
`THIRD_PARTY_LICENSES.html`, which ships inside each package.

## Connect an AI assistant

The server listens on `http://127.0.0.1:8765/mcp` (streamable HTTP, JSON responses); it moves to the next free port
when 8765 is taken. Any MCP client can connect; menu bar → Connect MCP has shortcuts for common ones:

- **Claude Desktop**: "Configure Claude Desktop" adds `Blankey mcp` (a stdio bridge that starts the app if needed).
- **Claude Code**: `claude mcp add --transport http blankey http://127.0.0.1:8765/mcp`
- **Codex**: `codex mcp add blankey --url http://127.0.0.1:8765/mcp`

Only loopback connections are accepted and foreign Host/Origin headers are refused. No tool returns personal data
(`mcp::tools::tests::no_tool_leaks_vault_values`).

## Layout

- `src-tauri/src/vault.rs`, `crypto.rs`, `keychain.rs`: SQLite vault, AES-256-GCM per field, Argon2id-wrapped keys,
  X25519-sealed requests, keychain / Touch ID unlock, rotating backups
- `src-tauri/src/templates/`: template store, Jinja bindings (minijinja), docx tokenizing and rendering, Typst,
  fill sets, previews
- `src-tauri/src/render/`: AcroForm filling with an embedded subset of Noto Sans (lopdf), PDFium previews
- `src-tauri/src/mcp/`: MCP server (axum) and the `blankey mcp` stdio bridge
- `src-tauri/src/app/`: tray, window, auto-lock, request handling and the commands the UI calls
- `ui/`: Preact + htm components (vendored in `ui/vendor`), loaded as ES modules with no build step. `ui/js/main.js`
  boots, `pages/` has one file per page, `fill/` the fill form. `ui/tests/` runs the real UI in happy-dom against a
  fake backend; release builds package `ui/` without the tests (`scripts/stage-ui.sh`)
