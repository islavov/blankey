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

test("a first case from the user's own document is named after the file and ends the first use", async () => {
  backend.db.pickPaths = ["/Users/test/Downloads/car_sale-contract.docx"];
  await click(byText(".start button", "Choose a file"));
  assert.deepEqual(backend.called("create_case").at(-1), { title: "car sale contract" });
  assert.equal(backend.called("import_case_files").at(-1).items[0].path, "/Users/test/Downloads/car_sale-contract.docx");
  assert.deepEqual(backend.called("first_use_set").at(-1), { stage: null });
  assert.equal($(".case-title").value, "car sale contract");
  assert.equal($(".guide"), null, "a case of the user's own has no guide");
  assert.ok(text(".case-tabs .chip.active").startsWith("Files"), "without the assistant a case opens on its files");
  assert.equal(backend.called("create_sample_case").length, 0);
});
