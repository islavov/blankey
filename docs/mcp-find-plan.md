# MCP: find instead of list, with small results

## Context

Profiles, templates, fill sets and documents are expected to grow into the hundreds or thousands. The MCP list tools return everything:
- `list_profiles`, which with `fields: true` also includes every field of every profile.
- `list_templates`.
- `list_fill_sets`, which includes the source of every var.
- `list_documents`.
- `list_requests`, which returns up to 100.

All of this goes into the assistant's context, so every session slows down and costs more as the vault grows. The goal: the assistant finds what it needs with criteria, and gets small paged results. Detail comes from the existing single-item tools (`describe_profile`, `get_template`) and a new `get_fill_set`.

Decisions:
- The `list_*` tools for these four are removed and replaced by `find_*`. Called with no criteria, a `find_*` tool returns the most recent or first N.
- Profile search uses metadata only: name, kind, field keys, labels, filled or empty. No value search. Masks such as `egn:8907XXXXXX` may come later.
- The UI is unchanged. The UI's tauri `list_*` commands stay.

## Common shape (`mcp/tools.rs`)
- Every find tool takes these args:
  - `query`: free text, split into words. Each word must appear (case-insensitive, NFC, Cyrillic-safe `to_lowercase`) in one of the item's searchable attributes.
  - `limit`: default 20, at most 100.
  - `offset`.
- Helper `fn words(query) -> Vec<String>` and `fn hit(words, attrs: &[&str]) -> bool`.
- Results come back as `{"total": N, "next_offset": M?, "items": [...]}`. `next_offset` is present only when there are more. A new `found(total, offset, items)` writes compact JSON with one item per line. It does not use `dump`'s pretty printing, because the newlines and indentation add a lot of tokens per item.
- Ranking: an exact id or name match comes first, then a name prefix match, then everything else in the tool's default order.

## Tools

### `find_profiles` (replaces `list_profiles`)
- Args:
  - `query`: matches name, kind, field keys and labels.
  - `kind`.
  - `keys: [key]`: the profile must have every key (`fill::has_field` semantics, so `address` matches `address.city`).
  - `filled: bool`: default true when `keys` is given; the keys must also be non-empty (length > 0).
  - `ids`.
  - `with_fields: bool`.
- Each item: `{id, name, kind, fields: <count>, filled: <count>}`.
  - With `keys`: also `keys: {key: length}`, for the requested keys only.
  - With `with_fields`: the full `[{key, label, type, length}]`. Use this for a few profiles, not for hundreds.
- Default order: by name.
- Vault: a new `field_index() -> HashMap<i64, Vec<FieldInfo>>`. It is a single `SELECT profile_id, key, label, type, value_length FROM fields`, plaintext metadata only, so it also works while the vault is locked, the way `describe` does. Add an index on `fields(key)` in a new migration, so a later SQL filter stays cheap.
- This is the "where is the data already" lookup for fills: `find_profiles(keys: ["offered_price", "deposit_amount"])` returns only the profiles that have those keys filled.

### `find_templates` (replaces `list_templates`)
- Args:
  - `query`: matches id, name, description, role names, var names and labels.
  - `kind`.
  - `role`.
  - `var`.
- Each item: `{id, name, kind, roles: [names], vars: <count>, description: <first 120 chars>}`.
- Default order: by name.
- `TemplateStore` cache (`templates/mod.rs`): `all()` currently parses every `manifest.yaml` on every call. Instead, keep `Mutex<HashMap<id, (mtime, Template)>>`, stat each manifest, and re-parse only the changed ones. `save` and `delete` evict their entry. The UI's `list_templates` command benefits too.

### `find_fill_sets` (replaces `list_fill_sets`) and `get_fill_set`
- `find_fill_sets` args:
  - `query`: matches the name, template ids and names, and profile names.
  - `template_id`.
  - `profile_id`: matches any role.
- Each item: `{id, name, templates, profiles: {role: id}, updated_at}`.
- Default order: newest first.
- The var sources are no longer included.
- `get_fill_set(fill_set_id)`: one set, plus its per-var source kinds (`fill::describe_binding`). This is what `list_fill_sets` returned for every set.

### `find_documents` (replaces `list_documents`)
- Args:
  - `query`: matches title, filename and template id.
  - `template_id`.
  - `profile_id`.
  - `since`: an ISO date.
- Each item: the existing `DocumentInfo` fields.
- Default order: newest first.

### Queues
- `list_requests` and `list_collection_links` keep their names; both are short queues, not catalogs.
- Both get `limit` and `offset` and return the `found` shape. `list_collection_links` also gets `status` (active, expired, revoked) and `query` over the title.

### MCP text
- `INSTRUCTIONS`:
  - Step 2: "find_profiles with keys to see which profiles already hold the data a template needs. describe_profile for one profile's fields."
  - Name `find_templates` where an existing template should be reused before writing a new one.
  - Docx step 4: `find_fill_sets` / `get_fill_set`.
- Add a line: "Search with criteria; don't page through everything. Results are capped (limit, next_offset)."
- The `request_fill` description: `list_profiles with fields: true` becomes `find_profiles with keys`.

## Out of scope
- Value masks and value search.
- UI pagination or search changes.
- Full-text indexes. A linear scan over metadata is enough at thousands; measure before adding FTS.

## Tests (`mcp/tools.rs`, `templates/mod.rs`, `vault.rs`)
- `find_profiles`:
  - query on a name and on a field label.
  - `keys` with `filled` true and false; an empty key is excluded by default.
  - paging gives `total`, and `next_offset` is present only when more items remain.
  - `limit` is capped at 100.
- `find_templates`:
  - query on a var name and a role.
  - after `save_template` renames a template, the new name is found (the cache is invalidated).
- `find_fill_sets` filters by `profile_id` and `template_id`, newest first; `get_fill_set` gives per-var sources.
- `find_documents` filters by `template_id` and `since`.
- Update `no_tool_leaks_vault_values` and `locked_vault_allows_status_and_requests_only` to the new tool names, including `find_profiles` with `with_fields`, which still returns no values.
- Unit tests for `words` and `hit`, including Cyrillic case folding.

## Verification
- First `git checkout main && git pull`, then branch `mcp-find`.
- `cd src-tauri && cargo test`; `npm test` to confirm the UI is untouched.
- Lint afterwards: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `node scripts/i18n-check.mjs`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. Over MCP:
  - `find_profiles` with `keys: ["offered_price"]` returns profile 9 only.
  - `find_templates` with `query: "оферта"` finds `property_purchase_offer`.
  - `find_fill_sets` with `profile_id: 9` returns fill set 7, and `get_fill_set(7)` shows its sources.
- Save this plan as `docs/mcp-find-plan.md` when implementing.
