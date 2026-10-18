import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, action, byText, click, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp();
  await goTo("Connections");
});

const rows = () => $$(".grid tbody tr").map((tr) => $$("td", tr).map((td) => td.textContent.trim()));

test("shows the relay, its status and the accepted agents", () => {
  assert.equal(text(".toolbar h2"), "Connections");
  assert.equal(text("#relay-url"), "https://relay.blankey.io");
  assert.equal(text("#relay-mcp-url"), "https://relay.blankey.io/mcp");
  assert.equal(text("#relay-state"), "Connected");
  assert.ok($(".relay .pill"));
  assert.equal(rows().length, 1);
  assert.deepEqual(rows()[0], ["Claude", "claude.ai", "2026-10-11 09:30", "", "Revoke"]);
});

test("the account link opens the relay's account page", async () => {
  await click(byText(".relay button", "Account and billing"));
  assert.equal(backend.called("relay_open_account").length, 1);
});

test("revoke asks first", async () => {
  await click(byText(".grid button", "Revoke"));
  assert.equal(text(".modal h3"), "Revoke Claude?");
  await click(byText(".modal button", "Cancel"));
  assert.equal(backend.called("relay_revoke").length, 0);

  await click(byText(".grid button", "Revoke"));
  await submit($(".modal form"));
  assert.deepEqual(backend.called("relay_revoke"), [{ grantId: "g-1" }]);
  assert.equal($(".grid"), null);
  assert.ok(text(".empty").startsWith("No web agents yet."));
});

test("the sidebar shows the Blankey Online account above the MCP status", async () => {
  assert.equal(text("#relay-account"), "you@example.com");
  assert.equal(text("#relay-btn"), "you@example.com");
  assert.equal($("#relay-btn").title, "Blankey Online: you@example.com, Connected");
  const foot = [...document.querySelector(".sidebar-foot").children].map((el) => el.id || el.className);
  assert.deepEqual(foot, ["relay-btn", "mcp-status", "foot-actions"], "the account sits above the MCP status");
  await goTo("Activity");
  await click($("#relay-btn"));
  assert.equal(text(".toolbar h2"), "Connections");
});

test("sign out and in again", async () => {
  await action("Sign out");
  assert.equal(text("#relay-state"), "Signed out");
  assert.equal($("#relay-account"), null);
  assert.equal(text("#relay-btn"), "Sign in to Blankey Online");
  await click($("#relay-btn"));
  assert.equal(backend.called("relay_login").length, 1);
  assert.equal(text("#relay-state"), "Signing in…");
  assert.equal(text("#relay-btn"), "Signing in…");
  assert.ok($("#relay-btn").disabled);
});

test("another online server, then back to the official one", async () => {
  await click(byText(".relay button", "Another online server…"));
  await type($(".modal input[name=url]"), " http://relay.example.com ");
  await submit($(".modal form"));
  assert.equal(text("#toast"), "http://relay.example.com must use https://");
  assert.equal(text("#relay-url"), "https://relay.blankey.io");

  await click(byText(".relay button", "Another online server…"));
  await type($(".modal input[name=url]"), "https://relay.example.com");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("relay_set_url").at(-1), { url: "https://relay.example.com" });
  assert.equal(text("#relay-url"), "https://relay.example.com");
  assert.equal($(".relay .pill"), null);

  await click(byText(".relay button", "Reset to official"));
  assert.equal(text(".modal h3"), "Use the official server?");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("relay_set_url").at(-1), { url: null });
  assert.equal(text("#relay-url"), "https://relay.blankey.io");
});

test("relay status events refresh the page; the tray can open it", async () => {
  Object.assign(backend.db.relay, { signed_in: true, state: "offline", message: "relay unreachable" });
  backend.emit("relay-status");
  await settle();
  assert.equal(text("#relay-state"), "Offline");
  assert.equal(text(".relay-row .small"), "relay unreachable");

  await goTo("Documents");
  backend.emit("show-page", "connections");
  await settle();
  assert.equal(text(".toolbar h2"), "Connections");
});
