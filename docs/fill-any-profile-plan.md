# Fill from any profile, ask for the rest in the form

## Context

The purchase-offer session showed that filling is slow whenever data sits outside the template's roles. Vasil's offer needed the offer terms from profile 9 and the phone from a separate `request_profile_input`. The workaround was to add a `terms` role, which silently changed fill set 7, and a fresh fill set that lost the typed `offered_price_words`.

The goal is filling speed: use data wherever it already is, and get missing data in the same round trip. For each var Claude picks one of three sources:
- **values**: a non-PII literal Claude knows (listing details, amounts, deadlines). It is stored in the fill set only, as today.
- **paths**: a key in *any* profile, not only the role profiles: `role.key` or `profile:9.key`.
- **ask**: the user types it in the fill form. It stays in the fill set only (encrypted), is never saved to a profile, and there is no toggle. Saving typed PII into a profile is deferred to later.

Collection links stay as they are and are de-emphasized in the MCP text.

## Changes

### 1. Profile-pinned bindings (`templates/fill.rs`, `core.rs`, `vault.rs` doc comment)
- New binding shape `{"path": "key", "profile": 9}` next to `{"path": "role.key"}`. `Bindings` doc comment updated.
- `fill::pinned_profiles(bindings) -> Vec<i64>`.
- Context: `Blankey::fill_context(profiles, bindings)` returns the role context (existing `real_context`) plus `profile id -> bindings::profile_context(values)` for the pinned ids. Use it in `generate_fill`, `app/fill.rs` `evaluate` and `previews`.
- `evaluate_binding` / `resolve` take that context. A pinned path evaluates `{{ p.<key> }}` against `{"p": <that profile's context>}`.
- `describe_binding`: `path:profile:9.key`.
- When the user changes a role's profile, role paths follow the new profile and pinned paths stay. That gives "same terms, different buyer" for free.

### 2. Roles only required when used (`app/fill.rs` `save`, `mcp/tools.rs` `request_fill`)
- New `fill::used_roles(templates, bindings)`: roles referenced by role-path bindings plus roles referenced by the defaults of vars still on `{"default": true}`.
- "Choose profiles for" (save) and "Missing profiles for roles" (request_fill) check only the used roles. A template whose vars are all pinned, asked or literal needs no role profile.
- The form shows unused roles as optional.

### 3. `ask` vars (`mcp/tools.rs`, `app/fill.rs` `OpenArgs`/`Session`, `ui/js/fill/*`, `ui/js/requests.js`)
- `request_fill` arg `ask: {var: hint}` (lenient string map). It is validated against known vars and is exclusive with `values`/`paths` for the same var. Its vars count as covered in the `uncovered` check.
- Payload and `OpenArgs.ask` go to `Session.ask`. In the form:
  - Asked rows get the chip "To type" (kind `Ask`) and the hint as placeholder.
  - The first asked row opens in typing mode.
  - Enter moves to the next asked row.
- The typed value is stored as `{"value": ...}` in the fill set, so nothing new is stored. The summary counts asked-but-empty rows.
- `requests.js` passes `ask` through.

### 4. Pick any profile in the form (`ui/js/fill/source.js`, `form.js`, `app/commands.rs`, `app/fill.rs`)
- New command `fill_profile_entries(profile_id)` -> `[{key, value}]` (UI only).
- In the source menu, below the role sections, an item "From another profile…" opens a second popup listing the profiles. Picking one opens a popup with its fields. Picking a field binds `{path: key, profile: id}`.
- `Evaluation.sources` also includes the pinned profiles, keyed `profile:ID` (the menu shows the profile name), so they show up in the menu directly. `Evaluation.used_roles` tells the form which roles still need a profile.
- `describe` shows a pinned path as `Vault` with path `<profile name> › key` (names from `all_profiles`).

### 5. Faster discovery and fill set reuse (`mcp/tools.rs`)
- `list_profiles` gets an optional `fields: true` that returns each profile's `[{key, label, type, length}]`, so one call shows where the data is. This replaces describing 7 profiles one by one.
- `request_fill` paths accept `profile:ID.key`, checked with `vault.describe(id)` + `fill::has_field`.
- `request_fill` gets `copy_fill_set_id`: the form starts from that set's templates, profiles and bindings (explicit args win), but saves as a new fill set (`fill_set_id: None`). Typed values like `offered_price_words` carry over.
- Docs: `fill_set_id` reopens and **overwrites** that set, and `copy_fill_set_id` forks it.

### 6. MCP text (`INSTRUCTIONS`, `request_fill` description)
- Step 7 is rewritten as a per-var decision, aimed at one `request_fill` call:
  1. Non-PII you know → `values`.
  2. Present in any profile (`list_profiles` with `fields: true`) → `paths`, as `role.key` for the role's profile or `profile:ID.key` for any other.
  3. Otherwise → `ask`, with a short hint. The user types it in the form and it stays in this fill set. Don't call `request_profile_input` first just to fill a document.
- Never put PII in `values`; `ask` for it instead.
- Keep roles for the party a template is about, and don't add roles just to reach another profile's data.
- Replace step 6 with: "request_profile_input only when the user wants data kept in a profile for reuse".
- Collection links section: one leading line saying they are for data held by someone else.

## Out of scope (from the session notes, later)
- Saving asked values into a profile.
- Typst built-in shadowing lint.
- Bulgarian `words` filter.
- save_template warnings for fill sets.
- `copy_profile_fields`.
- Splitting collection links across profiles.

## Tests
- `templates/fill.rs`:
  - a pinned binding evaluates against its profile.
  - `used_roles` skips roles that only pinned or asked vars would use.
  - `describe_binding` handles a pinned path.
- `mcp/tools.rs` `request_fill_validates_input`:
  - `profile:ID.key` is accepted, and an unknown key or profile is rejected.
  - `ask` covers a var that would otherwise be uncovered, and an `ask` var that also appears in `values` errors.
  - an unused role needs no profile.
  - `copy_fill_set_id` with an unknown id errors.
  - `no_tool_leaks_vault_values` still passes with `list_profiles` `fields: true` (lengths only).
- `app/fill.rs` / core: `generate_fill` with a pinned binding renders the other profile's value.
- `ui/tests/fill.test.js`:
  - an `Ask` row starts in typing mode.
  - the "From another profile…" flow binds `{path, profile}`.
  - a pinned row shows the profile name.

## Verification
- `cd src-tauri && cargo test`, `cd .. && npm test`.
- Lint afterwards: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the UI lint/i18n check (`scripts/i18n-check.mjs` for the new strings, with `ui/i18n/bg.json` updated).
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir, then over MCP:
  - Run `list_profiles` with `fields: true`.
  - Call `request_fill` for `property_purchase_offer` with buyer 1, offer terms as `profile:9.*`, the phone as `ask`, and `copy_fill_set_id` from the first offer's set.
  - The form should open in one round trip with the phone row ready to type, the terms showing profile 9's name, and `offered_price_words` carried over. Save and Generate produces the document.
- Save this plan as `docs/fill-any-profile-plan.md` when implementing.
