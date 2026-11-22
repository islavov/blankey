import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, click, settle, startApp, submit, text, type } from "./setup.js";

let backend;

before(async () => {
  backend = await startApp({
    cases: [{ id: 1, title: "Car sale, Johnson", notes: "", created_at: "2026-11-02T09:00:00", updated_at: "2026-11-02T10:00:00" }],
    templates: [],
    sets: [],
    docs: [],
    activity: [],
  });
});

const tab = (label) => click($$(".case-tabs .chip").find((el) => el.textContent.trim().startsWith(label)));
const buttons = () => $$(".case-tab .empty-actions button").map((b) => b.textContent);
const relay = async (change) => {
  Object.assign(backend.db.relay, change);
  backend.emit("relay-status");
  await settle();
};

test("profiles: signed in to Blankey Online, a client is typed in, scanned or asked for through a link", async () => {
  await tab("Profiles");
  assert.equal(text(".empty-tab h3"), "No clients in this case yet");
  assert.equal(text(".empty-tab p"), "Saved once, reused in every case. Stored encrypted on this computer.");
  assert.deepEqual(buttons(), ["Add manually", "Scan ID card", "Request via secure link"]);
  assert.equal($(".case-tab .tab-actions"), null, "the empty tab offers its actions itself");

  await click(byText(".empty-actions button", "Scan ID card"));
  assert.ok(backend.called("pick_files").length > 0, "a scan is a file brought into the case");
  await click(byText(".sheet-foot button", "Cancel"));

  await click(byText(".empty-actions button", "Request via secure link"));
  await type($(".sheet input"), "Buyer details");
  await click($(".sheet-foot button.primary"));
  const { draft } = backend.called("create_link").at(-1);
  assert.equal(draft.case_id, 1, "what the client sends lands in this case");
  assert.equal(draft.new_profile, "New client");
});

test("profiles: without document recognition there is nothing to scan; signed out, only typing is left", async () => {
  backend.db.docConfigs.state = "none";
  await tab("Files");
  await tab("Profiles");
  assert.deepEqual(buttons(), ["Add manually", "Request via secure link"]);

  await relay({ signed_in: false, state: "signed_out", account: null });
  assert.deepEqual(buttons(), ["Add manually"]);
  assert.ok(text(".empty-tab").includes("Scanning ID cards and requesting details via a secure link need Blankey Online."));
  await click(byText(".empty-tab button", "Sign in"));
  assert.equal(backend.called("relay_login").length, 1);
});

test("profiles: Add manually makes a profile of the case, and the tab turns into its table", async () => {
  await click(byText(".empty-actions button", "Add manually"));
  await type($(".modal input[name=name]"), "Emily Clarke");
  await submit($(".modal form"));
  assert.equal(backend.called("create_profile").at(-1).caseId, 1);
  assert.equal($(".case-tab .empty-tab"), null);
  assert.deepEqual($$(".case-tab .tab-actions button").map((b) => b.textContent), ["New profile", "Add existing…"]);
});

test("templates: only the assistant makes one, so the tab leads to it", async () => {
  await tab("Templates");
  assert.equal(text(".empty-tab h3"), "No templates yet");
  assert.deepEqual(buttons(), ["Ask the AI"]);
  await click(byText(".empty-actions button", "Ask the AI"));
  assert.ok(text(".case-tabs .chip.active").startsWith("AI assistant"));
  assert.equal(text(".offer h3"), "Try the built-in assistant");
  assert.ok(byText(".offer button", "Create a profile"), "signed out: the profile comes first");
});

test("documents: Fill a document goes to the templates until there is exactly one to fill", async () => {
  await tab("Documents");
  assert.equal(text(".empty-tab h3"), "Filled documents appear here");
  assert.deepEqual(buttons(), ["Fill a document"]);
  await click(byText(".empty-actions button", "Fill a document"));
  assert.ok(text(".case-tabs .chip.active").startsWith("Templates"));

  backend.db.templates.push({ id: "car-sale", name: "Car sale contract", kind: "docx", blanks: 4, used_by: [], description: "", case_id: 1 });
  backend.emit("cases-changed");
  await settle();
  await tab("Documents");
  await click(byText(".empty-actions button", "Fill a document"));
  await settle(50);
  assert.ok($(".sheet"), "the fill form opens for the case's template");
});
