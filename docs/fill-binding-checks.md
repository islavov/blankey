# Fill bindings: catch bare paths, make the assistant map vars to profile keys

## Context

Filling `real_estate_purchase_offer` with profile 9 opened a form where every var was "Default" showing the literal text `buyer.buyer_full_name` instead of the vault value. Verified causes:

1. **Broken template, silently accepted.** The assistant saved the field bindings as bare paths (`"buyer_name": "buyer.buyer_full_name"`) instead of `"{{ buyer.buyer_full_name }}"`. `Templates::save` (`src-tauri/src/templates/mod.rs:202`) does no validation.
2. **`check_bindings` false pass.** `referenced_paths` (`templates/bindings.rs:146`) only scans `{{ }}`/`{% %}` regions, so it finds no paths. Nothing is missing, and `evaluate` renders the bare text as a non-empty string, so all 10 fields came back `ok` (`templates/engine.rs:122`).
3. **Fill form can't recover.** `default_binding` (`templates/fill.rs:94`) turns a var into a path only when its default is exactly `{{ role.key }}`. Everything else stays `{"default": true}`.
4. **The assistant didn't map vars itself.** It created collection link `ZcUku94` with the exact keys (`buyer_full_name`, `buyer_address`, ...), and the link saved into profile 9. So the var-to-key mapping was known, but `request_fill` was called without `paths`. Nothing in the instructions, the link's `next` hint (`mcp/tools.rs:137`) or the `request_fill` response pushes the assistant to check this.

Goal: bad bindings are rejected or flagged, and the MCP context tells the assistant to map every var to a vault path before `request_fill`. `request_fill` refuses to open the form while a var would still fall back to a useless template default.

## Changes

### 1. `bare_path` helper (`templates/bindings.rs`)
- `pub fn bare_path(expression: &str, roles: &HashSet<String>) -> Option<String>`: returns the trimmed text when the expression has no tag regions and matches `role(.ident)+` with a known role.
- Unit test next to `referenced_paths_follow_the_chains`.

### 2. Reject bare paths on save (`templates/mod.rs` `Templates::save`)
- Check every `spec.fields` value with `bare_path(expr, spec.roles.keys())`. On a match, return an error naming each offending field: `buyer_name: "buyer.buyer_full_name" is plain text; write "{{ buyer.buyer_full_name }}"`.
- This covers `save_template`, `save_docx_template` and `save_pdf_overlay_template`, which all go through `save`. Existing manifests still load, because only saving is checked.

### 3. Flag them in `check_bindings` (`templates/engine.rs` `check`)
- Before scanning paths, if `bare_path` matches, set `status = "literal"` and `error = Some("plain text, not a binding; wrap it as {{ ... }}")`, then still check that the key exists.
- Update the tool description: mention status `literal`.

### 4. `request_fill` reports what is still uncovered (`mcp/tools.rs`)
- Before creating the request, work out for each var the binding the form would start from. Use the same order as `app/fill.rs:open`: explicit `paths`/`values`, else the saved fill set, else `fill::default_binding`.
- A var is **uncovered** when it falls to `{"default": true}` and its default is either a bare path (`bare_path`) or references a key missing from the role's profile. A computed default over existing keys (`{{ manager.name | upper }}`) or with no paths (`{{ today() }}`) counts as covered.
- If any var is uncovered and the new arg `allow_defaults: bool` (default false) isn't set, return an error listing each uncovered var with its default and the profile keys available for each role. The request isn't created. The assistant re-calls with `paths`.

### 5. MCP text (`mcp/tools.rs`)
- `INSTRUCTIONS` workflow step 7 and docx step 3 get one line each: "Before request_fill, compare the template's vars (get_template) with the profile keys (describe_profile) and pass a path for every var a key answers, e.g. {"buyer_name": "buyer.buyer_full_name"}. Don't rely on template defaults that are not exactly {{ role.key }}."
- Step 3 (save_template): "Every binding is Jinja: "{{ buyer.name }}", never a bare buyer.name."
- Collection links, step 5: "...then request_fill with that profile, passing paths that map each template var to the key you asked for in the link."
- `request_fill` description: same mapping sentence, plus `allow_defaults`.
- `next_step` (`mcp/tools.rs:137`): change `use_them` to "use describe_profile, then request_fill with paths mapping each template var to these keys. Do not ask the user to paste the values."

### 6. Repair the existing template (data, after the code lands)
- Through MCP, re-save `real_estate_purchase_offer` with `save_template` and no source (keeps `main.typ`), wrapping all 10 bindings in `{{ }}`. Then run `check_bindings` with buyer=9, then `request_fill` with paths. Profile 9 has `additional_conditions` empty (length 0), so mention that to the user.

## Tests
- `bindings.rs`: `bare_path` cases (bare match, unknown role, wrapped, literal text, filter expression).
- `templates/mod.rs`: save rejects a bare path, accepts `{{ role.key }}`.
- `engine.rs`: `check` reports `literal` for a bare binding.
- `mcp/tools.rs` `request_fill_validates_input`: uncovered vars error without `allow_defaults`; passes with paths; passes with `allow_defaults: true`.
- Update the `next_step` expectations if any test asserts on its text.

## Verification
- `cd src-tauri && cargo test`
- Run linting afterwards: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`.
- Manual: `cargo run` with `BLANKEY_DATA_DIR` set to the real Blankey dir. Then over MCP:
  - `check_bindings` on the current broken template shows 10 `literal` problems.
  - Re-saving with bare paths fails.
  - Re-saving with `{{ }}` passes.
  - `request_fill` without paths returns the uncovered list.
  - With paths, the form opens with every buyer field on its vault path.
