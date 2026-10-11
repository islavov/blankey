import assert from "node:assert/strict";
import { before, test } from "node:test";
import { fillRequest, profileInputRequest } from "./backend.js";
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
