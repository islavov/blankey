# RFC: Profiles from existing documents, without the assistant seeing values

Status: proposal. Builds on docs/pii-detection-rfc.md (the `pii` module, placeholders, user corrections).

## 1. Idea

The user's folders are full of filled documents: signed contracts, powers of attorney, invoices. Each one holds the
data a profile needs: a party's name, EGN, ID card, address, IBAN. Today that data reaches the vault only through
typing (`request_profile_input`, the fill form) or collection links.

The PII detector already finds these values and groups them by party. Once the user has reviewed a file, the
assistant reads it as placeholders (`⟦person:1⟧, ЕГН ⟦egn:1⟧, наричан по-долу ПРОДАВАЧ`). This is enough context for
the assistant to **label** the values: which party each belongs to, which profile key it is, what type, and whether
the party is a person or a company. It never needs the values themselves.

So the assistant proposes profiles in placeholder form. Blankey resolves the placeholders locally, shows the user a
prefilled profile form, and the user confirms it. Values move from file to vault inside Blankey. The assistant gets
back only what it already gets from `request_profile_input` today: saved or skipped per key, and the profile id.

```
file --detect + corrections--> placeholders --assistant labels--> request_profile_input
                                                                    { source: "lib:12",
                                                                      new_profile_name: "⟦person:1⟧",
                                                                      fields: [{key: "egn", type: "egn",
                                                                                suggested: "⟦egn:1⟧"}, ...] }
     --UI resolves placeholders locally--> prefilled form --user confirms--> vault
     --wait_request--> {profile_id, fields: [{key, status}]}
```

Why it fits the current direction:
- No new manual UI. The review stays review-only (mark and unmark values to hide), and the existing profile input
  sheet does the confirmation. The assistant decides what goes where.
- No profile writes from fill. This is a separate, explicit action, and the user confirms every profile.
- One round trip per party. The assistant already has the placeholders from `inspect_docx` or `inspect_pdf`.

## 2. User stories

1. "Make profiles for the parties in this lease." The assistant inspects the file, sees two parties, and sends two
   profile requests. The user gets two prefilled sheets, checks them, and saves.
2. While templatizing a contract, the assistant notices that the seller has no profile (no match, section 3.3) and
   offers to create one from the same file. The fill that follows can then bind `seller` to the new profile.
3. A party already has a profile, but without an ID card. The assistant sends a request with `profile_id` and only
   the missing keys. The sheet shows the document's value as a suggestion next to the empty field.
4. A value the detector missed was hidden by the user in the review ("Номер на членска карта: ⟦marked:1⟧"). The
   assistant reads the label, names the key `membership_card`, and includes it.

## 3. Design

### 3.1 Placeholders in `request_profile_input`

Two additions to the tool, nothing else changes:

- `source`: a file the assistant can already read (`lib:<id>`, an upload, or a remote grant), checked the same way as
  `source_path` in `inspect_docx`.
- With `source`, `suggested` and `new_profile_name` may contain placeholders. A `suggested` that is exactly one
  placeholder resolves to that entity's value. A `new_profile_name` may mix text and placeholders (`⟦org:1⟧`,
  `⟦person:1⟧ (наемател)`).

```rust
pub struct RequestProfileInputArgs {
    fields: Vec<Map<String, Value>>,
    reason: Option<String>,
    profile_id: Option<i64>,
    new_profile_name: Option<String>,
    new_profile_kind: Option<String>,
    /// File whose placeholders (⟦kind:n⟧) appear in suggested and new_profile_name.
    source: Option<String>,
}
```

At request creation the handler rebuilds the `Document` (with the file's corrections) and checks that every
placeholder names a live entity. An unknown id is an error that names the id, never a value. The payload stores the
placeholders, `source`, and the file hash. It never stores values.

Values are resolved when the sheet opens. `next_request` rebuilds the `Document` from `source`, replaces the
placeholders, and returns the resolved fields to the UI only:

```rust
// app/commands.rs, next_request, for PROFILE_INPUT with a source
let doc = app.pii_document(source)?;                 // same builder as inspect_docx / inspect_pdf, with corrections
let stale = library::hash_file(&path)? != payload.hash;
for field in &mut fields {
    if let Some(id) = pii::placeholder(field["suggested"].as_str()) {
        field["suggested"] = json!(doc.value(id, field_type(field)));   // None when stale or gone
        field["from_document"] = json!(true);
    }
}
```

If the file changed since the request, the placeholders may point at other values, so the sheet opens with no
suggestions and a note ("The file changed; type the values or ask again"). Entity ids are deterministic, so an
unchanged file always resolves to the same values.

### 3.2 From a surface to a field value

`Document::value(id, field_type)` picks one surface and makes it fit the field type. All of this lives in `pii`.

| field type | rule |
|---|---|
| egn, lnch, eik | digits only |
| iban | uppercase, no spaces |
| date | `fieldtypes::parse_date` on the surface without the trailing `г.`; the stored form follows `fieldtypes::coerce` |
| phone | as written, whitespace collapsed |
| email | lowercased |
| text | the most common surface; a name or company written only in capitals gets title case for each word, keeping the legal-form suffix (`ЕООД`, `Ltd`) as written |

The value only prefills the form. `save_profile_values` still validates it, so a wrong pick shows up as an ordinary
problem in the sheet.

### 3.3 Parties in the inspect output

The assistant can group entities by `role` today, but the grouping should come from the detector, which knows the
clusters. `inspect_docx` and `inspect_pdf` gain a `parties` list next to `pii`:

```json
"parties": [
  { "role": "seller", "kind": "person",
    "entities": ["person:1", "egn:1", "id_card:1", "address:1"],
    "match": { "profile_id": 7, "on": "egn" } },
  { "role": "buyer", "kind": "company",
    "entities": ["org:1", "eik:1", "address:2", "representative:1"] }
]
```

- `kind` is `company` when the cluster has an Org or an EIK, otherwise `person`. These are the profile kinds the
  vault already uses.
- `Representative` stays a field of the company (`representative`, from `keys.rs`), not a profile of its own. The
  assistant can still propose one.
- `match` is a local lookup: a party matches a profile when one of its strong identifiers (EGN, LNCh, EIK, IBAN,
  email) equals that profile's value after normalization. Name equality alone never matches. This tells the
  assistant "extend profile 7" instead of "create a duplicate", and it is the linkage the assistant needs for the
  fill anyway. It reveals no value. The query cannot be steered, because the assistant chooses the file but not the
  value that is compared.

The suggested keys stay the ones `keys.rs` gives (`full_name`, `egn`, `id_card.number`, ...), so a profile made from
a document binds to templates made from the same kind of document without any mapping.

### 3.4 The sheet

The existing `ProfileInput` sheet already prefills `suggested` and offers "Use suggestion" when a field has another
value. Three small changes:
- In the head: "From <file name>", with a link that opens the PII review for that file, so the user can check where a
  value came from.
- The prefilled fields keep the `suggested` style and get the title "From the document" instead of "Suggested by the
  assistant".
- The profile name field is prefilled from `new_profile_name`, resolved.

Several parties mean several requests, and `processRequests` already opens them one after another.

### 3.5 What the assistant is told

The tool descriptions change in two places:
- `inspect_docx` / `inspect_pdf`: "`parties` groups the placeholders by party; `match` is an existing profile that
  holds the same ID."
- `request_profile_input`: "With source, suggested and new_profile_name may be placeholders from that file
  (⟦egn:1⟧). Blankey fills them in for the user. To save a document's parties as profiles, send one request per
  party: profile_id for a match, otherwise new_profile_name."

The server instructions get one line, after the filling paragraph: "A party with no profile can be saved from a
reviewed file: request_profile_input with source and placeholders."

## 4. Privacy and safety

- Values never enter the MCP response, the request payload or the audit log. They exist only in the UI detail
  returned by `next_request` and in the vault after the user saves. This is the same boundary as typed profile input.
- Placeholders are accepted only in `suggested` and `new_profile_name`, and only with `source`. Elsewhere they are
  stored as text, as today.
- `source` goes through the same read checks as `inspect_docx`. A remote session can only name files it was granted.
- Error messages from resolution go through `doc.scrub`.
- What the user hid in the review is offered (`⟦marked:n⟧`). What the user showed (marked as not personal) is
  already visible to the assistant, so it does not need a placeholder.
- The `match` lookup compares inside Blankey and returns a profile id and a key name only.

## 5. Not in scope

- Creating profiles with no confirmation. Every profile goes through the sheet.
- A local "Create profiles" button with no assistant. The detector's parties and keys would be enough for one, and it
  could come later. For now the assistant does the labelling, as with templates.
- Splitting an address into `address.city`, `address.street` and so on. The detector sees an address as one value.
  The address grammar already knows its tokens (`гр.`, `ул.`, `п.к.`), so a later `Document::address_parts` could
  offer the parts as more placeholders (`⟦address:1.city⟧`).
- Scans. These need OCR first (a separate RFC).
- Bulk extraction across many files.

## 6. Changes

| where | change |
|---|---|
| `pii/document.rs` | `value(id, field_type)`, `parties()`; `pii::placeholder(&str)` parser shared with `resolve_finds` |
| `pii/resolve.rs` | expose the clusters that `roles` already builds, so `parties()` reuses them |
| `core.rs` | `pii_document(source)`: path, corrections and kind in one place (inspect tools, save tools, `next_request`); `match_party` against `vault.field_index` and values |
| `mcp/tools.rs` | `source` on `request_profile_input`, placeholder validation, hash in the payload; `parties` in the inspect outputs; descriptions |
| `app/commands.rs` | `next_request` resolves placeholders for `PROFILE_INPUT` with a source |
| `ui/js/requests.js` | source line, "From the document" title, link to the review |
| server instructions | one line (section 3.5) |

## 7. Testing

Fast unit tests with invented values only:
- `value`: EGN with spaces gives digits; an IBAN in groups; `12.03.1980 г.` as a date; a name in capitals gets title
  case; `„Пример“ ЕООД` keeps its suffix.
- `parties`: a seller cluster and a buyer company with a representative give two parties with the right kinds and
  entity lists.
- `match`: a profile with the same EGN matches; a profile with the same name only does not.
- `request_profile_input`: an unknown placeholder is refused without echoing a value; the stored payload holds no
  surface of any entity.
- `next_request`: placeholders resolve on an unchanged file; a changed hash gives no suggestions.
- MCP round trip on `tests/data/filled.docx`: inspect, then request with placeholders, then resolve with the values the
  UI would send. The tool outputs and the payload contain no value text, and the saved profile holds the values.

End to end: `cargo run` with `BLANKEY_DATA_DIR` set to the real data dir. Ask the assistant to save the parties of a
reviewed contract as profiles, check the sheets, save, then fill a template from the same contract with the new
profiles. Then `cargo clippy` and `cargo fmt --check`.

## 8. Open questions

1. **One request per party, or one request for all of them?** One per party reuses everything as it is. A
   `profiles: [...]` form would be a single call and a single sheet with sections. I recommend one per party first.
2. **Conflicts on an existing profile.** When the document's value differs from the stored one (an ID card renewed
   since the contract), the sheet keeps the stored value and offers the document's value as a suggestion. Is that the
   right default, or should a newer document win?
3. **Remembering the source.** Should the profile record which file a value came from (a metadata key, no value), so
   the user can see why a field holds what it holds?
