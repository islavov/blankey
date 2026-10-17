import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, action, byText, click, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp();
});

const rows = () => $$(".grid tbody tr").map((tr) => tr.textContent.replace(/\s+/g, " ").trim());

test("activity lists the assistant's requests", async () => {
  assert.equal(text(".toolbar h2"), "Activity");
  assert.equal(text(".toolbar .muted"), "Nothing waiting");
  assert.equal(rows().length, 1);
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
  assert.equal(backend.called("delete_collected_files").length, 0);
  assert.equal(text(".empty"), "No documents yet. Documents are generated from fill sets or collected through links.");
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

  await type($$("input.value")[1], "8001010000");
  assert.equal(text(".detail-actions > span.small"), "Unsaved changes");

  await click(byText(".list-item .title", "Acme").parentElement);
  assert.equal(text(".modal h3"), "Discard unsaved changes?");
  await click(byText(".modal button", "Cancel"));
  assert.equal(text(".list-item.active .title"), "John");

  await type($$("input.key")[1], "bad key");
  await click(byText(".detail-actions button", "Save"));
  assert.equal(text(".problems"), "bad key: keys cannot contain spaces");

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
