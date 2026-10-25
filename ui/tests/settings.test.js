import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, check, choose, click, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp();
  await click($("#settings-btn"));
});

const rows = () => $$(".grid tbody tr").map((tr) => $$("td", tr).map((td) => td.textContent.trim()));

test("the cog opens the settings; they are not in the sidebar list", () => {
  assert.equal(text(".toolbar h2"), "Settings");
  assert.ok($("#settings-btn").classList.contains("active"));
  assert.ok(!$$(".nav-item").some((el) => el.textContent.includes("Settings") || el.textContent.includes("Connections")));
});

test("the MCP server status sits in the settings, not in the sidebar", () => {
  assert.equal(text("#mcp-state"), "MCP on port 8765");
  assert.equal(text("#mcp-url"), "http://127.0.0.1:8765/mcp");
  assert.equal($(".sidebar .mcp-status"), null);
});

test("the language picked here holds until set back to automatic", async () => {
  assert.deepEqual($$("#language option").map((o) => [o.value, o.textContent]), [["", "Default"], ["en", "English"], ["bg", "Български"]]);
  assert.equal($("#language").value, "");

  await choose($("#language"), "bg");
  assert.deepEqual(backend.called("language_set").at(-1), { language: "bg" });
  assert.equal(document.documentElement.lang, "bg");
  assert.equal(text(".toolbar h2"), "Настройки");
  assert.equal($("#language").value, "bg");

  await choose($("#language"), "");
  assert.deepEqual(backend.called("language_set").at(-1), { language: null });
  assert.equal(document.documentElement.lang, "en");
  assert.equal(text(".toolbar h2"), "Settings");
});

test("shows the relay, its status and the accepted agents", () => {
  assert.equal(text("#relay-url"), "https://relay.blankey.io");
  assert.equal(text("#relay-mcp-url"), "https://relay.blankey.io/mcp");
  assert.equal(text("#relay-state"), "Connected");
  assert.ok($(".relay .pill"));
  assert.equal(rows().length, 1);
  assert.deepEqual(rows()[0], ["Claude", "claude.ai", "2026-10-11 09:30", "", "Revoke"]);
});

test("anonymous statistics are on until the user turns them off", async () => {
  assert.equal($("#telemetry").checked, true);
  await check($("#telemetry"), false);
  assert.deepEqual(backend.called("telemetry_set"), [{ enabled: false }]);
  await settle();
  assert.equal($("#telemetry").checked, false);
});

test("check now downloads a new version and offers to restart into it", async () => {
  assert.equal(text("#app-version"), "0.2.5");
  assert.equal(text("#update-state"), "");
  await click(byText(".relay button", "Check now"));
  await settle();
  assert.equal(text("#update-state"), "0.3.0 is ready");
  await click(byText(".relay button", "Restart to update"));
  assert.equal(backend.called("update_install").length, 1);
});

test("automatic updates are on until the user turns them off", async () => {
  assert.equal($("#auto-update").checked, true);
  await check($("#auto-update"), false);
  assert.deepEqual(backend.called("update_auto_set"), [{ enabled: false }]);
  await settle();
  assert.equal($("#auto-update").checked, false);
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

test("the sidebar shows the Blankey Online account above the buttons", async () => {
  assert.equal(text("#relay-account"), "you@example.com");
  assert.equal(text("#relay-btn"), "you@example.com");
  assert.equal($("#relay-btn").title, "Blankey Online: you@example.com, Connected");
  const foot = [...document.querySelector(".sidebar-foot").children].map((el) => el.id || el.className);
  assert.deepEqual(foot, ["relay-btn", "foot-actions"]);
  await goTo("Activity");
  await click($("#relay-btn"));
  assert.equal(text(".toolbar h2"), "Settings");
});

test("sign out and in again, next to the online server", async () => {
  await click(byText(".relay button", "Sign out"));
  assert.equal(text("#relay-state"), "Signed out");
  assert.equal($("#relay-account"), null);
  assert.equal($("#page-actions button"), null);
  assert.ok(byText(".relay button", "Sign in").classList.contains("primary"));
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
  assert.equal(text("#relay-state + .small"), "relay unreachable");

  await goTo("Documents");
  backend.emit("show-page", "settings");
  await settle();
  assert.equal(text(".toolbar h2"), "Settings");
});
