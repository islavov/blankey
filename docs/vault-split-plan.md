# Vault: split vault.rs into vault/ with a repository and an encryption layer

## Context

`src-tauri/src/vault.rs` is 1590 lines that mix four concerns in one `impl Vault`: the SQLite schema and 81 query
sites, key management (password / recovery / keychain wrapping, the request key pair), per-record encryption (AAD
strings, encrypt/decrypt, sealing) and the domain operations (profiles, documents, fill sets, requests, relay grants,
collection links, collected files, audit). Following the library refactor (#30), the vault gets the same rule: one
repository owns the connection, the schema and every query; the rest works with typed rows and never sees SQL.
Encryption moves to its own module so what is encrypted, under which AAD, sits in one place.

The public API stays as it is (`crate::vault::{Vault, VaultError, Profile, FieldInfo, ..., now, kept_months}`, every
`Vault` method signature), so callers in `core.rs`, `app/`, `mcp/`, `links.rs`, `relay/`, `telemetry.rs` do not
change. No schema or behaviour change: the same migrations, the same AADs, the same on-disk bytes.

## Layout (`src-tauri/src/vault/`)

| File | Contents |
|---|---|
| `mod.rs` | `Vault` struct (`path`, `repo`, `secrets`, `dek`, `seal_private`, `results`), `open`, re-exports, `now`, `kept_months`, module docs |
| `error.rs` | `VaultError`, `Result` (the only non-repo file that names `rusqlite::Error`, for `#[from]`) |
| `types.rs` | `Profile`, `FieldInfo`, `Field`, `FieldInput`, `DocumentInfo`, `FillSetInfo`, `Profiles`, `Bindings`, `Request`, `RelayGrant`, `Link`, `SubmissionInfo`, `CollectedFile` |
| `encrypt.rs` | the AADs (`field`, `document`, `fillset`, `request`, `collected`, `collected data`, `secret`, `link_secret`) and small helpers over a `Key`: encrypt / decrypt a field value, a document, fill set bindings, collected file metadata and data; seal / unseal a request part. Pure functions, no I/O |
| `keys.rs` | `impl Vault`: `initialized`, `unlocked`, `initialize`, `unlock*`, keyring enable / disable, `change_password`, `lock`, `require_dek`, secrets in `meta` (`get_secret` / `put_secret`), `after_unlock` (request key pair, sealing legacy requests) |
| `profiles.rs` | `impl Vault`: profiles and fields (`list_profiles` .. `delete_field`, `get_values`, `set_values`, `field_index`) |
| `documents.rs` | `impl Vault`: documents and fill sets |
| `requests.rs` | `impl Vault`: requests (create, get, list, count, resolve, the unsealing of payloads / results) |
| `relay.rs` | `impl Vault`: relay grants and the relay text settings in `meta` |
| `links.rs` | `impl Vault`: collection links, submissions, collected files (`files/<id>.bin`) |
| `audit.rs` | `impl Vault`: `audit`, `usage_months`, `backup` (rotation stays here, the copy goes through the repo) |
| `repo/mod.rs` | `Repo { conn: Connection }`: `open` (WAL, foreign keys, migrate), `backup_to(path)`, shared row helpers |
| `repo/schema.rs` | `MIGRATIONS` and their step functions, `add_column`, `migrate` (moved as they are) |
| `repo/{meta,profiles,documents,requests,relay,links,audit}.rs` | `impl Repo` blocks with that area's queries |
| `tests.rs` | the vault tests, moved from `vault.rs` |

## Repository shape
- Methods take and return plain data: rows with plaintext columns plus `Option<Vec<u8>>` nonce / ciphertext or
  sealed blobs. The repo never sees a key; the domain files call `encrypt.rs` before writing and after reading.
- Inserts whose ciphertext needs the new row id (document, fill set, collected file) take a closure that gets the
  id and returns the encrypted columns, so insert + encrypt + update stay one transaction:
  `repo.insert_document(&meta, |id| encrypt::document(dek, id, content))`. The collected file closure also writes the
  blob, as today, so a failed write rolls the row back.
- Multi-row writes stay transactional in the repo: `replace_fields(profile_id, &[FieldRow])`,
  `seal_legacy_requests(&[(id, payload, result)])`, `delete_link(link_id, secret_key)`.
- The dynamic clauses today (`query_requests(clause, args)`, `query_links`, `query_submissions`, `query_collected`)
  become named methods (`request(id)`, `requests_with_status(s)`, `latest_requests(100)`, ...), so no SQL fragment
  leaves the repo.
- `Repo` methods that need a transaction take `&mut self`; `Vault` keeps its current `&self` / `&mut self` per
  method.

## Tests
- `vault/tests.rs`: today's tests, unchanged in what they assert. The at-rest checks (`values_are_encrypted_at_rest`,
  `ciphertext_cannot_be_moved_between_fields`, sealed requests, legacy plaintext requests, `migrations_rerun_safely`)
  use a `#[cfg(test)] Repo::raw(&self) -> &Connection`, reached as `vault.repo.raw()`.
- `links.rs` and `mcp/tools.rs` tests that read the `audit` table through `vault.conn()` use a new
  `#[cfg(test)] Vault::audit_log() -> Vec<(String, String)>` (action, target) instead; `Vault::conn()` goes away.
- Guard test in `vault/repo/mod.rs`, like `library/repo.rs`: `include_str!` of `mod.rs`, `types.rs`, `encrypt.rs`,
  `keys.rs` and the domain files; none may contain `rusqlite`, `.execute(`, `.prepare(`, `query_row` or
  `query_map`. (`error.rs` names `rusqlite::Error` and `tests.rs` uses raw SQL by design; both are left out.)
- `reads_python_vault` (fixture `tests/data/vault_v3.db`) must keep passing: the proof that the bytes on disk did not
  change.

## Order of work
1. `git mv vault.rs vault/mod.rs` so history follows, then carve out `error.rs`, `types.rs`, `repo/schema.rs`,
   `tests.rs` (pure moves). Build and test.
2. `encrypt.rs`: move the AAD functions and wrap each encrypt / decrypt call site. Build and test.
3. `repo/`: move queries area by area (meta, profiles, documents, requests, relay, links, audit), each area's domain
   file at the same time. Build and test after each area.
4. Guard test, `audit_log`, remove `conn()`.

## Verification
- `cd src-tauri && cargo test` (all vault tests, `reads_python_vault`, `no_tool_leaks_vault_values`, links and relay
  tests); `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`.
- `grep -rln "rusqlite\|\.execute(\|query_row\|\.prepare(" src-tauri/src/vault` lists only `repo/*`, `error.rs`,
  `tests.rs`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. Unlock, open a profile and see its values,
  open a fill set, generate a document, open Activity (sealed requests). Check the Documents and Links pages.
- Save this plan as `docs/vault-split-plan.md` when implementing.
