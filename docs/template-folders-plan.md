# Template folders: index the user's own folders, templatize on demand

## Context

Templates exist only in `<data dir>/templates/<id>/`, made one at a time by the assistant. Firms already keep a
folder of forms and model contracts (meet customers where they are). The user adds such a folder; Blankey walks it,
indexes every pdf/docx with metadata and a name-based category (no AI), and the assistant templatizes a file only
when a fill needs it. A docx stays as the user has it: the template keeps a replayable recipe and a derived copy.

This pulls the "Library" and "Templates over library files" parts of `docs/chat-design.md` forward on their own,
for MCP agents, without the chat. Decisions taken:
- Any number of folders (roots), added and removed on the Templates page (Folders view).
- Index in a separate SQLite `library.db` (ready for FTS later); the roots live there too, not in `config.toml`.
- `library/watch.rs` (one entry point, `watch::start(folders, poll)` over a `Folders` trait): the OS watcher
  (`notify`: FSEvents, ReadDirectoryChangesW, inotify) rescans 0.3 s after the last relevant event, and a 60 s poll
  covers network shares, sync clients and folders that cannot be watched. A scan stats files and hashes only those
  whose size or mtime changed. Fills and `find_templates` also rescan when the last scan is older than 10 s. A scan
  that changes anything emits `library-changed`; the Templates page reloads on it.
- docx: recipe + cached tokenized copy, regenerated when the source hash changes.
- No form/case class gate yet.
- Candidates surface in `find_templates` and the Templates page; no new find tool.

## Changes

### 1. Config (`config.rs`)
- `library_db_path()` -> `<data dir>/library.db`.

### 2. Library module (`src-tauri/src/library/`)
- `mod.rs`: `Library { conn: Mutex<Connection> }` over `library.db`, migrations in the vault's style
  (`MIGRATIONS` + `user_version`, `vault.rs:33`).
  - `roots(id, path, added_at)`
  - `files(id, root_id, rel_path, kind, size, mtime, sha256, title, pages, fields, blanks, readiness, error,
    category, tags (JSON), indexed_at, missing_at)`, unique `(root_id, rel_path)`
  - Roots may not overlap. A file is a candidate while no template's `origin.file` names it, so one file can back
    several templates and nothing has to be kept in sync.
- `scan.rs`: walk a root (`std::fs::read_dir` recursion, skip hidden, `~$` lock files, `_blankey_trash`), only
  `.pdf` / `.docx`. Unchanged mtime+size -> skip; else rehash (sha2) and re-extract. A vanished path whose hash
  reappears elsewhere is a move: keep the row id, update `rel_path`. Vanished otherwise -> `missing_at`.
  Runs when a root is added, on watcher events, every minute, and on demand (Rescan, fills, find_templates).
- `extract.rs`, metadata without AI, reusing existing readers:
  - docx: `docProps/core.xml` title (via `docx::read_part`), first non-empty paragraph as fallback,
    `docx::variables` (existing `{{ }}`), a count of blank runs (`.{4,}`, `_{3,}`, `[   ]`, `«...»`) from
    `docx::inspect` texts.
  - pdf: `pdf_form::inspect_form` field count, `preview::page_sizes`, text layer present via `preview::layout_lines`
    on page 1.
  - readiness: `ready` (AcroForm fields or `{{ }}` tags), `blanks` (blank runs found), `needs_ai`, `scan` (no text).
- `categorize.rs`: tokens from the file name and folder names (NFC, lowercase via `mcp::find::fold`, split on
  separators and camel case, drop dates, numbers, `v2`, `final`, `копие`, `draft`). Match stems against a seeded
  table embedded with `include_str!("categories.yaml")` (bg + en: contract, lease, power_of_attorney, declaration,
  application, agreement, minutes, invoice, notice, claim, ...). First hit is `category`; folder names become tags.

### 3. `lib:` sources (`mcp/tools.rs` `source_path`, `core.rs`)
- `lib:<file id>` resolves to `root/rel_path` like `file:<uuid>` does, for local and remote sessions.
- Accepted by `inspect_pdf`, `inspect_docx`, `save_template`, `save_docx_template`, `save_pdf_overlay_template`.

### 4. Template manifest (`templates/mod.rs`)
- New optional manifest keys: `origin: {file: lib:<id>, sha256, recipe}` (recipe: the docx replacements list) and
  `stale` (why the last rebuild failed). `SaveTemplate` gets `origin`; saving clears `stale`.
- `save_docx_template` with a `lib:` source stores the recipe; `template.docx` stays the tokenized copy (cache).
- `Blankey::refresh_template(id)`: when the library file's sha256 differs from `source_sha256`, replay
  `docx::tokenize(original, cache, recipe)`. Success updates `source_sha256`; failure leaves the old cache and
  records `stale: <report>` (returned in `get_template`, shown as a badge, fill warns). Called from the scan when a
  linked file changes. PDF forms re-copy the source and re-run `check_bindings`-style field name checks; overlay
  templates keep `flat.pdf` and mark stale when page sizes change.

### 5. MCP (`mcp/tools.rs`)
- `find_templates` merges library candidates (files with no `template_id`) after templates:
  `{source: "lib:<id>", status: "candidate", name: title or file name, kind, category, tags, readiness, path}`.
  Templates linked to a changed source show `status: "stale"`. Query matches path, file name, title, category,
  tags. New arg `status` (template, candidate, stale).
- `request_fill` with a `lib:` id in `templates`: error "not a template yet; inspect_docx/inspect_pdf with
  source_path lib:<id>, then save_*" plus the readiness.
- `INSTRUCTIONS`: one line: "find_templates also lists the user's own files (status candidate, source lib:<id>):
  templatize one only when a fill needs it, with source_path lib:<id>."

### 6. UI (`ui/js/pages/templates.js`, `app/commands.rs`)
- Commands `library_roots`, `add_library_root` (folder picker), `remove_library_root`, `library_files`,
  `rescan_library`, `library_preview`, `open_library_file`, `show_library_file`.
- Templates page: a Templates / Folders switch. Folders lists the roots (Add folder…, Remove, Rescan) and the
  candidates with type, readiness and category; the panel previews the selected file (docx as HTML, the first 10
  PDF pages) with Open and Show in Finder. Show in Finder lives only on the preview panels. Stale badge on
  templates.
  Strings and category names in `ui/i18n/bg.json`.

## Out of scope
- form/case classes and refusing `inspect_*` on case files.
- Native Word placeholders (content controls, MERGEFIELD) as zero-AI templates: a good next slice.
- FTS search, assistant tag suggestions.

## Tests (English fixtures, invented values, no scale tests)
- `library/categorize.rs`: bg and en names, folder tags, version/date noise stripped.
- `library/extract.rs`: docx with dots/underscores -> `blanks`; docx with `{{ }}` -> `ready`; AcroForm PDF -> `ready`.
- `library/scan.rs` on a tempdir: add, modify (rehash), move (same id), delete (`missing_at`).
- `templates/mod.rs`: manifest round-trips `source`, `source_sha256`, `recipe`.
- core: edit the original docx -> `refresh_template` regenerates the cache; break the recipe -> stale with report.
- `mcp/tools.rs`: `find_templates` returns candidates with `status`; `save_docx_template` with `lib:` links the
  row; `request_fill` on a candidate errors; `no_tool_leaks_vault_values` still passes.
- `ui/tests`: Folders section lists candidates from the fake backend.

## Verification
- `cd src-tauri && cargo test`, `cd .. && npm test`.
- Lint afterwards: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `node scripts/i18n-check.mjs`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. Add a fixture folder on the Templates
  page and check the categories. Over MCP: `find_templates query: lease` shows the candidate,
  `save_docx_template source_path: lib:<id>`, then `request_fill`. Edit the original docx and check that the template
  rebuilds; break the edited span and check it goes stale.
