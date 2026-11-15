import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, action, byText, click, dblclick, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp();
});

const rows = () => $$(".grid tbody tr").map((tr) => tr.textContent.replace(/\s+/g, " ").trim());

test("the app opens on Cases; activity lists the assistant's requests", async () => {
  assert.equal(text(".toolbar h2"), "Cases");
  await goTo("Activity");
  assert.equal(text(".toolbar h2"), "Activity");
  assert.equal(text(".toolbar .muted"), "Nothing waiting");
  assert.equal(rows().length, 1);
});

test("documents: one opens inside Blankey, to read, print or hand to its app; several open in their apps", async () => {
  await goTo("Documents");
  const buttons = () => $$(".sheet-head .head-actions button").map((b) => b.textContent);
  const [loan, nzok] = $$(".grid tbody tr");
  await dblclick(nzok);
  assert.deepEqual(backend.called("document_preview").at(-1), { id: 1 });
  assert.equal(text(".sheet h2"), "NZOK (acme)");
  assert.equal(text(".sheet .reason"), "nzok.pdf");
  assert.ok($(".sheet .preview-area .pdf-page"));
  assert.deepEqual(buttons(), ["Print", "Open in default app", "Close"]);
  await click(byText(".sheet-head button", "Print"));
  assert.equal(backend.called("print_window").length, 1);
  await click(byText(".sheet-head button", "Open in default app"));
  assert.deepEqual(backend.called("open_documents").at(-1), { ids: [1] });
  await click(byText(".sheet-head button", "Close"));

  await dblclick(loan);
  assert.ok($(".sheet .preview-area .paper"));
  assert.deepEqual(buttons(), ["Open in default app", "Close"], "a Word document prints from its own app");
  await click(byText(".sheet-head button", "Close"));

  await click(loan);
  await click(nzok, { metaKey: true });
  await action("Open");
  assert.equal($(".sheet"), null);
  assert.deepEqual(backend.called("open_documents").at(-1), { ids: [2, 1] });
  await click(nzok);
  await click(nzok, { metaKey: true });
});

test("documents: cmd-click selects several, delete asks first", async () => {
  await goTo("Documents");
  assert.deepEqual(
    rows().map((r) => r.split(" ")[0]),
    ["Loan", "NZOK"]
  );
  const [first, second] = $$(".grid tbody tr");
  await click(first);
  await click(second, { metaKey: true });
  assert.equal($$(".grid tr.selected").length, 2);

  await action("Delete");
  assert.equal(text(".modal h3"), "Delete 2 documents?");
  await click(byText(".modal button", "Cancel"));
  assert.equal($(".modal"), null);
  assert.equal(backend.called("delete_documents").length, 0);

  await action("Delete");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("delete_documents")[0].ids, [2, 1]);
  assert.equal(text(".empty"), "No documents yet. Documents are generated from fill sets.");
});

test("search filters the current page and clears on navigation", async () => {
  await goTo("Fill sets");
  assert.equal(rows().length, 1);
  await type($("#search"), "nothing like this");
  assert.equal(text(".empty"), "No fill sets yet. Ask your AI assistant to fill in a template.");
  await type($("#search"), "acm");
  assert.equal(rows().length, 1);
  await goTo("Templates");
  assert.equal($("#search").value, "");
});

test("templates: the previous preview stays until the next one is ready", async () => {
  await settle();
  assert.equal(text(".detail h3"), "NZOK request");
  assert.equal($$(".detail .pdf-page .box").length, 2);

  backend.db.previewDelay = 400;
  await click(byText(".card .title", "Loan contract").parentElement);
  assert.ok($(".card.active").textContent.includes("Loan contract"));
  assert.equal(text(".detail h3"), "NZOK request", "old detail kept while loading");
  assert.equal(text(".preview-error"), null);

  await settle(450);
  assert.equal(text(".detail h3"), "Loan contract");
  assert.ok($(".detail .paper [data-var=who]"));
  assert.equal($$("#page-actions button").filter((b) => b.textContent === "Show in Finder").length, 0);
  await click(byText(".detail button", "Show in Finder"));
  assert.deepEqual(backend.called("show_template_folder").at(-1), { id: "loan" });
});

test("templates: folders list their files with a preview and are added and removed", async () => {
  await goTo("Templates");
  await click(byText(".filters .chip", "Folders"));
  assert.equal(text(".toolbar .muted"), "1 folder");
  assert.deepEqual(
    $$(".library-file .title").map((el) => el.textContent),
    ["Lease agreement.docx", "Application for a permit"]
  );
  await settle();
  assert.equal(text(".detail h3"), "Lease agreement.docx");
  assert.equal(backend.called("library_preview").at(-1).id, 4);
  assert.equal($$("#page-actions button").filter((b) => b.textContent === "Show in Finder").length, 0);

  await click(byText(".library-file .title", "Application for a permit").parentElement);
  await settle();
  assert.equal(text(".detail h3"), "Application for a permit");
  await click(byText(".detail button", "Show in Finder"));
  assert.deepEqual(backend.called("show_library_file").at(-1), { id: 5 });
  await click(byText(".detail button", "Open"));
  assert.deepEqual(backend.called("open_library_file").at(-1), { id: 5 });

  // a file deleted on disk: the watcher's event reloads the list
  const deleted = backend.db.libraryFiles.shift();
  backend.emit("library-changed");
  await settle();
  assert.deepEqual($$(".library-file .title").map((el) => el.textContent), ["Application for a permit"]);
  backend.db.libraryFiles.unshift(deleted);
  backend.emit("library-changed");
  await settle();
  assert.equal($$(".library-file").length, 2);

  await action("Add folder…");
  assert.equal(backend.called("add_library_root").length, 1);
  assert.equal($$(".root").length, 2);

  await click(byText(".root button", "Remove"));
  await submit($(".modal form"));
  assert.deepEqual(backend.called("remove_library_root")[0], { id: 1 });
  assert.equal($$(".root").length, 1);
  assert.equal(text(".list-note"), "Every file in these folders is a template already.");

  await click(byText(".filters .chip", "Templates"));
  assert.ok($(".card .title"));
});

test("profiles: edits mark the profile unsaved, problems are shown, switching away asks", async () => {
  backend.db.previewDelay = 0;
  await goTo("Profiles");
  assert.equal(text(".list-item.active .title"), "John");
  const keys = () => $$("input.key").map((input) => input.value);
  assert.deepEqual(keys(), ["name", "egn"]);
  assert.deepEqual(
    $$("select.type").map((select) => select.value),
    ["text", "egn"]
  );
  assert.ok($("input.value").classList.contains("masked"));
  await click($$(".fields .eye")[1]);
  assert.deepEqual($$("input.value").map((input) => input.classList.contains("masked")), [true, false], "the eye shows one value");
  assert.equal($$(".fields .eye")[1].title, "Hide value");
  await click($$(".fields .eye")[1]);
  assert.ok($$("input.value")[1].classList.contains("masked"));

  await type($$("input.value")[1], "8001010000");
  assert.equal(text(".detail-actions > span.small"), "Unsaved changes");

  await click(byText(".list-item .title", "Acme").parentElement);
  assert.equal(text(".modal h3"), "Discard unsaved changes?");
  await click(byText(".modal button", "Cancel"));
  assert.equal(text(".list-item.active .title"), "John");

  await type($$("input.key")[1], "bad key");
  await click(byText(".detail-actions button", "Save"));
  assert.equal(text(".fields .field-problem"), "use dotted words without spaces");
  assert.equal(text(".problems"), "");

  await type($$("input.key")[1], "egn");
  await click(byText(".detail-actions button", "Save"));
  const saved = backend.called("save_fields").at(-1);
  assert.deepEqual(saved.fields[1], { key: "egn", label: "EGN", type: "egn", value: "8001010000" });
  assert.deepEqual(saved.removed, []);
  assert.equal(text(".detail-actions > span.small"), "");

  await click(byText("button", "Add field"));
  assert.equal(keys().length, 3);
  assert.equal(document.activeElement, $$("input.key")[2]);
});

test("profiles: the fields are edited as CSV and copied into another profile", async () => {
  await goTo("Profiles");
  const csv = () => $("textarea.fields-csv");
  const toggle = (label) => click(byText(".fields-head button", label));
  const save = () => click(byText(".detail-actions button", "Save"));
  await toggle("Edit as CSV");
  const copied = csv().value;
  assert.equal(copied, "key,label,type,value\nname,Full name,text,John Watson\negn,EGN,egn,8001010000");
  assert.equal($("table.fields"), null);

  await click(byText(".list-item .title", "Acme").parentElement);
  assert.equal(csv().value, "key,label,type,value", "the next profile opens as CSV too");
  await type(csv(), `${copied}\n\naddress,"Baker St, 221B",,"say ""hi"""\n`);
  assert.equal(text(".detail-actions > span.small"), "Unsaved changes");
  await save();
  assert.deepEqual(backend.called("save_fields").at(-1), {
    id: 2,
    fields: [
      { key: "name", label: "Full name", type: "text", value: "John Watson" },
      { key: "egn", label: "EGN", type: "egn", value: "8001010000" },
      { key: "address", label: "Baker St, 221B", type: "text", value: 'say "hi"' },
    ],
    removed: [],
  });
  assert.equal(csv().value, `${copied}\naddress,"Baker St, 221B",text,"say ""hi"""`);

  await type(csv(), "name,Full name,text,John Watson\ncity,City,text,London");
  await toggle("Edit as table");
  assert.deepEqual($$("input.key").map((input) => input.value), ["name", "city"]);
  await save();
  assert.deepEqual(backend.called("save_fields").at(-1).removed, ["egn", "address"], "saved fields the CSV left out are removed");

  await toggle("Edit as CSV");
  await type(csv(), 'name,Full name,text,John\ncity,City,place,London');
  await toggle("Edit as table");
  assert.equal(text(".problems"), 'Line 2: unknown type "place"');
  const saves = backend.called("save_fields").length;
  await type(csv(), "key,label,type,value\nname,Full name,text,John\ncity,City,text,London\nname,Name,text,Jon\n,No key,text,x\ncity,Town,place,Leeds\nname,Again,text,J");
  await save();
  assert.deepEqual($(".problems").textContent.split("\n"), [
    'Line 4: the key "name" is already on line 2',
    "Line 5: the key is missing",
    'Line 6: the key "city" is already on line 3',
    'Line 6: unknown type "place"',
    'Line 7: the key "name" is already on line 2',
  ], "every bad line is listed");
  await type(csv(), 'name,Full name,text,"John');
  await save();
  assert.equal(text(".problems"), "Line 1: expected key, label, type, value");
  assert.equal(backend.called("save_fields").length, saves, "CSV that does not parse is not saved");
  await type(csv(), "name,Full name,text,John");
  await toggle("Edit as table");
  assert.deepEqual($$("input.value").map((input) => input.value), ["John"]);
  assert.equal(text(".problems"), "");
});

test("the theme switch flips light and dark and remembers the choice", async () => {
  const theme = () => document.documentElement.dataset.theme;
  const first = theme();
  assert.ok(["light", "dark"].includes(first));
  await click($(".foot-actions .icon-btn"));
  assert.notEqual(theme(), first);
  assert.equal(localStorage.getItem("blankey.theme"), theme());
  await click($(".foot-actions .icon-btn"));
  assert.equal(theme(), first);
});

test("lock returns to the unlock screen", async () => {
  await click($("#lock-btn"));
  assert.equal(text(".gate h1"), "Blankey is locked");
});
