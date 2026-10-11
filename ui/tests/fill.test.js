import assert from "node:assert/strict";
import { before, test } from "node:test";
import { fillRequest } from "./backend.js";
import { $, $$, byText, click, key, settle, startApp, text, type } from "./setup.js";

const PREVIEW_DELAY = 250;

let backend;
before(async () => {
  // a waiting request opens the fill form right after unlock
  backend = await startApp({ requests: [fillRequest(7)] });
  await settle(PREVIEW_DELAY);
});

const row = (name) => $(`.fill-row[data-var="${name}"]`);
const lastBindings = () => backend.called("fill_evaluate").at(-1).args.bindings;
const openSource = (name) => click($(".source", row(name)));

test("shows the request with one row per blank and the summary", async () => {
  assert.equal(text(".sheet h2"), "Your AI assistant wants to fill in 2 documents");
  assert.deepEqual(
    $$(".fill-row").map((r) => r.dataset.var),
    ["name", "egn", "who", "amount"]
  );
  assert.equal(text(".sheet-summary"), "4 fields · 2 empty");
  assert.equal(text(".chip", row("name")), "Default");
  assert.equal(text(".val", row("name")), "John Watson");
  assert.equal(text(".chip", row("amount")), "Empty");
  assert.equal($(".sheet-form select").value, "1", "the applicant's profile is chosen");
});

test("the preview shows the first document with the active field highlighted", async () => {
  assert.equal(text(".tab.active"), "NZOK request");
  assert.equal($$(".fill-preview .box").length, 2);
  assert.ok(row("name").classList.contains("active"));
  assert.ok($('.box[data-var="name"]').classList.contains("active"));
});

test("picking the suggested value from the source menu binds it", async () => {
  await openSource("amount");
  const items = $$(".menu-item").map((el) => el.textContent);
  assert.ok(items.includes("Suggested by the assistant  —  34 000"));
  assert.ok(items.includes("applicant.name  —  John Watson"));

  await click(byText(".menu-item", "Suggested by the assistant — 34 000"));
  assert.equal($(".menu"), null);
  assert.deepEqual(lastBindings().amount, { value: "34 000" });
  assert.equal(text(".chip", row("amount")), "Suggested");
  assert.equal(text(".sheet-summary"), "4 fields · 1 empty");
});

test("selecting a blank of another document switches the preview tab", async () => {
  await click($(".fill-field", row("who")));
  assert.equal(text(".tab.active"), "Loan contract");
  assert.ok($('.paper [data-var="who"]').classList.contains("active"));
});

test("clicking a field in the preview selects its row", async () => {
  await click(byText(".tab", "NZOK request"));
  await click($('.box[data-var="egn"]'));
  assert.ok(row("egn").classList.contains("active"));
});

test("arrow keys move between blanks", async () => {
  await key("ArrowDown", document.body);
  assert.ok(row("who").classList.contains("active"));
  await key("ArrowUp", document.body);
  assert.ok(row("egn").classList.contains("active"));
});

test("typing a value: Enter keeps it, Escape does not", async () => {
  await openSource("egn");
  await click(byText(".menu-item", "Type a value…"));
  const input = $(".source input", row("egn"));
  assert.equal(document.activeElement, input);
  await type(input, "8001010000");
  await key("Enter", input);
  assert.deepEqual(lastBindings().egn, { value: "8001010000" });
  assert.equal(text(".chip", row("egn")), "Value");

  await openSource("egn");
  await click(byText(".menu-item", "Type a value…"));
  const again = $(".source input", row("egn"));
  assert.equal(again.value, "8001010000");
  await type(again, "something else");
  await key("Escape", again);
  assert.deepEqual(lastBindings().egn, { value: "8001010000" });
  assert.ok($(".sheet"), "Escape in the input does not close the form");
});

test("a vault field can be picked as the source", async () => {
  await openSource("who");
  await click(byText(".menu-item", "applicant.name — John Watson"));
  assert.deepEqual(lastBindings().who, { path: "applicant.name" });
  assert.equal(text(".chip", row("who")), "Vault");
});

test("Save and Generate answers the request and closes the form", async () => {
  await click(byText(".sheet-foot button", "Save and Generate"));
  const saved = backend.called("fill_save").at(-1);
  assert.equal(saved.args.request_id, 7);
  assert.equal(saved.args.generate, true);
  assert.equal(saved.args.name, "acme");
  assert.deepEqual(saved.args.profiles, { applicant: 1 });
  assert.equal($(".sheet"), null);
  assert.equal(text("#toast"), "2 documents generated");
  assert.equal(backend.db.requests.length, 0);
});
