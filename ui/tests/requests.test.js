import assert from "node:assert/strict";
import { before, test } from "node:test";
import { fileRequest, fillRequest, profileInputRequest } from "./backend.js";
import { $, $$, byText, check, click, key, settle, startApp, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({ requests: [profileInputRequest(8)] });
});

test("profile input: required fields are checked, values go to the vault", async () => {
  assert.equal(text(".sheet h2"), "2 fields for John");
  assert.equal(text(".sheet .reason"), "Need contact details");
  const email = $("input[name=f0]");
  assert.equal(email.value, "old@example.com");

  await type(email, "");
  await click(byText(".sheet-foot button", "Save"));
  assert.equal(text(".problems"), "Email is required");
  assert.ok($(".sheet"));

  await type(email, "john@example.com");
  await check($("input[name=f1]"));
  await click(byText(".sheet-foot button", "Save"));
  const resolved = backend.called("resolve_profile_input").at(-1);
  assert.deepEqual(resolved.values, { email: "john@example.com", vip: "yes" });
  assert.equal(resolved.profileName, null);
  assert.equal($(".sheet"), null);
});

test("a request that arrives while the app is open is shown; Escape cancels it", async () => {
  backend.db.requests.push(fillRequest(9));
  backend.emit("request-created", 9);
  await settle(300);
  assert.equal(text(".sheet h2"), "Your AI assistant wants to fill in 2 documents");
  assert.ok($$(".fill-row").length > 0);

  await key("Escape", document.body);
  const closed = backend.called("fill_close").at(-1);
  assert.equal(closed.args.request_id, 9);
  assert.equal(closed.args.fill_set_id, null);
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});

test("file request: the user picks the file; closing the picker keeps the request open", async () => {
  backend.db.requests.push(fileRequest(10));
  backend.emit("request-created", 10);
  await settle(300);
  assert.equal(text(".sheet h2"), "Your AI assistant asks for a PDF file");
  assert.equal(text(".sheet .reason"), "The blank NZOK form");

  backend.db.pickedFile = null;
  await click(byText(".sheet-foot button", "Choose file…"));
  assert.ok($(".sheet"));
  assert.equal(backend.db.requests.length, 1);

  backend.db.pickedFile = "nzok.pdf";
  await click(byText(".sheet-foot button", "Choose file…"));
  assert.deepEqual(backend.called("resolve_file_request").at(-1), { id: 10 });
  assert.equal(text("#toast"), "Shared nzok.pdf");
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});

test("file request: Cancel cancels it", async () => {
  backend.db.requests.push(fileRequest(11));
  backend.emit("request-created", 11);
  await settle(300);
  await click(byText(".sheet-foot button", "Cancel"));
  assert.deepEqual(backend.called("cancel_request").at(-1), { id: 11 });
  assert.equal($(".sheet"), null);
});
