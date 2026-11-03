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
});

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
  backend.emit("tauri://drag-enter", { paths: [], position: { x: 10, y: 10 } });
  await settle();
  assert.equal(text(".drop-overlay"), "Drop the files into “Sale of the flat, 12 Oak Street”");
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/paul_id-card.jpg", "/Users/test/power-of-attorney.docx"], position: { x: 10, y: 10 } });
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
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/deed.pdf"] });
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

test("files: double-click opens one; its ⋮ menu has what a file can do", async () => {
  await dblclick(row("Title deed"));
  assert.deepEqual(backend.called("open_collected_files").at(-1), { ids: [12] });

  assert.deepEqual(await menuOf("Title deed"), ["Open", "Export…", "Description…", "Personal data…", "Delete"]);
  for (const item of $$(".menu-item")) {
    await hover(item);
    assert.equal(text(".help-title"), item.textContent, "each menu item has its help");
  }
  assert.equal(text(".help-text"), "Removes the files from the vault. Copies you exported stay.");
  await key("Escape", document.body);
  assert.equal($(".menu"), null);
  assert.deepEqual(await menuOf("mary-id.jpg"), ["Open", "Export…", "Description…", "Delete"], "no personal data check for an image");
  await key("Escape", document.body);

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
  await click(box("mary-id.jpg"));
  await click(box("power-of-attorney.docx"));
  assert.equal(text(".tab-actions .checked-count"), "2 selected");
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Delete", "Clear"]);

  await click(byText(".tab-actions button", "Clear"));
  assert.deepEqual($$(".tab-actions button").map((b) => b.textContent), ["Import…"]);

  // rows: power of attorney, Paul's ID, Mary's ID, title deed
  await click(box("power-of-attorney.docx"));
  await click(box("mary-id.jpg"), { shiftKey: true });
  assert.deepEqual(ticks(), [true, true, true, false], "shift-click on a checkbox checks the rows from the last one");
  await click(box("paul_id-card.jpg"), { shiftKey: true });
  assert.deepEqual(ticks(), [true, false, false, false], "and unchecks them when it unchecks");
  await tabAction("Clear");

  await click($(".file-label", row("Title deed")), { metaKey: true });
  assert.equal(text(".tab-actions .checked-count"), "1 selected", "cmd-click on a row checks it");
  await click($(".file-label", row("mary-id.jpg")), { shiftKey: true });
  assert.deepEqual(ticks(), [false, false, true, true], "shift-click on a row checks the range");
  await click($(".file-label", row("Title deed")), { metaKey: true });
  assert.equal(box("Title deed").checked, false, "cmd-click again unchecks it");
  await click($(".file-label", row("power-of-attorney.docx")));
  assert.equal(box("power-of-attorney.docx").checked, false, "a plain click checks nothing");
  await tabAction("Clear");
  await click($(".file-label", row("Title deed")), { shiftKey: true });
  assert.deepEqual(ticks(), [false, false, false, true], "shift-click with nothing clicked before checks that row");
  await tabAction("Clear");

  await check($("thead th.check input"));
  assert.equal(text(".tab-actions .checked-count"), "4 selected");
  await click(box("Title deed"));
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
  assert.equal(text(".case-tab .empty"), "No files yet. Drop the documents of this case here, or use Import…");
});
