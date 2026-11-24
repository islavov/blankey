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
  assert.equal(text(".gate-logo .beta-tag"), "Beta");
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

test("Continue lands on Cases, which offers the sample case and a start from the user's own document", async () => {
  await click(byText(".welcome-actions button", "Continue"));
  assert.deepEqual(backend.called("first_use_set").at(-1), { stage: "case" });
  assert.ok($(".app"));
  assert.equal(text(".start-label"), "Welcome to Blankey");
  assert.equal(text(".start h1"), "Fill legal documents in minutes.");
  assert.deepEqual($$(".start-card h2").map((el) => el.textContent), ["Try the sample case", "Start with my own document"]);
  assert.equal(text(".start-card p"), "An invented loan contract and two invented people. About 2 minutes.");
  assert.deepEqual($$("button.coach").map((b) => b.textContent), ["Open the sample case"]);
  assert.equal($(".modal"), null, "nothing pops up");
});

test("the sample case opens with a guide that says what to ask an assistant", async () => {
  await click(byText(".start button", "Open the sample case"));
  assert.deepEqual(backend.called("create_sample_case").at(-1), { sample: "loan" });
  assert.equal(text(".case-item.active .title"), "Sample: loan contract");
  assert.equal(text(".case-tabs .chip.active"), "AI assistant");
  assert.deepEqual($$(".guide-steps li").map((li) => li.textContent.trim()), ["1Ask for a template", "2Ask to fill it in", "3Check and generate"]);
  assert.equal(text(".guide-steps li.current"), "1Ask for a template");
  assert.equal(text(".guide-text h3"), "Ask the assistant for a template");
  // signed in, yet the account has no built-in assistant: the prompt is for an assistant of the user's own
  assert.deepEqual($$(".guide-actions button").map((b) => b.textContent), ["Copy"]);
  await click(byText(".guide-actions button", "Copy"));
  assert.equal(backend.called("copy_text").at(-1).text, "In Blankey, open the case “Sample: loan contract”. Make a template from the sample contract in this case");
  assert.equal(text(".offer h3"), "Try the built-in assistant");
  assert.equal($$("button.coach").length, 0);
});

test("with the case chat, the guide asks the assistant itself and the chat offers the same questions", async () => {
  backend.db.relay.chat = true;
  backend.emit("relay-status");
  await click(document.body);
  assert.equal($(".offer"), null);
  assert.deepEqual($$(".chat-starters .chip").map((c) => c.textContent), [
    "Make a template from the sample contract in this case",
    "Fill in the contract for the lender and the borrower",
  ]);
  assert.deepEqual($$(".guide-actions button").map((b) => b.textContent), ["Ask the assistant", "Copy"]);
  await click(byText(".guide-actions button", "Ask the assistant"));
  assert.deepEqual(backend.called("send_chat_message").at(-1), { caseId: 1, text: "Make a template from the sample contract in this case" });
  await click(byText(".guide-actions button", "Copy"));
  assert.equal(backend.called("copy_text").at(-1).text, "Make a template from the sample contract in this case");
});

test("New case asks for a title", async () => {
  await action("New case");
  assert.ok($(".modal input[name=title]"));
});
