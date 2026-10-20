# Library: one repository for all library.db access

## Context

The template folders work (#29) spread `library.db` access over the module: `library/mod.rs` owns the connection,
migrations, the row mapping and some queries, and `library/scan.rs` reaches into `library.conn()` with its own
SELECT / UPDATE / INSERT statements. Scan logic and SQL are mixed, and any new library code would add queries
wherever it happens to be. Goal: one repository type in `library/repo.rs` owns the connection, migrations and every
query; the rest of the module (scan, the `Library` service, watch) calls typed methods and never sees SQL or
`rusqlite`. Scope is the library only; `vault.rs` stays as it is. No skill is recorded for now.

## Changes

### 1. `library/repo.rs` (new, `pub(super)`)
- Moves here from `mod.rs`: `MIGRATIONS`, the migration loop in `Library::open`, `FILE_COLUMNS`, `file_row`, the
  `rusqlite` imports, the `string` error helper (or a local one).
- `pub(super) struct Repo { conn: Mutex<Connection> }` with:
  - `open(path) -> Result<Repo, String>`: connection, `foreign_keys`, migrations.
  - roots: `roots() -> Vec<Root>`, `insert_root(path: &str) -> i64`, `delete_root(id)`.
  - files: `files() -> Vec<LibraryFile>` (not missing), `file(id) -> Option<LibraryFile>`.
  - scan support:
    - `known(root_id) -> HashMap<String, Known>` (the snapshot scan diffs against; `Known` moves here).
    - `upsert_file(root_id, &FileRow)` (today's `store` INSERT ... ON CONFLICT).
    - `move_file(id, &FileRow)` for a detected move (path, size, mtime, category, tags, missing cleared).
    - `set_missing(id, missing: bool)` (replaces both the "clear missing_at" and "set missing_at" statements).
  - `FileRow { rel_path, kind, size, mtime, sha256, title, pages, fields, blanks, readiness, error, category,
    tags }`: plain data the scan builds; the repo turns it into parameters (tags as JSON).
- Each method takes the connection lock for its own statement, as today, so extraction and hashing still run outside
  the lock.

### 2. `library/mod.rs`
- `Library { repo: Repo, scanned, generation }`; `open` calls `Repo::open`.
- `roots`, `add_root` (overlap check stays here, the insert goes to `repo.insert_root`), `remove_root`, `files`,
  `file`, `path`, `sync`, `scan_root`, `generation` keep their signatures, so `core.rs`, `mcp/tools.rs`,
  `app/commands.rs` and the tests outside the module do not change.
- No `rusqlite` import, no `conn()`.

### 3. `library/scan.rs`
- `walk` and the diff stay; every statement becomes a repo call: `repo.known(root.id)`, `repo.set_missing(id,
  false)`, `repo.upsert_file`, `repo.move_file`, `repo.set_missing(id, true)`.
- `store` keeps reading the metadata (`extract`, `categorize`) and builds a `FileRow` for `repo.upsert_file`.
- No `rusqlite` import (`params!` goes away), no SQL strings.

### 4. Guard
- A test in `library/repo.rs` reads the sibling sources with `include_str!` (`mod.rs`, `scan.rs`, `extract.rs`,
  `categorize.rs`, `watch.rs`) and asserts none mentions `rusqlite`, `.execute(`, `.prepare(` or `query_row`, so new
  library SQL lands in the repo.

## Out of scope
- `vault.rs` and the raw-SQL tests in `links.rs` / `mcp/tools.rs` that use `vault.conn()`.
- Recording the rule as a skill.
- Schema changes: the migration stays step 1 as shipped.

## Tests
- Existing library tests keep passing unchanged: `scan::tests::add_change_move_and_remove`,
  `tests::roots_must_not_overlap`, `watch` tests, and `mcp::tools::tests::library_files_become_templates_on_demand`.
- New in `repo.rs`: `upsert_file` twice keeps the id and updates fields; `move_file` and `set_missing` round-trip
  through `file(id)`; the guard test above.

## Verification
- `cd src-tauri && cargo test`; `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`.
- `grep -rn "rusqlite\|\.execute(\|query_row\|\.prepare(" src-tauri/src/library` matches only `repo.rs`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. The existing `library.db` opens without
  migration changes. On Templates → Folders, add, delete and move a file in Finder and check the list follows.
- Save this plan under `docs/` (e.g. `docs/library-repo-plan.md`) when implementing.
