import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, action, byText, check, click, dblclick, goTo, hover, key, settle, startApp, submit, tabAction, text, type } from "./setup.js";

const WORD = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const file = (id, caseId, name, type, description) => ({
  id, case_id: caseId, profile_id: null, link_id: "", name, type, size: 2048, description, created_at: "2026-10-28T09:00:00", link_title: "", profile_name: null,
});

let backend;
before(async () => {
  backend = await startApp({
    profiles: [
      { id: 1, name: "John", kind: "person" },
      { id: 2, name: "Acme", kind: "company" },
      { id: 3, name: "Mary Example", kind: "person", global: false },
      { id: 4, name: "Paul Sample", kind: "person", global: false },
    ],
    fields: { 1: [], 2: [], 3: [], 4: [] },
    cases: [
      { id: 1, title: "Sale, 12 Oak Street", notes: "Deed on 14 November", created_at: "2026-10-28T09:00:00", updated_at: "2026-11-02T10:00:00" },
      { id: 2, title: "Lease, Main Road", notes: "", created_at: "2026-10-20T09:00:00", updated_at: "2026-10-21T10:00:00" },
    ],
    caseLinks: [
      { case_id: 1, profile_id: 3, role: "seller" },
      { case_id: 1, profile_id: 1, role: "buyer" },
      { case_id: 2, profile_id: 4, role: "tenant" },
    ],
    collected: [
      file(11, 1, "mary-id.jpg", "image/jpeg", "Mary Example, seller, ID card"),
      file(12, 1, "title-deed.pdf", "application/pdf", "Title deed, 2019"),
      file(13, 2, "lease.docx", WORD, "Model lease"),
    ],
    templates: [
      { id: "nzok", name: "NZOK request", kind: "pdf_form", blanks: 2, used_by: [], description: "" },
      { id: "oak-sale", name: "Sale contract, Oak Street", kind: "docx", blanks: 9, used_by: [], description: "", case_id: 1 },
    ],
    docs: [
      { id: 1, title: "NZOK (acme)", template_id: "nzok", filename: "nzok.pdf", pages: 2, created_at: "2026-10-10T10:00:00" },
      { id: 3, title: "Sale contract (Oak)", template_id: "oak-sale", filename: "oak-sale.docx", pages: 0, created_at: "2026-11-02T10:00:00", case_id: 1 },
    ],
  });
  document.elementFromPoint = (x) => spots[x]?.() ?? null;
});

// happy-dom lays nothing out: a drag position stands for the element the test puts under the pointer
const spots = [];
const at = (find) => ({ x: spots.push(find) - 1, y: 0 });

// a row's cells, and the lines inside a cell, joined with spaces
const cell = (td) => (td.children.length ? [...td.children] : [td]).map((el) => el.textContent.trim()).join(" ");
const rows = () => $$(".case-tab .grid tbody tr").map((tr) => [...tr.children].map(cell).filter(Boolean).join(" "));
const row = (part) => $$(".case-tab .grid tbody tr").find((tr) => tr.textContent.includes(part));
/** The items of a row's ⋮ menu, left open. */
const menuOf = async (part) => {
  await click($(".more", row(part)));
  return $$(".menu-item").map((el) => el.textContent);
};
const rowMenu = async (part, item) => {
  await click($(".more", row(part)));
  await click(byText(".menu-item", item));
};
const tab = (label) => click($$(".case-tabs .chip").find((el) => el.textContent.trim().startsWith(label)));
const caseItem = (title) => byText(".split.cases .list-item .title", title).parentElement;
const onCase = at(() => $(".case-tab"));
const onList = at(() => $(".split.cases .list"));
const onItem = (title) => at(() => $(".title", caseItem(title)));

test("the app opens on Cases with the most recently changed case and its files", () => {
  assert.equal(text(".toolbar h2"), "Cases");
  assert.deepEqual($$(".split.cases .list-item .title").map((el) => el.textContent), ["Sale, 12 Oak Street", "Lease, Main Road"]);
  assert.deepEqual($$(".split.cases .list-item")[0].querySelectorAll(".muted").length, 2);
  assert.equal(text(".split.cases .list-item .muted"), "Mary Example, John");
  assert.equal(text(".split.cases .list-item .muted:last-child"), "2 files · 2026-11-02 10:00");
  assert.equal($(".case-title").value, "Sale, 12 Oak Street");
  assert.equal($(".case-notes").value, "Deed on 14 November");
  assert.deepEqual(rows(), ["Mary Example, seller, ID card mary-id.jpg Image 2 KB 2026-10-28 09:00", "Title deed, 2019 title-deed.pdf PDF 2 KB 2026-10-28 09:00"]);
});

test("Copy link puts the case's link on the clipboard, for the AI assistant", async () => {
  await click(byText(".case-head button", "Copy link"));
  assert.deepEqual(backend.called("copy_text").at(-1), { text: "blankey://case/1" });
  assert.equal(text("#toast"), "Copied blankey://case/1");
});

test("title and notes are saved when they lose focus; an empty title is put back", async () => {
  const title = $(".case-title");
  title.value = "Sale of the flat, 12 Oak Street";
  title.dispatchEvent(new FocusEvent("blur"));
  await settle();
  assert.deepEqual(backend.called("update_case").at(-1), { id: 1, title: "Sale of the flat, 12 Oak Street" });

  const notes = $(".case-notes");
  notes.value = "Deed on 15 November";
  notes.dispatchEvent(new FocusEvent("blur"));
  await settle();
  assert.deepEqual(backend.called("update_case").at(-1), { id: 1, notes: "Deed on 15 November" });

  const calls = backend.called("update_case").length;
  $(".case-title").value = "  ";
  $(".case-title").dispatchEvent(new FocusEvent("blur"));
  await settle();
  assert.equal(backend.called("update_case").length, calls);
  assert.equal($(".case-title").value, "Sale of the flat, 12 Oak Street");
});

test("a drag shows where the files go; the drop opens Import with descriptions from the file names", async () => {
  backend.emit("tauri://drag-enter", { paths: [], position: onList });
  await settle();
  assert.equal(text(".case-detail > .drop-overlay"), "Drop the files into “Sale of the flat, 12 Oak Street”");
  assert.equal($(".drop-overlay.over"), null, "not lit outside the case");
  backend.emit("tauri://drag-over", { position: onCase });
  await settle();
  assert.ok($(".case-detail > .drop-overlay.over"), "lit over the case");
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/paul_id-card.jpg", "/Users/test/power-of-attorney.docx"], position: onCase });
  await settle();
  assert.equal($(".drop-overlay"), null);
  assert.equal(text(".sheet h2"), "Import into “Sale of the flat, 12 Oak Street”");
  assert.deepEqual($$(".import-row input").map((input) => input.value), ["paul id card", "power of attorney"]);

  backend.emit("tauri://drag-drop", { paths: ["/Users/test/power-of-attorney.docx", "/Users/test/deed.pdf"] });
  await settle();
  assert.deepEqual($$(".import-row .import-name").map((el) => el.textContent), ["paul_id-card.jpg", "power-of-attorney.docx", "deed.pdf"], "a drop adds to the open import, once per file");

  await click($$(".import-row .remove")[2]);
  await type($$(".import-row input")[0], "Paul Sample, buyer, ID card");
  await click(byText(".sheet-foot button", "Import 2 files"));
  assert.deepEqual(backend.called("import_case_files").at(-1), {
    caseId: 1,
    items: [
      { path: "/Users/test/paul_id-card.jpg", description: "Paul Sample, buyer, ID card" },
      { path: "/Users/test/power-of-attorney.docx", description: "power of attorney" },
    ],
  });
  assert.equal($(".sheet"), null);
  assert.equal(text("#toast"), "2 files imported");
  assert.equal(rows().length, 4);
});

test("files land only on the open case; held over another case, they open it", async () => {
  backend.emit("tauri://drag-enter", { paths: [], position: onList });
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/deed.pdf"], position: onList });
  await settle();
  assert.equal($(".sheet"), null, "a drop beside the case is ignored");
  assert.equal($(".drop-overlay"), null);

  backend.emit("tauri://drag-enter", { paths: [], position: onItem("Sale of the flat, 12 Oak Street") });
  await settle();
  assert.equal($(".case-item.opening"), null, "the open case does not open again");
  backend.emit("tauri://drag-over", { position: onItem("Lease, Main Road") });
  await settle();
  assert.ok(caseItem("Lease, Main Road").classList.contains("opening"), "the case shows that it is about to open");
  assert.equal($(".case-title").value, "Sale of the flat, 12 Oak Street");
  await settle(2000);
  assert.equal($(".case-title").value, "Lease, Main Road");
  assert.ok(caseItem("Lease, Main Road").classList.contains("active"));
  assert.equal($(".case-item.opening"), null);
  assert.equal(text(".case-detail > .drop-overlay"), "Drop the files into “Lease, Main Road”");

  backend.emit("tauri://drag-over", { position: onCase });
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/deed.pdf"], position: onCase });
  await settle();
  assert.equal(text(".sheet h2"), "Import into “Lease, Main Road”");
  await click(byText(".sheet-foot button", "Cancel"));
  await click(caseItem("Sale of the flat, 12 Oak Street"));
});

test("Import… picks files; a drop while a dialog is open is ignored", async () => {
  backend.db.pickPaths = ["/Users/test/car-registration.pdf"];
  await action("Import…");
  assert.deepEqual($$(".import-row input").map((input) => input.value), ["car registration"]);
  await click(byText(".sheet-foot button", "Cancel"));
  assert.equal($(".sheet"), null);

  await action("New case");
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/x.pdf"] });
  await settle();
  assert.equal($(".sheet"), null);
  await click(byText(".modal button", "Cancel"));
});

test("Escape closes what is on top: the dialog first, then the sheet", async () => {
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/deed.pdf"], position: onCase });
  await settle();
  assert.ok($(".sheet"));
  await key("Escape", $(".import-row input"));
  assert.equal($(".sheet"), null);

  await action("New case");
  await key("Escape", document.body);
  assert.equal($(".modal"), null);
  assert.equal(backend.called("create_case").length, 0);

  await tab("Profiles");
  await dblclick($$(".case-tab .grid tbody tr")[0]);
  await type($(".sheet .fields input.label"), "Changed");
  await key("Escape", document.body);
  assert.equal(text(".modal h3"), "Discard unsaved changes?");
  await key("Escape", document.body);
  assert.equal($(".modal"), null);
  assert.ok($(".sheet"), "the sheet stays when the question is cancelled");
  await key("Escape", document.body);
  await submit($(".modal form"));
  assert.equal($(".sheet"), null);
  await tab("Files");
});

test("files: double-click shows one inside Blankey; its ⋮ menu has what a file can do", async () => {
  await dblclick(row("Title deed"));
  assert.equal(text(".sheet h2"), "Title deed, 2019");
  assert.deepEqual(backend.called("case_file_preview").at(-1), { id: 12 });
  assert.ok($(".sheet .preview-area .pdf-page, .sheet .preview-area .paper"), "the file is shown, not handed to another app");
  assert.equal(backend.called("open_collected_files").length, 0);
  assert.deepEqual($$(".sheet-head .head-actions button").map((b) => b.textContent), ["Print", "Open in default app", "Close"]);
  assert.equal($(".sheet-foot"), null, "the buttons are in the heading");
  await click(byText(".sheet-head button", "Print"));
  assert.equal(backend.called("print_window").length, 1);
  await click(byText(".sheet-head button", "Open in default app"));
  assert.deepEqual(backend.called("open_collected_files").at(-1), { ids: [12] });
  await click(byText(".sheet-head button", "Close"));
  assert.equal($(".sheet"), null);

  assert.deepEqual(await menuOf("Title deed"), ["Open", "Export…", "Rename…", "Description…", "Personal data…", "Read as a document", "Delete"]);
  for (const item of $$(".menu-item")) {
    await hover(item);
    assert.equal(text(".help-title"), item.textContent, "each menu item has its help");
  }
  assert.equal(text(".help-text"), "Removes the files from the vault. Copies you exported stay.");
  await key("Escape", document.body);
  assert.equal($(".menu"), null);
  assert.deepEqual(await menuOf("mary-id.jpg"), ["Open", "Export…", "Rename…", "Description…", "Read as a document", "Delete"], "no personal data check for an image");
  await key("Escape", document.body);
  await rowMenu("mary-id.jpg", "Open");
  assert.ok($(".sheet img.preview-image"), "an image shows as it is");
  await click(byText(".sheet-head button", "Close"));

  await rowMenu("mary-id.jpg", "Rename…");
  assert.equal($(".modal input[name=name]").value, "mary-id.jpg");
  await type($(".modal input[name=name]"), "scan-of-mary-id.jpg");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("update_case_file").at(-1), { id: 11, name: "scan-of-mary-id.jpg" });
  assert.ok(row("scan-of-mary-id.jpg"));

  await rowMenu("Title deed", "Description…");
  await type($(".modal input[name=description]"), "Title deed of the flat, 2019");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("update_case_file").at(-1), { id: 12, description: "Title deed of the flat, 2019" });

  await rowMenu("Title deed", "Personal data…");
  assert.equal(text(".sheet h2"), "Personal data in Title deed of the flat, 2019");
  assert.equal(backend.called("pii_review").at(-1).source, "casefile:12");
  await click(byText(".sheet-foot button", "Done"));
});

test("files: checking rows turns the buttons on top into what to do with them", async () => {
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Import…"]);
  const box = (part) => $("td.check input", row(part));
  const ticks = () => $$("td.check input").map((input) => input.checked);
  /** Presses the first row's checkbox, moves over the others with the button held, lets go. */
  const drag = async (first, ...over) => {
    $("td.check", row(first)).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    for (const part of over) row(part).dispatchEvent(new MouseEvent("mouseenter"));
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    await settle();
  };
  await drag("mary-id.jpg");
  await drag("power-of-attorney.docx");
  assert.equal(text(".tab-actions .checked-count"), "2 selected");
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Delete", "Clear"]);

  await click(byText(".tab-actions button", "Clear"));
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Import…"]);

  // rows: power of attorney, Paul's ID, Mary's ID, title deed
  await drag("power-of-attorney.docx", "paul_id-card.jpg", "mary-id.jpg");
  assert.deepEqual(ticks(), [true, true, true, false], "a drag from an unchecked box checks every row passed over");
  await drag("paul_id-card.jpg", "mary-id.jpg", "Title deed");
  assert.deepEqual(ticks(), [true, false, false, false], "a drag from a checked box unchecks them");
  row("power-of-attorney.docx").dispatchEvent(new MouseEvent("mouseenter"));
  await settle();
  assert.deepEqual(ticks(), [true, false, false, false], "after letting go, moving over rows changes nothing");
  await tabAction("Clear");

  await click($(".file-label", row("Title deed")), { metaKey: true });
  assert.equal(text(".tab-actions .checked-count"), "1 selected", "cmd-click on a row checks it");
  await click($(".file-label", row("Title deed")), { metaKey: true });
  assert.equal(box("Title deed").checked, false, "cmd-click again unchecks it");

  $(".file-label", row("power-of-attorney.docx")).dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, metaKey: true }));
  row("paul_id-card.jpg").dispatchEvent(new MouseEvent("mouseenter"));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  await settle();
  assert.deepEqual(ticks(), [true, true, false, false], "cmd held down on a row drags like the checkbox");
  await tabAction("Clear");
  await click($(".file-label", row("power-of-attorney.docx")), { shiftKey: true });
  assert.deepEqual(ticks(), [false, false, false, false], "a plain or shift click checks nothing");

  await check($("thead th.check input"));
  assert.equal(text(".tab-actions .checked-count"), "4 selected");
  await drag("Title deed");
  await tabAction("Delete");
  assert.equal(text(".modal h3"), "Delete 3 files?");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("delete_collected_files").at(-1).ids.sort((a, b) => a - b), [11, 14, 15], "all but the unchecked title deed");
  assert.deepEqual(rows().map((r) => r.split(" ")[0]), ["Title"]);
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Import…"]);
});

test("profiles: new ones belong to the case, existing ones are added with a role", async () => {
  await tab("Profiles");
  assert.deepEqual(rows(), ["Mary Example person seller Only in this case", "John person buyer Global"]);

  await tabAction("New profile");
  await type($(".modal input[name=name]"), "Paul's brother");
  await type($(".modal input[name=role]"), "attorney");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("create_profile").at(-1), { name: "Paul's brother", kind: "person", caseId: 1, role: "attorney" });
  assert.equal(rows().length, 3);

  await tabAction("Add existing…");
  const options = $$(".modal select[name=profile] option").map((o) => o.textContent);
  assert.deepEqual(options, ["Acme", "Paul Sample (Lease, Main Road)"], "global profiles, and local ones named by their cases");
  $(".modal select[name=profile]").value = "4";
  await type($(".modal input[name=role]"), "buyer");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("link_case_profile").at(-1), { caseId: 1, profileId: 4, role: "buyer" });
  assert.ok(rows().includes("Paul Sample person buyer In 2 cases"));
});

test("profiles: make one global; removing one that exists only here deletes it after asking", async () => {
  assert.deepEqual(await menuOf("John"), ["Edit", "Role…", "Remove from case"], "John is global already");
  await key("Escape", document.body);
  await rowMenu("Mary Example", "Make global");
  assert.equal(text(".modal h3"), "Make \"Mary Example\" global?");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("make_profile_global").at(-1), { id: 3 });
  assert.ok(rows().includes("Mary Example person seller Global"));

  await rowMenu("Paul's brother", "Remove from case");
  assert.equal(text(".modal h3"), "Remove \"Paul's brother\" from the case?");
  await submit($(".modal form"));
  assert.equal(backend.called("remove_case_profile").at(-1).profileId, 5);
  assert.ok(!rows().some((r) => r.includes("Paul's brother")));
  assert.ok(!backend.db.profiles.some((p) => p.id === 5));

  await dblclick(row("John"));
  assert.equal(text(".sheet h2"), "John");
  await click(byText(".sheet-foot button", "Close"));
});

test("templates: the case's own first; make global; fill goes into the case", async () => {
  await tab("Templates");
  assert.deepEqual(rows(), ["Sale contract, Oak Street Word 9 Only in this case", "NZOK request PDF 2 Global"]);

  assert.deepEqual(await menuOf("NZOK request"), ["Fill"], "a global template is only filled here");
  await key("Escape", document.body);

  await check($("thead th.check input"));
  await tabAction("Fill");
  assert.equal(backend.called("fill_open").at(-1).args.case_id, 1);
  assert.deepEqual(backend.called("fill_open").at(-1).args.templates, ["oak-sale", "nzok"], "checked templates are filled together");
  await click(byText(".sheet-foot button", "Cancel"));
  await tabAction("Clear");

  await rowMenu("Sale contract", "Make global");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("make_template_global").at(-1), { id: "oak-sale" });
  assert.deepEqual(rows(), ["NZOK request PDF 2 Global", "Sale contract, Oak Street Word 9 Global"]);
});

test("the global pages: Profiles and Templates show global items, Documents names the case", async () => {
  await goTo("Profiles");
  const names = $$(".split .list-item .title").map((el) => el.textContent);
  assert.ok(names.includes("Mary Example"), "made global");
  assert.ok(!names.includes("Paul Sample"), "local to its cases");

  await goTo("Documents");
  const row = $$(".grid tbody tr").find((tr) => tr.textContent.includes("Sale contract (Oak)"));
  await click($(".case-link", row));
  assert.equal(text(".toolbar h2"), "Cases");
  assert.equal($(".case-title").value, "Sale of the flat, 12 Oak Street");
});

test("delete a case: it says what goes with it", async () => {
  await click(caseItem("Lease, Main Road"));
  await action("Delete case");
  assert.equal(text(".modal h3"), "Delete the case \"Lease, Main Road\"?");
  const said = text(".modal .muted");
  assert.ok(said.includes("1 file, 0 fill sets, 0 documents, 0 templates"), said);
  assert.ok(!said.includes("Paul Sample"), "Paul Sample is in another case too");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("delete_case").at(-1), { id: 2 });
  assert.deepEqual($$(".split.cases .list-item .title").map((el) => el.textContent), ["Sale of the flat, 12 Oak Street"]);
});

test("a new case opens ready for files", async () => {
  await action("New case");
  await type($(".modal input[name=title]"), "Power of attorney, car");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("create_case").at(-1), { title: "Power of attorney, car" });
  assert.equal($(".case-title").value, "Power of attorney, car");
  assert.equal(text(".case-tab .empty"), "No files yet. Drop what the client brought here, such as IDs, deeds and contracts, or use Import…");
  assert.ok($(".case-tab .drop-zone svg"), "the empty tab is a drop zone");
  backend.emit("tauri://drag-enter", { paths: [], position: onList });
  await settle();
  assert.equal($(".drop-zone.over"), null);
  backend.emit("tauri://drag-over", { position: onCase });
  await settle();
  assert.ok($(".drop-zone.over"), "lit while files are dragged over the case");
  backend.emit("tauri://drag-leave");
  await settle();
  assert.equal($(".drop-zone.over"), null);
  await click($(".drop-zone"));
  assert.ok(backend.called("pick_files").length > 0, "a click opens Import");
});

test("importing an old Word file asks before it converts it; Cancel goes back to the import", async () => {
  // the Import sheet of the new case is open: the drop adds to it
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/lease-2009.doc"] });
  await settle();
  const imports = backend.called("import_case_files").length;
  await click($(".sheet-foot button.primary"));
  assert.equal(text(".modal h3"), "Old Word files (.doc) need converting");
  assert.ok(text(".modal").includes("lease-2009.doc: Blankey makes a .docx of it"));
  assert.deepEqual($$(".modal button").map((b) => b.textContent), ["Cancel", "Convert and import"]);
  await click(byText(".modal button", "Cancel"));
  assert.equal(backend.called("import_case_files").length, imports, "nothing is imported");
  assert.ok($(".sheet"), "the import stays open");

  await click($(".sheet-foot button.primary"));
  await submit($(".modal form"));
  assert.equal(backend.called("import_case_files").length, imports + 1);
  assert.equal($(".modal"), null, "nothing more to say");
  assert.ok(row("lease-2009.docx").textContent.includes("Word"));
  assert.ok(!row("lease-2009.docx").textContent.includes("Word (.doc)"));
});

test("where it cannot be converted, importing an old Word file says the assistant cannot read it, and how to fix that", async () => {
  backend.db.convertsDoc = false;
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/old-contract.doc"], position: onCase });
  await settle();
  assert.ok($$(".import-row .import-name").some((el) => el.textContent === "old-contract.doc"));
  await click($(".sheet-foot button.primary"));
  await submit($(".modal form"));
  assert.equal(text(".modal h3"), "The assistant cannot read old Word files (.doc)");
  assert.ok(text(".modal").includes("old-contract.doc: open it in Word, save it as .docx and import that file."));
  assert.deepEqual($$(".modal button").map((b) => b.textContent), ["OK"]);
  await submit($(".modal form"));
  assert.ok(row("old-contract.doc").textContent.includes("Word (.doc)"));
  await dblclick(row("old-contract.doc"));
  assert.ok($(".sheet .preview-area"), "it still opens inside Blankey");
  assert.equal(backend.called("case_file_preview").at(-1).id, backend.db.collected.find((f) => f.name === "old-contract.doc").id);
  await click(byText(".sheet-head button", "Close"));
});
