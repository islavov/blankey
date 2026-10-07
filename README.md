# Blankey

Local, offline tray app that keeps personal data encrypted and fills document templates.
An MCP server lets Claude design templates, test them with invented data, and ask for real
documents, without ever seeing the real values.

## Run

```bash
uv sync
uv run blankey          # first run creates the vault (master password + one-time recovery key)
uv run pytest
uv run ruff check && uv run ruff format
```

The app lives in the menu bar / system tray. Data is stored in the per-user data directory
(`~/Library/Application Support/Blankey` on macOS): `vault.db`, `templates/`, `previews/`, `config.toml`.

## Connect Claude

The server listens on `http://127.0.0.1:8765/mcp` without authentication. It only accepts local
connections, the SDK rejects foreign Host/Origin headers (DNS rebinding), and no tool returns personal data.

- **Claude Desktop**: tray menu → "Connect Claude Desktop", then restart Claude Desktop. This adds
  `blankey mcp` (a stdio bridge to the running app, which starts the app if needed) to
  `claude_desktop_config.json`.
- **Claude Code**: `claude mcp add --transport http blankey http://127.0.0.1:8765/mcp`
  (tray menu → "Copy Claude Code command").

## How the PII boundary works

- Profile values are AES-256-GCM encrypted per field. A random data key is wrapped by the master
  password (Argon2id), optionally by the OS keychain, and by a recovery key.
- Field keys, labels, types and value lengths are plaintext metadata. That is all MCP can read.
- MCP tools render templates only with example data. Real documents are rendered in the app,
  after the user approves the request, and stored encrypted. Only metadata goes back to Claude.
- Requests from Claude (`request_profile_input`, `request_generate`, `request_fill`) open a dialog in the app;
  Claude blocks on `wait_request` until the user saves, approves or cancels.
- Request payloads and results are sealed to a request key pair (X25519 + AES-GCM): stored while the vault
  is locked, readable only after unlocking. The Activity page needs an unlocked vault.
- `tests/test_mcp.py` pushes sentinel values through every tool and asserts they never come back.

## Templates

`templates/<id>/manifest.yaml` maps each PDF field / template variable to a Jinja expression over
roles, e.g. `{{ applicant.address.permanent.city }}` or `{{ loan.amount | money }}`. Kinds:

- `pdf_form`: fills an AcroForm. Text is drawn with the bundled Noto Sans (Latin, Cyrillic, Greek)
  and flattened; checkboxes / radios use their native on-state.
- `typst`: `main.typ` reads `json(bytes(sys.inputs.data))`.
- `docx`: docxtpl; converted to PDF only if LibreOffice is installed.

### Filled .docx documents and fill sets

Claude turns an existing filled .docx into a template with `inspect_docx` + `save_docx_template`
(text spans become `{{ var }}` tags; use the same var names across related documents). `request_fill`
opens a form listing every blank of one or more templates with its surrounding text; per blank you pick a
vault field (`manager.egn`), a literal Claude proposed (non-PII only), the template default, or type a value.
The choices are saved as a named fill set, encrypted in the vault, and can be reopened, regenerated,
exported (YAML, or a zip with the templates) and imported from tray → "Fill sets…".

## Signing

- Self-signed: a certificate is generated on first use and stored encrypted in the vault.
- QES: set `pkcs11_lib` in `config.toml` to the card driver (e.g. the B-Trust / StampIT PKCS#11 library).
  Encryption is applied first and the signature is added incrementally, so both stay valid.

Linux: the tray needs StatusNotifier support (GNOME: AppIndicator extension).
