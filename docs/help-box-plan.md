# Hover help box (frontend only)

## Context
Blankey has many buttons and panes whose purpose isn't obvious: fill sources, bindings, collection links, the PII review, Blankey Online. You want a help box in the left sidebar, above the controls (relay status, Lock, theme, cog). It explains in a few words whatever the pointer is over, can be closed from inside the box, and comes back from Settings. One catch-all hover listener drives it. Full-window sheets cover the sidebar, so while a sheet is open the help goes into the sheet's footer note. When nothing is hovered, the footer shows its usual note again.

No backend changes. The on/off choice is kept in `localStorage` on this machine, like the theme.

## Approach

### 1. `ui/js/help.js` (new): the catch-all and the texts
- **Texts**: `HELP = { key: () => [title, text] }`. Every string is a literal `t("...")`, so `scripts/i18n-check.mjs` picks it up and the text is resolved in the current language at render time. Titles reuse existing labels where they can (`t("Generate")`, `t("Lock")`, ...). Each text is one or two short sentences (about 160 characters at most) covering what the thing does and how to use it.
- **Catch-all**: one `document` `mouseover` listener, registered when the module is imported (like `ui/js/ui/menu.js` does with `mousedown`). It walks up from `e.target` and stops at the first element that either:
  - has a `data-help` key that exists in `HELP`, or
  - is a `button` with a `title` or `aria-label`. This is the fallback for buttons nobody annotated: its title becomes the help title.
  It stores that element in `hovered` and notifies only when it changed. A document `mouseout` with no `relatedTarget` (the pointer left the window) clears it. The listener does nothing while help is off.
- **On/off**: key `blankey.help`, read and written with try/catch exactly as `ui/js/theme.js` does. The value is cached in a module variable. `setHelpEnabled(on)` saves it and calls `update({})` from `ui/js/lib.js`, so App and Settings re-render. This happens rarely, so a full re-render is fine.
- **Hover subscription**: a small listener set of its own, not the app store. Calling `update()` on every hover would re-render every page.
- **Exports**:
  - `HelpBox({ page })`: the sidebar box. It shows the hovered help when the element is connected and not inside `.sheet`; otherwise it shows `page.<page>`. It renders the title in bold with a close button, then the text.
  - `HelpNote({ children })`: the sheet footer note. It shows "**title**: text" while the hovered element is inside `.sheet`, otherwise its children (the existing note).
  - `helpEnabled()` and `setHelpEnabled()`.
  - `helpText(key)`, so a test can check that every key exists.
- **Close button**: the ✕ (same glyph as the existing remove buttons) calls `setHelpEnabled(false)` and shows the toast "Help is off. Turn it on again in Settings."

### 2. Where keys go (`data-help="..."`)
Annotate each element of interest, sections as well as their controls. The nearest key wins, so a button inside an annotated pane shows the button's help and empty space in the pane shows the pane's help. This is what makes it "catch-all".
- **`ui/js/app.js`**:
  - render `<${HelpBox} page=${page} />` between `#nav` and `.sidebar-foot` when `helpEnabled()`;
  - `data-help` on each `NavItem` (`nav.<id>`), `#relay-btn`, `#lock-btn`, the theme switch and `#settings-btn`;
  - `<main class="main" data-help=${`page.${page}`}>`, so any spot in the page without its own key shows what the page is for.
- **`ui/js/ui/page.js`**:
  - an optional 4th action element `[label, onClick, cls, help]`, rendered as `data-help=${help}`;
  - `data-help="search"` on `#search`.
- **Pages**: add the help key to each action tuple, plus keys on panes and controls.
  - `ui/js/pages/activity.js`, `fillsets.js` and `documents.js`: actions only; the page text covers the table.
  - `ui/js/pages/templates.js`: actions, the Templates/Folders chips, the card list, the preview, Bindings, Copy ID, Show in Finder, folder roots and Remove, library files, Personal data…, Open.
  - `ui/js/pages/profiles.js`: actions, the profile list, the fields table, Add field, Show values, Save, remove field.
  - `ui/js/pages/links.js`: actions, the filter chips, the side panel; in the New link sheet, Save into, Fields, Files, Expires after, Submissions and Create link.
  - `ui/js/pages/settings.js`: one key per `.relay-row` (Language, Version, Updates, Statistics, Help, MCP server, Online server, Connector URL, Account, Status) and the agents table.
- **Sheets**: replace each `<span class="note">…</span>` with `<${HelpNote}>…</${HelpNote}>` (it renders `span.note`), and add keys to their controls:
  - `ui/js/fill/form.js`: fill set name, role pickers, the rows pane, the preview, Cancel, Save, Save and Generate. In `ui/js/fill/rows.js`, `data-help` on `.fill-field` and `.source`.
  - `ui/js/requests.js`: the profile input form, Choose file…, Share, Allow, Cancel.
  - `ui/js/submission.js`: Save into, the files list with Keep, Discard, Later, Save.
  - `ui/js/review.js`: the hidden-values list, the eye toggle, remove, the prev/next place buttons, Text to hide, the preview; and `openReview`'s note.
  - `ui/js/pages/links.js`: `LinkSheet`'s note.

### 3. Settings toggle (`ui/js/pages/settings.js`)
A new row in General, after Statistics, following the same pattern as the other checkboxes:
```js
<div class="relay-row" data-help="settings.help">
  <span class="muted">${t("Help")}</span>
  <label class="check small">
    <input type="checkbox" id="help-box" checked=${helpEnabled()} onChange=${(e) => setHelpEnabled(e.target.checked)} />
    ${t("Show the help box in the sidebar")}
  </label>
</div>
```
No hint text next to it (per your preference).

### 4. Style (`ui/style.css`, sidebar block)
- `.help-box`: `flex: none`, margin `0 10px 8px`, padding 8px 10px, radius 8px, `background: var(--hover)`, font size 11.5px.
- It has a `min-height` of 136px, which fits every sidebar text in English and Bulgarian, so in practice the box keeps one height. A longer future text grows the box upward into the empty space under the nav instead of being cut off. The box can't be scrolled, because hovering it shows its own help. At the 520px minimum window height, the nav scrolls a little; it already has `overflow: auto`.
- `.help-box .help-head`: a flex row with the bold title and a ghost ✕ button (`.help-close`, muted). The text is muted with `pre-line`.
- Every color comes from the existing tokens, so dark mode works with no extra rules.

### 5. Bulgarian (`ui/i18n/bg.json`)
Add a translation for every new string (about 90 help entries; most titles already exist). `npm test` runs the i18n check first and fails on any missing or unused key.

## Sample texts (tone)
- `nav.fillsets`, **Fill sets**: "Saved fills: the templates, the profiles and where each blank's value comes from. Open one to change it or generate again."
- `fillsets.export`, **Export**: "Saves the fill set as YAML (names, profiles, sources; no values), to import on another computer."
- `fillsets.bundle`, **Export bundle**: "A zip with the fill set and its templates, for a computer that does not have them."
- `fill.source`, **Source**: "Where this blank's value comes from: a profile field, the template default, the assistant's suggestion or a value you type. Click to change."
- `settings.language`, **Language**: "Blankey's language on this computer. Default uses your Blankey Online account's language."
- `help.box`, **Help**: "Explains what is under the pointer. Hide it with ✕ and turn it on again in Settings."

## Tests (`ui/tests/help.test.js`, new; add a `hover(el)` helper to `ui/tests/setup.js` that dispatches a bubbling `mouseover`)
- The box sits between `#nav` and `.sidebar-foot` and shows the current page's help.
- Hovering a nav item, a page action (Fill sets, Generate) or `#lock-btn` shows its help. An unannotated button with a `title` falls back to the title. Leaving the window brings back the page help.
- Every `[data-help]` on the visited pages and in an open fill sheet resolves through `helpText()`, which catches key typos.
- In the fill sheet, hovering Save and Generate puts its help in `.sheet-foot .note`. Hovering the plain sheet head brings back the original note.
- Clicking ✕ removes the box and shows the toast. The Settings checkbox `#help-box` is then off; checking it brings the box back. The choice is in `localStorage` under `blankey.help`.
- Existing assertions are unaffected. `settings.test.js` checks the children of `.sidebar-foot`, and the box sits outside it.

## Verification
1. `npm test`: i18n check plus all UI tests. There is no separate JS linter, so this is the lint step too.
2. Run the real app with `cargo run` and `BLANKEY_DATA_DIR` pointing at your Blankey data directory (per memory). Check:
   - hovering across the sidebar, toolbar, every page, Settings, and inside the fill and PII-review sheets;
   - closing with ✕ and turning it back on in Settings, which survives a restart;
   - dark mode, Bulgarian, and the minimum window height.

## Follow-up (after review)
- **Nav items explain concepts:** what each part of Blankey is for, not how to click it. For example, a collection link is a request you send to someone; only you can decrypt what they send, nobody in between. Profiles are where the sensitive data lives. Section titles have no help. Buttons, panes and settings rows keep short, high-level help.
- **Fill sets lose Export, Export bundle and Import:** YAML and zip are of no use to non-technical people. The `export_fill_set` and `import_fill_set` commands and the core code behind them are removed.
- **Collected files live with their profile:** they move from Documents to a Files section under the profile's fields, with Open, Export and Delete. Deleting a profile deletes its files and their encrypted blobs. Documents lists generated documents only.
- **You can add files to any profile:** the Files section always shows, with an Add files… button. Files you add are encrypted like collected ones, have no link, and are limited to 50 MB each, the same as collection links.
