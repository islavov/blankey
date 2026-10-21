# RFC: Local PII detection in document text (T1 patterns, T2 anchors)

Status: draft.

## 1. Context

Blankey turns filled documents into templates through the AI over MCP. The AI calls `inspect_docx` or `inspect_pdf`, reads the text, picks the spans that hold values, and calls `save_docx_template` or `save_pdf_overlay_template`. This has two problems:

1. **Leak.** `inspect_docx` (`src/mcp/tools.rs:909`) returns paragraph text and run texts verbatim. `inspect_pdf` returns the printed lines of flat PDFs, plus page images. For a filled contract, that sends names, EGNs, ID card numbers and addresses to the model. This contradicts the server contract: "You never see values".
2. **Work the AI should not need to do.** Finding "the EGN after ЕГН" is deterministic. A local detector does it faster, without errors, and without seeing the value.

This RFC specifies a local, model-free detector for **text** (docx paragraphs and text-layer PDF lines), in two tiers:

- **T1 (patterns):** regex shapes plus checksums, for identifiers that validate themselves.
- **T2 (anchors):** per-language label lexicons ("ЕГН", "л.к. №", "с адрес", "наричан по-долу Продавач") that classify and delimit values a pattern alone cannot: ID card numbers, names, addresses, issuers.

It also specifies where the detector plugs in:
- in the MCP tools: redaction plus binding suggestions in the inspect tools, and placeholder resolution in the save tools;
- in the preview UI: detected values are highlighted, the user marks more with the mouse and assigns vars, and the result is saved as a reusable template, with no AI involved.

Both paths share one interface: `Document` plus `Mark`, then `compile`, then save.

Everything lives in one self-contained module, `src/pii/`: detection, validators, packs, keys, and the `Document` adapters for docx and PDF, and mark compilation. Call sites outside it are thin (section 4.0.1). Removing the module and those calls removes the feature.

### Goals
- Bulgarian and English from day one. A new language means adding a YAML pack, not code, unless the language has a new checksummed ID.
- High precision for suggestions, high recall for redaction.
- Deterministic and stateless. The same file and the same packs always give the same entities and placeholder ids.
- Cheap: linear in text length, regexes compiled once. It never runs on the fill path.

### Non-goals (later RFCs)
- OCR for scans (T-OCR) and the NER model (T3). The `Detector` interface below leaves room for both as extra span sources.
- Redacting filled AcroForm values in `inspect_pdf` form mode.
- Learning from user corrections beyond the optional label-only user pack (section 4.4).

## 2. Concepts

```rust
/// What a span is. `class` decides redaction.
enum Kind {
    // personal: always redacted
    Person, Org, Address, Egn, Lnch, Eik, VatId, Iban, Bic, IdCard, IdIssueDate, IdIssuer,
    Passport, BirthDate, Email, Phone, Postcode, Plate, Ssn, Nino,
    // variable: suggested as template vars, never redacted
    Date, Money, DocNumber,
}
enum Class { Personal, Variable }   // Kind::class()

/// One hit in one block of text. Offsets are byte offsets into the block's original text.
struct Detection {
    kind: Kind,
    start: usize, end: usize,
    score: f32,              // 0..1
    source: Source,          // Pattern(recognizer id) | Anchor(label) | Suffix(org suffix) | Chain(address)
}

/// A distinct value across the document: all detections whose normalized value is equal.
struct Entity {
    id: String,              // "egn:1": kind + 1-based ordinal by first appearance
    kind: Kind,
    norm: String,            // normalized value, kept in memory only, never serialized to MCP
    surfaces: Vec<String>,   // distinct original spellings ("8001010000", "800101 0000")
    occurrences: Vec<Occ>,   // (block, start, end)
    role: Option<String>,    // "seller", "party_1", ...
    key: String,             // suggested profile key: "egn", "full_name", "id_card.number"
    score: f32,              // max over occurrences
    crosses_runs: bool,      // docx only: some occurrence spans more than one run
}

/// Text in, the unit the detector reads. Docx: one paragraph. PDF: one page (lines joined by '\n').
struct Block {
    text: String,
    /// Byte boundaries of runs (docx) or lines (pdf), so a span can be mapped back.
    cuts: Vec<usize>,
}
```

## 3. Pipeline

```
blocks --normalize--> folded text + offset map
       --T1--> pattern detections
       --T2--> anchor detections, plus boosts to T1 detections
       --resolve--> non-overlapping detections
       --entities--> dedupe by normalized value, assign ordinal ids
       --parties--> role per entity, suggested key and var
       --> Analysis { blocks, entities }
```

```rust
pub fn analyze(blocks: &[Block], packs: &Packs) -> Analysis {
    let mut per_block = Vec::new();
    for (b, block) in blocks.iter().enumerate() {
        let f = Folded::new(&block.text);            // section 3.1
        let mut hits = t1::scan(&f);                 // section 3.2
        hits.extend(t2::scan(&f, packs, &mut hits)); // section 3.3 (also boosts hits)
        per_block.push(resolve(hits));               // section 3.4
    }
    let mut entities = group(blocks, &per_block);    // section 3.5
    parties::assign(blocks, &per_block, &mut entities, packs); // section 3.6
    Analysis { per_block, entities }
}
```

### 3.1 Normalization

- `Folded` is NFC plus lowercase, the same rule as `mcp::find::fold`, together with a `Vec<usize>` that maps each folded byte offset back to an original byte offset. Lowercasing can change byte length, so the map is required. Every detection is reported in original offsets.
- Typographic variants are folded to one form: `„“”«»` -> `"`, `–—` -> `-`, NBSP -> space, `№` stays `№`, and both `No.` and `Nr.` are recognized as labels.
- Values are not rewritten in the text. Each kind has its own normalizer, used only for comparing values: digits only for numeric IDs, uppercase without spaces for IBAN, collapsed whitespace and casefold for names and addresses, digits only after the country code for phones.

### 3.2 T1: pattern recognizers

A static registry in `pii/t1.rs`, compiled once (`LazyLock`, as in `library/extract.rs`). Each recognizer:

```rust
struct Recognizer {
    id: &'static str,
    kind: Kind,
    re: Regex,                       // matched on the original text (case-sensitive where shape needs it)
    check: fn(&str) -> Check,        // Valid | Shape | Invalid
    alone: f32,                      // score without an anchor; 0 = needs an anchor to count at all
}

fn scan(f: &Folded) -> Vec<Detection> {
    REGISTRY.iter().flat_map(|r| r.re.find_iter(f.original()).filter_map(|m| {
        match (r.check)(m.as_str()) {
            Check::Invalid => None,
            Check::Valid | Check::Shape if r.alone > 0.0 => Some(det(r, m, r.alone)),
            _ => Some(det(r, m, 0.0)),   // kept at 0, so T2 can lift it
        }
    })).collect()
}
```

Initial registry (BG + EN). New checksum validators (LNCh, EIK, EGN date decode) live in `pii/validate.rs`. The module reuses `fieldtypes::valid_egn`, `valid_iban` and `valid_email` (the last one made `pub`) instead of copying them. `fieldtypes.rs` gets no new code.

| id | kind | shape (simplified) | check | alone |
|---|---|---|---|---|
| egn | Egn | `\b\d{10}\b` | `valid_egn` + birth date decodes (month 1-12, 21-32, 41-52) | 0.80 |
| lnch | Lnch | `\b\d{10}\b` | LNCh checksum (weights 21,19,17,13,11,9,7,3,1, mod 10) | 0.0 |
| eik | Eik | `\b\d{9}(\d{4})?\b` | BULSTAT 9/13-digit checksum | 0.0 |
| vat_bg | VatId | `\bBG\d{9,10}\b` | the digits pass eik or egn | 0.95 |
| iban | Iban | `\b[A-Z]{2}\d{2}(?:\s?[A-Z0-9]{4}){2,7}(?:\s?[A-Z0-9]{1,3})?\b` | `valid_iban` | 0.99 |
| bic | Bic | `\b[A-Z]{4}[A-Z]{2}[A-Z0-9]{2}(?:[A-Z0-9]{3})?\b` | country code is a known one | 0.0 |
| email | Email | `[^\s@<>()]+@[^\s@<>()]+\.[a-z]{2,}` | `valid_email` | 0.95 |
| phone_bg | Phone | `(?:\+359\|00359\|0)\s?8[789](?:[\s-]?\d){7}` | none | 0.85 |
| phone | Phone | `\+\d{1,3}(?:[\s\-()]?\d){6,13}` | 8 to 15 digits | 0.50 |
| id_card | IdCard | `\b\d{9}\b` | none | 0.0 |
| passport | Passport | `\b[A-Z]{0,2}\d{6,9}\b` | none | 0.0 |
| ssn | Ssn | `\b(?!000\|666\|9\d\d)\d{3}-(?!00)\d{2}-(?!0000)\d{4}\b` | none | 0.85 |
| nino | Nino | `\b[A-CEGHJ-PR-TW-Z]{2} ?\d{2} ?\d{2} ?\d{2} ?[A-D]\b` | none | 0.85 |
| plate_bg | Plate | `\b[АВЕКМНОРСТУХABEKMHOPCTYX]{1,2} ?\d{4} ?[АВЕКМНОРСТУХABEKMHOPCTYX]{2}\b` | none | 0.85 |
| date | Date | `\d{1,2}\.\d{1,2}\.\d{4}(?: ?г\.?)?`, `\d{4}-\d{2}-\d{2}`, `\d{1,2} <month> \d{4}` (months from the packs) | `parse_date`-style calendar check | 0.90 |
| money | Money | `\d[\d \.,]*\s?(?:лв\.?\|лева\|BGN\|EUR\|евро\|€\|\$\|USD\|GBP\|£)` | none | 0.90 |

Notes:
- A 10-digit run is tried as egn, then lnch. A number that is both a valid EGN and a valid LNCh stays Egn unless an LNCh anchor claims it (section 3.4).
- About 10% of random digit strings pass the EGN checksum. The date check cuts that to roughly 4%, so a standalone EGN scores 0.80, and 0.99 when anchored.
- Entries with `alone = 0` exist only so that T2 can claim them. Bare 9-digit numbers are never PII by shape alone.

### 3.3 T2: anchors (per-language packs)

Packs are YAML files compiled into the binary (`include_str!`, as with `categories.yaml`). All packs always run. A pack whose `script` does not appear in the text is skipped, which is a cheap check.

```yaml
# src/pii/packs/bg.yaml
lang: bg
script: cyrillic
months: [януари, февруари, март, април, май, юни, юли, август, септември, октомври, ноември, декември]
anchors:
  - { kind: egn,      labels: [егн, е.г.н.],                         value: { pattern: egn } }
  - { kind: lnch,     labels: [лнч, л.н.ч.],                         value: { pattern: lnch } }
  - { kind: eik,      labels: [еик, булстат, еик/булстат],             value: { pattern: eik } }
  - { kind: id_card,  labels: [л.к. №, л.к., лична карта №, лична карта, лк №], value: { pattern: id_card } }
  - { kind: id_issue_date, labels: [издадена на, изд. на, изд.],      value: { pattern: date } }
  - { kind: id_issuer, labels: [от мвр, от мвр -],                    value: { grammar: until_delim, max: 40 }, include_label: true }
  - { kind: passport, labels: [паспорт №, паспорт],                  value: { pattern: passport } }
  - { kind: birth_date, labels: [роден на, родена на, дата на раждане], value: { pattern: date } }
  - { kind: address,  labels: [с постоянен адрес, с адрес, постоянен адрес, адрес, живущ в, живуща в,
                               седалище и адрес на управление, адрес на управление], value: { grammar: address } }
  - { kind: person,   labels: [г-н, г-жа, представлявано от, представлявана от, представляван от,
                               управител, трите имена, име], value: { grammar: person_name } }
  - { kind: iban,     labels: [iban, банкова сметка, с/ка],           value: { pattern: iban } }
  - { kind: bic,      labels: [bic, swift],                          value: { pattern: bic } }
  - { kind: phone,    labels: [тел., телефон, gsm, мобилен],          value: { grammar: phone_any } }
  - { kind: email,    labels: [имейл, e-mail, ел. поща],              value: { pattern: email } }
  - { kind: postcode, labels: [п.к., пощенски код],                   value: { regex: '\d{4}' } }
address_tokens: [гр., град, с., село, обл., област, общ., община, ул., улица, бул., булевард, ж.к., кв., бл., блок,
                 вх., ет., етаж, ап., апартамент, №, п.к.]
org_suffixes: [еоод, оод, ад, еад, ет, сд, кд, кда, дззд, сдружение, фондация]
honorifics: [г-н, г-жа, гн, гжа]
roles:
  seller: [продавач]
  buyer: [купувач]
  landlord: [наемодател]
  tenant: [наемател]
  employer: [работодател]
  employee: [служител, работник]
  principal: [упълномощител, възложител]
  agent: [пълномощник, изпълнител]
  lender: [заемодател]
  borrower: [заемател, кредитополучател]
defined_term: ['наричан(?:а|о|и)? (?:по-долу|за краткост)']
stop_labels: [егн, лнч, л.к., тел., e-mail, наричан, наричана]   # end an address or name chain
```

`en.yaml` has the same shape: `ssn`, `social security no.`, `date of birth`, `dob`, `passport no.`, `driver's licence no.`, `address`, `residing at`, `of`, `mr.`, `ms.`, `mrs.`, `dr.`, `name:`, `represented by`. Address tokens are `street`, `st.`, `avenue`, `ave.`, `road`, `rd.`, `suite`, `apt.`, `unit`, `floor`. Org suffixes are `ltd`, `llc`, `inc.`, `plc`, `gmbh`, `corp.`. Roles are `seller`, `buyer`, `landlord`, `tenant`, `employer`, `employee`, `lender`, `borrower`, `the company`, `the client`. The defined term is `(?:hereinafter|hereafter)(?: referred to as)?`.

Label matching: one alternation regex per pack over all labels. Labels are folded, longest first, and must sit on word boundaries.

```rust
fn scan(f: &Folded, packs: &Packs, t1: &mut [Detection]) -> Vec<Detection> {
    let mut out = vec![];
    for pack in packs.active_for(f) {
        for label in pack.labels.find_iter(f.folded()) {
            let anchor = pack.anchor_for(label);
            let window = f.after(label.end(), WINDOW /* 48 chars */)
                          .skip_leading(&[' ', ':', '-', '№', '.', ',']);
            match &anchor.value {
                Value::Pattern(id) => {
                    // Claim the first T1 hit of that recognizer starting inside the window.
                    if let Some(d) = t1.iter_mut().find(|d| d.recognizer == *id && window.contains_start(d.start)) {
                        d.kind = anchor.kind; d.score = d.score.max(ANCHORED /* 0.98 */);
                        d.source = Source::Anchor(label.text());
                    }
                }
                Value::Grammar(g) => if let Some((s, e)) = grammar::take(g, f, window, pack) {
                    out.push(Detection { kind: anchor.kind, start: s, end: e, score: g.score(), source: Source::Anchor(label.text()) });
                }
                Value::Regex(re) => /* same as Grammar with a pack-local regex, score 0.9 */,
            }
        }
        out.extend(org_by_suffix(f, pack));       // see below
        out.extend(address_chains(f, pack));      // unlabeled addresses, see below
    }
    out
}
```

Grammars, all in `src/pii/grammar.rs`:

- `person_name`: 2 to 4 tokens, each `\p{Lu}\p{Ll}+(?:-\p{Lu}\p{Ll}+)?` or all caps `\p{Lu}{2,}`. Initials `\p{Lu}\.` are allowed between them. It stops at a lowercase word, a digit, `,`, `;`, or a stop label. With a honorific the score is 0.90; with an "име"-type label it is 0.85. One token alone gives no detection.
- `address`: consume tokens while each token is an address token, a capitalized word, a number or number range (`12`, `12А`, `3-5`), a quoted name, or `,`. Stop at `;`, a blank line, a stop label, a role or defined-term marker, or after 160 chars. Trailing punctuation is trimmed. A valid address needs at least one address token and at least one number. Score 0.90.
- `until_delim`: from the window start to the first `,`, `;` or `\n`, capped at `max`. Score 0.80.
- `phone_any`: `[+\d][\d\s\-()/]{5,18}\d` with 6 to 15 digits. Score 0.90. Labels let numbers that do not look like BG mobiles count.

Two detectors run without a label:

- `org_by_suffix`: an optionally quoted sequence of up to 6 capitalized or quoted tokens immediately before or after an org suffix. For example `„Пример“ ЕООД`, `ЕТ „Пример – Иван Петров“`, `Example Ltd`. The span includes the suffix. Score 0.90.
- `address_chains`: two or more address tokens within 40 chars of each other start an address (for example `гр. София, ул. „Примерна“ № 1`). The span is expanded with the `address` grammar. Score 0.80.

### 3.4 Resolve overlaps (within one block)

```rust
fn resolve(mut hits: Vec<Detection>) -> Vec<Detection> {
    hits.retain(|d| d.score > 0.0);
    // Prefer: higher score, then validated sources (Pattern with Valid check, Anchor+Pattern), then longer span.
    hits.sort_by(|a, b| b.score.total_cmp(&a.score).then(rank(b.source).cmp(&rank(a.source))).then(b.len().cmp(&a.len())));
    let mut kept: Vec<Detection> = vec![];
    for d in hits {
        match kept.iter().find(|k| overlaps(k, &d)) {
            None => kept.push(d),
            // An address may contain a postcode, an org may contain a person (ЕТ „... Иван Петров“): keep the outer.
            Some(k) if contains(k, &d) && k.kind.can_contain(d.kind) => {}
            Some(_) => {}  // lower ranked, dropped
        }
    }
    kept.sort_by_key(|d| d.start);
    kept
}
```

### 3.5 Entities

```rust
fn group(blocks, per_block) -> Vec<Entity> {
    let mut by_key: IndexMap<(Kind, String), Entity> = IndexMap::new();   // keeps document order
    for (b, dets) in per_block.iter().enumerate() {
        for d in dets {
            let surface = &blocks[b].text[d.start..d.end];
            let norm = normalize(d.kind, surface);
            let e = by_key.entry((d.kind, norm.clone())).or_insert_with(|| Entity::new(d.kind, norm));
            e.add(b, d, surface, crosses(&blocks[b].cuts, d.start, d.end));
        }
    }
    // ids: "<kind>:<n>" with n counting per kind in first-appearance order
    number(by_key.into_values().collect())
}
```

A person's name written in a different case elsewhere ("ИВАН ПЕТРОВ" in the signature block) still folds to the same `norm`, so both are one entity.

### 3.6 Parties and suggested bindings

Legal text puts the role either before the party ("ПРОДАВАЧ: Иван Петров, ЕГН ...") or after it ("Иван Петров, ЕГН ..., наричан по-долу ПРОДАВАЧ"). Assignment works on clusters:

```rust
fn assign(blocks, per_block, entities, packs) {
    for (b, dets) in per_block.iter().enumerate() {
        // A cluster is a run of personal detections in one block with no sentence end ('.', ';' followed by
        // a capital, or a numbered clause start) between them.
        for cluster in clusters(&blocks[b].text, dets) {
            let role = role_in(&blocks[b].text, cluster.span(), packs)  // defined term or role label in the same sentence, before or after
                .or_else(|| heading_role(blocks, b, packs))              // a role label alone on one of the 2 previous blocks
                .unwrap_or_else(|| next_party_n());                      // "party_1", "party_2", ...
            for d in cluster { entity_of(d).role.get_or_insert(role.clone()); }   // first assignment wins
        }
    }
    for e in entities { e.key = KEYS[e.kind].to_string(); }
}
```

The role label maps to its pack role id (`продавач` -> `seller`). A defined term that is not in the pack ("наричан по-долу Довереник") becomes a slug of the term, transliterated.

Kind to suggested profile key (`src/pii/keys.rs`, shared by all languages):

| kind | key | kind | key |
|---|---|---|---|
| Person | full_name | Iban | iban |
| Org | name | Bic | bic |
| Address | address | Phone | phone |
| Egn | egn | Email | email |
| Lnch | lnch | Eik | eik |
| IdCard | id_card.number | VatId | vat_id |
| IdIssueDate | id_card.issued_on | Passport | passport.number |
| IdIssuer | id_card.issuer | BirthDate | birth_date |

The suggested var is `<role>_<key with . replaced by _>` (`seller_id_card_number`). The suggested field binding is `<role>.<key>`. Both use the existing conventions in the `save_docx_template` description. Variable-class kinds (Date, Money) get `role: null` and a var from their label context only when T2 found one ("дата на подписване" -> `signing_date`). Otherwise there is no var, and the AI names them.

## 4. Where it is used

### 4.0 Module layout

```
src/pii/
  mod.rs        public API only: Document, Mark, At, Draft, EntityView, guard, enabled
  types.rs      Kind, Class, Detection, Entity, Block, Folded (offset-mapped fold)
  validate.rs   lnch, eik (9/13), egn birth-date decode, bic country; reuses fieldtypes::{valid_egn, valid_iban, valid_email}
  t1.rs         recognizer registry
  t2.rs         pack loading (include_str!) and anchor scan
  grammar.rs    person_name, address, until_delim, phone_any, org_by_suffix, address_chains
  resolve.rs    overlaps, entities, parties
  keys.rs       kind -> profile key, var and binding naming
  redact.rs     placeholders, text and run redaction, scrub, guard, mask_png
  document.rs   Document: block builders for docx paragraphs and PDF pages, entities, expand, html/pages
  marks.rs      At, Mark, Mark::from_replacement, UTF-16 conversion, selection snapping, compile -> Draft
  packs/bg.yaml, packs/en.yaml
  tests.rs
```

### 4.0.1 Public interface: `Document` plus `Mark`

There are two consumers, and they do the same job. The AI over MCP and the user in the preview UI (section 4.4) both turn a filled document into a template by deciding which spans become which vars. So the interface centres on one source-bound object and one shared description of a decision, the **mark**. Typed outputs are `Serialize`; `pii` never builds MCP JSON or HTML strings outside its own render helpers.

```rust
/// Where a var goes. Every consumer speaks this.
pub enum At {
    Entity(String),                                   // "egn:1": every occurrence of a detected entity
    Span { block: usize, start: usize, end: usize },  // a user selection or one occurrence; char offsets in the block text
    Find { text: String, occurrence: Option<usize> }, // the AI's raw find, as accepted by save_docx_template today
    Rect { page: usize, rect: [f64; 4] },             // a PDF box drawn by the user
}

pub struct Mark {
    pub at: At,
    pub var: String,                // "seller_egn"
    pub binding: Option<String>,    // "seller.egn"; None = the user fills it at fill time
    pub label: Option<String>,
    pub all: bool,                  // Span/Find: also mark every other occurrence of the same normalized value
}

pub struct Document { /* source, blocks, analysis; private */ }

impl Document {
    pub fn docx(path: &Path) -> Result<Self, String>;
    pub fn pdf(bytes: &[u8]) -> Result<Self, String>;         // text-layer pages via preview::layout_lines

    // reading
    pub fn entities(&self) -> Vec<EntityView>;                // id, kind, role, var, binding, label, length, occurrences, places, score
    pub fn suggested(&self) -> Vec<Mark>;                     // one At::Entity mark per entity above SUGGEST

    // MCP (sections 4.1 to 4.3)
    pub fn redacted_paragraphs(&self) -> Vec<ParagraphInfo>;  // docx
    pub fn redacted_pages(&self) -> Vec<PageText>;            // pdf: lines with rects
    pub fn masks(&self, page: usize) -> Vec<[f64; 4]>;        // personal rects to paint on page images
    pub fn scrub(&self, message: &str) -> String;             // remove surfaces from error text

    // UI (section 4.4)
    pub fn expand(&self, mark: &Mark) -> Vec<Occurrence>;     // what a mark covers: block/span or page/rect per occurrence
    pub fn html(&self, marks: &[Mark]) -> String;             // docx preview with entity and mark highlights, see 4.4
    pub fn pages(&self, marks: &[Mark], dpi: f32) -> Result<Vec<PagePreview>, String>; // pdf preview with boxes

    // both
    pub fn compile(&self, marks: &[Mark]) -> Result<Draft, String>;
}

/// What templates.save needs, built from marks. The only path from marks to a template.
pub enum Draft {
    Docx { replacements: Vec<Value>, fields: StrMap, labels: StrMap, roles: StrMap },  // replacements in tokenize's {find, var, occurrence} form, longest first
    Overlay { boxes: Vec<Value>, fields: StrMap, labels: StrMap, roles: StrMap },     // boxes in save_pdf_overlay_template's form
}

pub fn guard(text: &str) -> Cow<str>;   // T1-checksum-only scrub for any outgoing MCP text (safety net)
pub fn enabled() -> bool;               // reads config pii_redaction
```

Why this shape:
- MCP and UI produce the same `Mark`s, so there is one `compile` and one save path, and they cannot drift apart. The AI's placeholder `find`s (`⟦egn:1⟧`) parse to `At::Entity`, and plain finds parse to `At::Find`.
- `Document` keeps source specifics (runs, lines, places) inside `pii`. Callers handle only `Mark`, views and `Draft`.
- It is stateless across calls. A UI session re-sends its marks and the `Document` is rebuilt from the file. The analysis is deterministic, so entity ids stay stable.
- `guard` is defence in depth. `ToolServer::run` applies it to every text block a tool returns, so a tool added later that echoes document text still cannot leak checksummed IDs.

Changes outside the module:
- `src/lib.rs`: `mod pii;`.
- `src/mcp/tools.rs`: `inspect_docx`, `inspect_pdf` (flat branch) and `save_docx_template` use `Document`; `run` applies `pii::guard`; tool descriptions updated.
- `src/app/commands.rs`: two Tauri commands for conversion (section 4.4).
- `src/templates/docx.rs`: `to_html` gains a mark hook (section 4.4). The tokenize cross-run change is pending open question 1.
- `ui/js`: conversion mode in the preview (section 4.4).
- `src/config.rs`: `pii_redaction: bool`, default `true`. It is a user setting; the AI cannot turn it off through a tool argument.
- `src/fieldtypes.rs`: `valid_email` becomes `pub`.

Image masking happens inside `pii`. It decodes the PNG that `preview::grid_page` already returns, paints `masks(page)` with `tiny-skia`, and re-encodes it, so `render/preview.rs` stays unchanged.

### 4.1 `inspect_docx`: redact and suggest

Today the tool returns `Vec<ParagraphInfo>` (`templates/docx.rs:195`). New output shape (a breaking change to the tool, which is fine because the only client is the AI):

```json
{
  "paragraphs": [
    { "index": 3, "location": "body",
      "text": "1. ⟦person:1⟧, ЕГН ⟦egn:1⟧, л.к. № ⟦id_card:1⟧, с адрес ⟦address:1⟧, наричан по-долу ПРОДАВАЧ",
      "runs": ["1. ", "⟦person:1⟧", ", ЕГН ", "⟦egn:1⟧", "..."], "tags": [] }
  ],
  "pii": [
    { "id": "egn:1", "kind": "egn", "role": "seller", "var": "seller_egn", "binding": "seller.egn",
      "label": "ЕГН", "length": 10, "occurrences": 2, "score": 0.99, "crosses_runs": false }
  ]
}
```

The handler in `tools.rs`:

```rust
let path = source_path(app, remote.as_ref(), &a.path)?;
if !pii::enabled() { return text(&json!({ "paragraphs": docx::inspect(&path)?, "pii": [] })); }
let doc = pii::Document::docx(&path)?;
text(&json!({ "paragraphs": doc.redacted_paragraphs(), "pii": doc.entities() }))
```

Inside `redacted_paragraphs`, each block's detections at or above `REDACT` (0.40) are replaced by their entity placeholder. In `runs`, the placeholder goes in the first run of the span and the rest of the span is removed. `entities()` lists entities at or above `SUGGEST` (0.70); lower ones are redacted but not suggested.

- Redaction covers only Personal-class kinds. Dates and money stay visible, because the AI needs them as context and they are not personal.
- `pii` never contains `norm` or `surfaces`, only `length`, matching how profile values are described to the AI.
- The tool description is updated: "Values that look personal appear as ⟦kind:n⟧. Pass these placeholders as `find` in save_docx_template. `pii` lists suggested var and binding per placeholder."

### 4.2 `save_docx_template`: resolve placeholders locally

The AI passes `{"find": "⟦egn:1⟧", "var": "seller_egn"}`. A placeholder may also sit inside a longer find, such as `"ЕГН ⟦egn:1⟧"`. Resolution is stateless: the `Document` is rebuilt from the same source, so the ids resolve the same way.

```rust
// tools.rs, save_docx_template
let doc = pii::Document::docx(&source)?;
let marks = a.replacements.iter().map(pii::Mark::from_replacement).collect::<Result<Vec<_>, _>>()?;
let draft = doc.compile(&marks)?;          // Draft::Docx { replacements, .. }; a.fields/labels/roles win over the draft's
let origin = origin(app, Some(&a.source_path), draft.replacements.clone(), None)?;
docx::tokenize(&source, &scratch, &draft.replacements).map_err(|e| doc.scrub(&e))?;
```

`Mark::from_replacement`: a find that is exactly one placeholder becomes `At::Entity`. A find with a placeholder inside it becomes `At::Find`, with the placeholder kept for `compile` to expand. Anything else becomes `At::Find` as is.

`compile` for docx:

```rust
fn compile_docx(&self, marks: &[Mark]) -> Result<Draft, String> {
    let mut out = vec![];
    for m in marks {
        for occ in self.expand(m) {                       // Entity -> its occurrences; Span{all} -> same-norm occurrences
            let surface = self.surface(&occ);
            let find = m.find_template().map_or(surface.clone(), |t| t.replace_placeholder(&surface));
            out.push(json!({ "find": find, "var": m.var, "occurrence": self.nth_of(&find, &occ) }));
        }
    }
    dedupe(&mut out);                                     // the same (find, occurrence) marked twice
    out.sort_by_key(|r| Reverse(r["find"].as_str().map_or(0, str::len)));   // longer spans first, as tokenize requires
    Ok(Draft::Docx { replacements: out, fields: bindings(marks), labels: labels(marks), roles: roles(marks) })
}
```

- `nth_of(find, occ)` counts the occurrences of `find` across the document text, in the same order `tokenize` walks paragraphs, and returns the 1-based index of `occ`. This turns any mark into tokenize's `{find, occurrence}` form.
- `roles(marks)` comes from binding prefixes (`seller.egn` -> role `seller`), with the description taken from the pack's role label.
- The stored origin keeps resolved surfaces, so rebuild-on-change behaves as it does today.
- `scrub` removes every entity surface from `tokenize` error reports.
- Occurrences with `crosses_runs: true` cannot be tokenized today ("a span must sit inside one run"). See section 7, question 1.

### 4.3 `inspect_pdf`, flat mode: redact lines and mask images

- Build one `Block` per page from `preview::layout_lines` (lines joined by `\n`, `cuts` = line starts). Analyze, then redact each `line.text` the same way as in docx. Line rects stay as they are.
- Each `pii` entry gains `places`: `preview::find_text` run on the entity's surfaces gives `[page, [x0,y0,x1,y1]]`. The AI can then place overlay boxes over the value without seeing it, and `save_pdf_overlay_template` needs no change.
- Page images: every PNG from `preview::grid_page` is masked with `doc.masks(page)` before it is returned (section 4.0.1). Without this, redacting the text would be pointless.
- In `tools.rs` the flat branch becomes `let doc = pii::Document::pdf(&pdf)?;`, then `doc.redacted_pages()` for the lines and `doc.entities()` for `pii`. The existing image loop calls `pii::mask_png(grid_page(..)?, &doc.masks(page), dpi)`.
- Form mode (AcroForm) is unchanged in this RFC.

### 4.4 Template conversion in the preview (UI)

Goal: the user opens a library file, sees the detected values highlighted, adds or removes marks with the mouse, assigns each mark a var, and saves it as a reusable template. No AI is involved.

**Flow**
1. In the library preview, a "Make template" action switches `Preview` (`ui/js/preview.js`) into conversion mode.
2. The UI calls `convert_preview(id, marks = [])`. The backend builds a `Document`, takes `doc.suggested()` as the initial marks, and returns the preview with highlights.
3. The user works the marks:
   - Click a highlight to edit its var or binding, or to remove it (a false positive).
   - Select text with the mouse (docx) or drag a box (PDF), then pick a var from a small popover. The popover offers the entity's suggestion, recent vars from this session, and free text.
   - "All occurrences" is on by default for a selection (`Mark.all`), so marking one EGN marks every copy of it.
4. Each change re-calls `convert_preview(id, marks)`. It is stateless and cheap: the file is re-read and re-analyzed. Typical contracts take a few ms, and nothing here runs on the fill path.
5. "Save" calls `convert_save(id, template_id, name, marks)`, which runs `doc.compile(marks)` and then the existing `templates.save` path used by `save_docx_template` and `save_pdf_overlay_template`.

**Tauri commands** (`src/app/commands.rs`, both through `spawn_blocking` like `library_preview`):

```rust
#[tauri::command]
async fn convert_preview(state, id: i64, marks: Vec<Mark>) -> CmdResult<ConvertView> {
    let file = state.app.library.file(id)?;
    let doc = pii::Document::open(&file.path, &file.kind)?;
    let marks = if marks.is_empty() { doc.suggested() } else { marks };
    Ok(ConvertView {
        preview: match file.kind { "docx" => Preview::Html { html: doc.html(&marks), names }, _ => Preview::Pages { pages: doc.pages(&marks, DPI)?, names } },
        marks,                                   // normalized and echoed, so the UI holds the canonical list
        entities: doc.entities(),
        occurrences: marks.iter().map(|m| doc.expand(m).len()).collect(),
    })
}

#[tauri::command]
async fn convert_save(state, id: i64, template_id: String, name: String, marks: Vec<Mark>) -> CmdResult<Value> {
    let file = state.app.library.file(id)?;
    let doc = pii::Document::open(&file.path, &file.kind)?;
    match doc.compile(&marks)? {
        Draft::Docx { replacements, fields, labels, roles } => /* tokenize to scratch + templates.save, as in save_docx_template */,
        Draft::Overlay { boxes, fields, labels, roles }     => /* the same body as save_pdf_overlay_template */,
    }
}
```

To share the save bodies, the bodies of `save_docx_template` and `save_pdf_overlay_template` in `tools.rs` move into one function each on `Blankey` (in `templates/`). The MCP tools and the Tauri commands both call them.

**docx highlighting.** `doc.html(marks)` reuses `docx::to_html`. It does not copy the HTML renderer.
- `to_html` gains an optional decorator: `Option<&dyn Fn(usize /* block */, &str /* run text */, usize /* char offset in block */) -> String>`. With no decorator, the output is unchanged.
- Every `<p>` gets `data-b="<block index>"`. **Index alignment:** `read_body` numbers body paragraphs first and table-cell paragraphs after them (`templates/docx.rs:174-180`), while `to_html` walks in document order. The decorator must receive `read_body`'s index. A shared helper that maps each `w:p` node to its `read_body` index fixes this, with a test that asserts the same mapping for both walkers on a fixture with a table.
- Highlights are `<mark class="pii" data-m="<mark idx>" data-kind="egn" title="seller.egn">…</mark>`. Detected entities with no mark yet get `class="pii suggest"`.
- Selection to `At::Span`: the UI finds the enclosing `[data-b]`, then sums text-node lengths up to the selection's start and end to get offsets. JS offsets are UTF-16 code units, so the boundary contract is UTF-16 and `Mark` deserialization converts to char offsets. Cyrillic is in the BMP, so this matters only for rare characters, but it has to be correct.
- A selection is snapped outward to word boundaries, and inward past surrounding spaces and punctuation, before it becomes a mark. A quick, sloppy drag still produces a clean value.

**PDF highlighting.** `doc.pages(marks, dpi)` returns the existing `PagePreview { image, width, height, boxes }`, with `boxes` keyed by mark index or entity id. The UI already draws boxes over page images (`PdfPage` in `preview.js`). Dragging a rectangle gives `At::Rect`, in PDF points from the same transform the UI uses for boxes. A rect is snapped to the text-line rects it intersects (`layout_lines`), so it matches what `save_pdf_overlay_template` expects.

**Making it reusable.** The output is an ordinary template, so it shows up in `find_templates` and fills like any other. Two cheap extras close the loop:
- Var names come from `keys.rs` and the roles from the packs, so a lease converted in the UI and one converted by the AI get the same var names, and one fill covers both.
- Optional, later: a manual mark that sits right after a label the packs do not know (for example "Номер на членска карта:") can be offered to the user as a local anchor. It is saved to a user pack (`<data dir>/pii/user.yaml`, loaded after the built-in packs), so the next document with that label is detected automatically. That file holds labels only, never values.

### 4.5 Library metadata (optional, last)

`library/extract.rs` could count Personal entities per file and add readiness `filled` (no fields, no blanks, at least 3 personal entities with at least 1 role), so `find_templates` surfaces filled contracts as templatizable. This changes the readiness enum used by `find_templates` and the UI, so it ships separately.

### 4.6 Not used
- Not on the fill or render path. Fill speed is unaffected.
- No profile writes. Detection never creates or updates vault data.
- Remote (relay) sessions call the same tool handlers, so they get redaction automatically.

## 5. Adding a language

1. Add `src/pii/packs/<lang>.yaml` with labels, address tokens, org suffixes, roles, the defined-term regex and month names.
2. If the country has checksummed IDs, add the validator to `pii/validate.rs` and a row to `pii/t1.rs`, for example Romanian CNP or German Steuer-ID.
3. Add the pack to the `PACKS` include list.
4. Add one fixture paragraph per anchor to the pack's unit test.

T1 patterns are language-agnostic, and roles map to the shared ids, so templates made from documents in different languages share var names.

## 6. Testing

All tests are fast unit tests in `cargo test`, with invented values (English fixtures, and Bulgarian strings with invented names and valid-checksum fake IDs). No corpus or benchmark runs.

All of the following go in `pii/tests.rs`.

- `validate`: LNCh, EIK 9/13 and EGN date-decode validators, with valid and invalid cases.
- `t1`: one positive and one near-miss per recognizer. A bare 9-digit number gives no detection.
- `t2`: per pack, one sentence per anchor asserts kind and exact span. Address stops at "ЕГН". A name stops at a lowercase word. Org by suffix with quotes.
- `resolve`: an anchored LNCh beats an unanchored EGN on the same digits. An address containing a postcode stays one address.
- `parties`: role before ("ПРОДАВАЧ: ..."), role after ("..., наричан по-долу ПРОДАВАЧ"), two parties in two numbered clauses, no role gives `party_1` and `party_2`.
- `redact` / `resolve_placeholders`: a round trip on `tests/data/filled.docx`. The inspect output contains no surface of any entity, and a save with placeholder finds produces the same template as a save with raw finds.
- MCP: `inspect_docx` and `inspect_pdf` (flat fixture) outputs contain no value text, and `pii` is present. `guard` masks a valid EGN and IBAN in arbitrary text.
- Marks: `compile` produces the same replacements from `At::Entity`, from `At::Span` with `all`, and from the equivalent `At::Find`. Longest-first ordering holds. A UTF-16 offset round trip on a string with a non-BMP character. Selection snapping trims spaces and punctuation and widens to word boundaries.
- HTML: `data-b` indices match `read_body` indices on a fixture with a table. With no decorator, the output is byte-identical to today.
- Conversion: `convert_save` on `filled.docx` with the suggested marks gives a template whose `render_example` with invented values reproduces the structure.

End to end: `cargo run` with `BLANKEY_DATA_DIR` set to the real data dir. From an MCP client, inspect a filled docx, check that only placeholders appear, then save it with placeholder finds and run `render_example` and `check_bindings`. After that, `cargo clippy` and `cargo fmt --check`.

## 7. Open questions

1. **Spans across runs.** Word often splits a value across runs (spell-check, revision ids). Proposal: let `docx::tokenize` merge adjacent runs when the find is a resolved placeholder. This would be the only change outside the `pii/` module that has logic in it. The tag goes in the first run, the rest of the span is cut, and the first run's formatting is kept. The alternative is to report `crosses_runs` and let the AI pick a wider find. I recommend merging.
2. **Origin storage.** Resolved finds are real values and are stored in the template origin, as they are today. Is the template store covered by vault encryption, or should the origin keep placeholders and re-resolve on rebuild? Re-resolving risks ids shifting when the document changes.
3. **Thresholds.** Should redact (0.40) and suggest (0.70) be user settings, or stay constants?

## 8. Rollout

1. `pii` module with T1 and T2, BG and EN packs, and unit tests.
2. `inspect_docx` redaction plus `save_docx_template` placeholder resolution, and the cross-run tokenize change if approved.
3. `inspect_pdf` flat-mode redaction, `places`, and image masks.
4. Preview conversion: the `to_html` decorator, `convert_preview` and `convert_save`, the shared save bodies, and conversion mode in `preview.js` (docx first, then PDF rects).
5. Library `filled` readiness, and the optional user anchor pack.
6. Separate RFCs: OCR for scans, then the T3 NER model as another span source in `analyze`.
