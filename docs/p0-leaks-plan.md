# Close the two P0 leaks: the check_bindings oracle and remote reads of library files

## Context

The hardening review found two ways an assistant can get at data that Blankey promises it never sees. These two
are fixed now; the rest of the review waits.

1. **`check_bindings` is a value oracle.** `templates/engine.rs::check` evaluates every binding expression against
   real profile values when the vault is unlocked, with no approval. The assistant writes those expressions
   (`save_template`), and the MCP tool returns three signals:
   - error vs ok, which a conditional expression turns into one bit per field per call
   - `missing_glyphs`: real characters, which a crafted expression can map from the value
   - `fit_ratio`: the width of the real text

   Together these let the assistant extract values. `check_bindings` is MCP only: its sole caller is
   `mcp/tools.rs::check_bindings` → `core.rs::check_bindings`.
2. **Remote agents read library files without asking.** `mcp/tools.rs::source_path` resolves `lib:<id>` for remote
   sessions (claude.ai through the relay), so `inspect_docx` and `inspect_pdf` return the text and page images of
   any file in the library folders, which often hold filled client documents.

Outcome:
- `check_bindings` output depends only on metadata, plus fixed facts about stored values the assistant cannot shape.
- A remote agent opens a library file only after the user approves that file, the same way `request_file` works for
  uploads.

## 1. check_bindings: no evaluation of assistant-written expressions on real data

- `templates/bindings.rs`: new `pub fn plain_path(expression, roles) -> Option<String>`, true for exactly
  `{{ role.key }}` (whitespace ignored). The test is the one `fill::default_binding` does inline today
  (`templates/fill.rs:126-135`); `default_binding` then calls it.
- `templates/engine.rs::check`:
  - Real-value checks run only when `plain_path` matches. The text checked is then the stored value itself, so the
    expression cannot change what is measured.
  - Computed expressions (filters, conditionals, concatenation) get the metadata checks only (missing / empty /
    literal) and are never evaluated on real data.
  - `BindingReport` loses `missing_glyphs` and `fit_ratio`; the statuses `overflow` and `glyphs` stay as plain
    statuses.
  - The `error` status for real values goes away: a plain path cannot fail, and computed expressions are not
    evaluated.
- `mcp/tools.rs::check_bindings` description:
  - real-value checks (overflow, glyphs) cover plain `{{ role.key }}` bindings only and say nothing about the
    value itself
  - use `render_example` with invented data for computed expressions
- Residual, noted in a code comment: the overflow status of a plain path still reveals whether the stored value fits
  a box the assistant sized. Repeated saves can narrow down the text width, never the characters. That's acceptable
  next to the value lengths, which are already metadata.

## 2. Remote library reads need the user's approval

- `request_file` gets an optional `source: "lib:<id>"` (from `find_templates`):
  - The payload gets `source`, plus the file's relative path and kind, so the sheet can name the file.
  - The id is checked against `app.library.file(id)`.
  - With a source, `kind` is taken from the file.
- `core.rs`: `share_library_file(request_id) -> Result<Value, String>`:
  - The request must be a pending `file` request with a `source`, and the file must still be on disk.
  - It resolves the request with `{outcome: "shared", file_id: "lib:<id>", kind, size}` and audits
    `share_library_file`.
- `core.rs`: `library_shared(grant, source) -> bool`: some `done` `file` request of that grant has `result.file_id ==
  source`. It reads `vault.list_requests(Some("done"))`, which needs an unlocked vault; when locked it is false.
- `mcp/tools.rs::source_path`: for a remote session, `lib:` resolves only when `library_shared` is true; otherwise
  the error is "Ask the user first: request_file with source lib:<id>". Local sessions are unchanged.
- `app/commands.rs`: `share_library_request(id)` calls `share_library_file`, refreshes the tray and emits
  `requests-changed`, like `resolve_file_request`.
- `ui/js/requests.js` `FileRequest`: when `payload.source` is set:
  - the head reads "Your AI assistant asks to read a file from your folders"
  - the body shows the file's path
  - the primary button "Allow" calls `share_library_request`
  - Cancel works as now
  - Strings go in `ui/i18n/bg.json`.
- `find_templates` and `INSTRUCTIONS` text: remote agents call `request_file` with `source` before `inspect_*` or
  `save_*` on a candidate.

## Tests
- `templates/engine.rs` or `mcp/tools.rs`: oracle test.
  - Two profiles whose sentinel values differ but have the same lengths.
  - A template with a conditional-error expression (`{% if applicant.name[0] == "S" %}{{ nope() }}{% endif %}`), a
    glyph-mapping expression (`{{ applicant.name | replace("S", "一") }}`) and a plain path.
  - `check_bindings` output is identical for both profiles and contains no character of either value.
- `templates/bindings.rs`: `plain_path` cases; `fill::default_binding` behaviour unchanged (existing tests).
- `mcp/tools.rs`: remote library test.
  - A remote `inspect_docx` on `lib:<id>` fails with the hint.
  - `request_file` with `source` creates a request naming the file; after `share_library_file` the same grant's
    `inspect_docx` and `save_docx_template` work.
  - Another grant still fails, and local sessions read `lib:` without a request.
  - Update `library_files_become_templates_on_demand`, which used a remote `inspect_docx`.
- `ui/tests`: a file request with `source` shows the path and "Allow", and calls `share_library_request`.
- `no_tool_leaks_vault_values` keeps passing.

## Verification
- `cd src-tauri && cargo test`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`; `npm test`;
  `node scripts/i18n-check.mjs`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. Over local MCP, `check_bindings` on an
  existing template still reports missing / empty keys. The remote path (claude.ai) shows the new Allow sheet when
  it asks for a library file.
- Save this plan as `docs/p0-leaks-plan.md` when implementing.
