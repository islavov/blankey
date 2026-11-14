import assert from "node:assert/strict";
import { before, test } from "node:test";
import { seed } from "./backend.js";
import { $, $$, byText, click, startApp, text } from "./setup.js";

let backend;
const signedOut = { ...seed().relay, signed_in: false, state: "signed_out", account: null, chat: false };

before(async () => {
  backend = await startApp({ firstUse: "wizard", relay: signedOut });
});

test("offline, the wizard connects an assistant on this computer", async () => {
  assert.equal(text(".gate h1"), "Blankey is better online");
  await click(byText(".welcome-actions button", "Continue offline"));
  assert.equal(text(".gate h1"), "Connect an AI assistant");
  assert.equal(text("#welcome-mcp"), "http://127.0.0.1:8765/mcp");

  await click(byText(".welcome-agents button", "Claude Desktop"));
  assert.equal(backend.called("connect_claude_desktop").length, 1);
  assert.equal(text("#toast"), "Blankey was added to claude_desktop_config.json. Restart Claude Desktop to see it.");

  // ChatGPT has no way to a server on this computer: it is sent to Blankey Online
  await click(byText(".welcome-agents button", "ChatGPT"));
  await click(byText("#welcome-chatgpt button", "Sign in to Blankey Online"));
  assert.equal(text(".gate h1"), "Blankey is better online");
  await click(byText(".welcome-actions button", "Continue offline"));
});

test("Back returns to the first page, Continue goes on to Cases", async () => {
  await click(byText(".welcome-actions button", "Back"));
  assert.equal(text(".gate h1"), "Blankey is better online");
  await click(byText(".welcome-actions button", "Continue offline"));
  await click(byText(".welcome-actions button", "Continue"));
  assert.deepEqual(backend.called("first_use_set").at(-1), { stage: "case" });
  assert.ok($(".app"));
});

test("Skip leaves the ordinary empty Cases page", async () => {
  await click(byText(".empty button", "Skip"));
  assert.deepEqual(backend.called("first_use_set").at(-1), { stage: null });
  assert.match(text(".empty"), /^No cases yet/);
  assert.equal($$("button.coach").length, 0);
  assert.equal(backend.called("create_sample_case").length, 0);
});
