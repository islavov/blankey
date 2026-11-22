import assert from "node:assert/strict";
import { before, test } from "node:test";
import { seed } from "./backend.js";
import { $, $$, byText, click, settle, startApp, text } from "./setup.js";

let backend;

before(async () => {
  backend = await startApp({
    cases: [{ id: 1, title: "Sample: loan contract", notes: "", sample: true, sample_id: "loan", created_at: "2026-11-02T09:00:00", updated_at: "2026-11-02T10:00:00" }],
    relay: { ...seed().relay, chat: true },
    chats: {},
    sets: [],
    docs: [],
    activity: [],
  });
});

const tab = (label) => click($$(".case-tabs .chip").find((el) => el.textContent.trim().startsWith(label)));
const current = () => text(".guide-steps li.current");
const done = () => $$(".guide-steps li.done").length;
/** What an assistant did to the case, built-in or the user's own: the backend says the cases changed. */
async function changed(change) {
  change(backend.db);
  backend.emit("cases-changed");
  await settle();
}

test("the guide sits on top of the case on every tab", async () => {
  assert.equal(current(), "1Ask for a template");
  assert.equal(text(".guide-prompt"), "PromptMake a template from the sample contract in this case");
  for (const label of ["Files", "Profiles", "Templates", "AI assistant"]) {
    await tab(label);
    assert.ok($(".guide"), label);
  }
});

test("a template in the case moves the guide to the second prompt", async () => {
  await changed((db) => db.templates.push({ id: "sample-loan", name: "Loan contract", kind: "docx", blanks: 6, used_by: [], description: "", case_id: 1 }));
  assert.equal(done(), 1);
  assert.equal(current(), "2Ask to fill it in");
  assert.equal(text(".guide-prompt"), "PromptFill in the contract for the lender and the borrower");
});

test("once the assistant asked for the fill, the last step opens the fill form", async () => {
  await changed((db) => db.sets.push({ id: 7, name: "Loan contract", templates: ["sample-loan"], profiles: ["Maria Example"], updated_at: "2026-11-02T11:00:00", case_id: 1 }));
  assert.equal(current(), "3Check and generate");
  assert.equal($(".guide-prompt"), null, "nothing to ask: the step is the user's own");
  assert.deepEqual($$(".guide-actions button").map((b) => b.textContent), ["Open the fill form"]);
  await click(byText(".guide-actions button", "Open the fill form"));
  await settle(50);
  assert.ok($(".sheet"), "the fill form opens");
  assert.equal($(".sheet .guide"), null, "the guide stays out of the fill form");
  await click(byText(".sheet-foot button", "Cancel"));
});

test("a document ends the guide with the way to it and to a case of the user's own", async () => {
  await changed((db) => db.docs.push({ id: 31, title: "Loan contract", template_id: "sample-loan", filename: "loan.docx", pages: 1, created_at: "2026-11-02T11:30:00", case_id: 1 }));
  assert.equal(done(), 3);
  assert.equal(current(), null);
  assert.equal(text(".guide-text h3"), "Your first document is ready");
  assert.equal(text(".guide-text p"), "Loan contract");
  assert.deepEqual($$(".guide-actions button").map((b) => b.textContent), ["Open document", "Try with my own document"]);
  await click(byText(".guide-actions button", "Open document"));
  await settle(50);
  assert.deepEqual(backend.called("document_preview").at(-1), { id: 31 });
});

test("Hide guide takes it off the case for good", async () => {
  await click(byText(".sheet button", "Close"));
  await click(byText(".guide button", "Hide guide"));
  assert.deepEqual(backend.called("sample_guide_hide").at(-1), { caseId: 1 });
  assert.equal($(".guide"), null);
  assert.equal(text(".case-item.active .title"), "Sample: loan contract");
});
