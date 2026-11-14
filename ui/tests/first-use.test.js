import assert from "node:assert/strict";
import { before, test } from "node:test";
import { seed } from "./backend.js";
import { $, $$, action, byText, check, choose, click, startApp, submit, text, type } from "./setup.js";

let backend;
const signedOut = { ...seed().relay, signed_in: false, state: "signed_out", account: null, chat: false };

before(async () => {
  backend = await startApp({ initialized: false, unlocked: false, relay: signedOut, chats: {} });
});

test("the language is picked on the first screen", async () => {
  assert.equal(text(".gate h1"), "Create your vault");
  await choose($("#gate-language"), "bg");
  assert.deepEqual(backend.called("language_set").at(-1), { language: "bg" });
  assert.equal(text(".gate h1"), "Създайте трезора си");
  await choose($("#gate-language"), "en");
  assert.equal(text(".gate h1"), "Create your vault");
});

test("after the recovery key comes the wizard, not the app", async () => {
  const form = $(".gate form");
  await type($("input[name=password]", form), "secret-pw");
  await type($("input[name=confirm]", form), "secret-pw");
  await submit(form);
  assert.ok($("#gate-language"));
  await check($(".gate input[type=checkbox]"));
  await submit($(".gate form"));
  assert.equal(text(".gate h1"), "Blankey is better online");
  assert.ok($("#gate-language"));
  assert.equal($(".app"), null);
});

test("signing in waits for the browser, then shows the account", async () => {
  await click($("#welcome-signin"));
  assert.equal(backend.called("relay_login").length, 1);
  assert.equal(text("#welcome-waiting"), "Finish signing in in your browser.");
  assert.equal($("#welcome-signin").disabled, true);

  Object.assign(backend.db.relay, { signed_in: true, state: "connected", account: "you@example.com" });
  backend.emit("relay-status");
  await click(document.body);
  assert.equal(text(".welcome-account"), "you@example.com");
});

test("Continue lands on Cases, which points at the sample case", async () => {
  await click(byText(".welcome-actions button", "Continue"));
  assert.deepEqual(backend.called("first_use_set").at(-1), { stage: "case" });
  assert.ok($(".app"));
  assert.match(text(".empty"), /^Start with a sample case/);
  assert.equal($$("button.coach").length, 2);
});

test("New case makes the sample case and says what to ask an assistant", async () => {
  await action("New case");
  assert.equal($(".modal"), null);
  assert.equal(backend.called("create_sample_case").length, 1);
  assert.equal(text(".case-item.active .title"), "Sample: loan contract");
  assert.equal(text(".case-tabs .chip.active"), "AI assistant");
  assert.match(text(".sample-prompt"), /open the case “Sample: loan contract”/);
  await click(byText(".sample-guide button", "Copy"));
  assert.equal(backend.called("copy_text").at(-1).text, text(".sample-prompt"));
  assert.equal($$("button.coach").length, 0);
});

test("with the case chat, the sample case offers its own first questions", async () => {
  backend.db.relay.chat = true;
  backend.emit("relay-status");
  await click(document.body);
  assert.equal($(".sample-guide"), null);
  const chips = $$(".chat-starters .chip");
  assert.deepEqual(chips.map((c) => c.textContent), [
    "Make a template from the sample contract in this case",
    "Fill in the contract for the lender and the borrower",
  ]);
  await click(chips[0]);
  assert.equal(backend.called("send_chat_message").at(-1).text, "Make a template from the sample contract in this case");
});

test("the next New case asks for a title", async () => {
  await action("New case");
  assert.ok($(".modal input[name=title]"));
});
