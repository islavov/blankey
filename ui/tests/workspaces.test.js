import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, click, key, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({
    workspaces: [
      { id: "default", name: "Personal", kind: "local" },
      { id: "w-2", name: "Oak Street Office", kind: "local" },
    ],
  });
});

/** The workspaces in the lock screen's menu, the open one marked with *. */
async function listed() {
  await click($("#workspace-pick"));
  const items = $$(".ws-option:not(.action)").map((el) => `${text(".ws-name", el)}${el.classList.contains("checked") ? "*" : ""}`);
  await key("Escape");
  assert.equal($(".ws-list"), null);
  return items;
}

async function pick(label) {
  await click($("#workspace-pick"));
  await click(byText(".ws-option .ws-name", label));
}

async function unlock() {
  await type($(".gate input[name=password]"), "pw");
  await submit($(".gate form"));
}

test("local backup asks for the master password and says where the file went", async () => {
  await click($("#settings-btn"));
  assert.equal(text("#workspace-name"), "Personal");
  await click($("#backup-create"));
  assert.equal(text(".modal h3"), "Local backup");
  await type($(".modal input[name=password]"), "nope");
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Wrong password.");

  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.equal($(".modal"), null);
  assert.equal(text("#toast"), "Saved to /Users/test/Backups/Personal 2026-11-21.blankey");
});

test("a cancelled save picker says nothing", async () => {
  backend.db.backupTarget = null;
  $("#toast").textContent = "";
  await click($("#backup-create"));
  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.equal($(".modal"), null);
  assert.equal(text("#toast"), "");
});

test("the lock screen names the open workspace and lists them all", async () => {
  assert.equal($("#workspace-pick"), null);
  await click($("#lock-btn"));
  assert.equal(text("#workspace-pick .ws-name"), "Personal");
  assert.equal(text("#workspace-pick .ws-avatar"), "PE");
  await click($("#workspace-pick"));
  assert.deepEqual($$(".ws-option").map((el) => text(".ws-name", el)), ["Personal", "Oak Street Office", "New workspace…", "Restore from backup…"]);
  assert.deepEqual($$(".ws-option .ws-avatar").map((el) => el.textContent), ["PE", "OS", "+", "↺"]);
  await click($(".gate h1"));
  assert.equal($(".ws-list"), null, "a click outside closes the list");
  assert.deepEqual(await listed(), ["Personal*", "Oak Street Office"]);
});

test("a new workspace needs a name and opens on the create-vault screen", async () => {
  await pick("New workspace…");
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Enter a name for the workspace.");
  assert.equal(backend.called("workspace_create").length, 0);

  await type($(".modal input[name=name]"), " Clients ");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_create"), [{ name: "Clients" }]);
  assert.equal(text(".gate h1"), "Create your vault");
  assert.equal(text("#workspace-pick .ws-name"), "Clients");
  assert.deepEqual(await listed(), ["Personal", "Oak Street Office", "Clients*"]);
});

test("a workspace that was never set up is deleted without a password", async () => {
  await click($(".gate #workspace-delete"));
  assert.equal(text(".modal h3"), "Delete Clients?");
  assert.equal($(".modal input"), null);
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_delete"), [{ password: null }]);
  assert.deepEqual(await listed(), ["Personal*", "Oak Street Office"]);
  assert.equal(text(".gate h1"), "Blankey is locked");
});

test("another workspace is opened from the lock screen", async () => {
  await pick("Oak Street Office");
  assert.deepEqual(backend.called("workspace_switch"), [{ id: "w-2" }]);
  assert.equal(text("#workspace-pick .ws-name"), "Oak Street Office");
  assert.equal(text(".gate h1"), "Blankey is locked");

  await pick("Oak Street Office");
  assert.equal(backend.called("workspace_switch").length, 1);
});

test("restore from backup: a cancelled picker opens nothing", async () => {
  backend.db.backupFile = null;
  await pick("Restore from backup…");
  assert.equal($(".modal"), null);
  delete backend.db.backupFile;
});

test("restore from backup adds a workspace named after the file and opens it", async () => {
  await pick("Restore from backup…");
  assert.equal(text(".modal h3"), "Restore from backup");
  assert.equal($(".modal input[name=name]").value, "Clients");
  await type($(".modal input[name=password]"), "nope");
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Wrong password.");
  assert.equal(backend.db.workspaces.length, 2);

  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_restore").at(-1), { path: "/Users/test/Backups/Clients 2026-11-20.blankey", name: "Clients", password: "pw" });
  assert.deepEqual(await listed(), ["Personal", "Oak Street Office", "Clients*"]);
  assert.equal(text(".gate h1"), "Blankey is locked");
});

test("deleting a workspace needs its master password and lands in another one", async () => {
  await unlock();
  await click($("#settings-btn"));
  assert.equal(text("#workspace-name"), "Clients");
  await click($("#workspace-delete"));
  assert.equal(text(".modal h3"), "Delete Clients?");
  assert.ok(text(".modal").includes("permanently deleted from this computer"));
  assert.ok($(".modal button[type=submit]").classList.contains("danger"));
  await click(byText(".modal button", "Cancel"));
  assert.equal(backend.called("workspace_delete").length, 1);

  await click($("#workspace-delete"));
  await type($(".modal input[name=password]"), "nope");
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Wrong password.");
  assert.equal(backend.db.workspaces.length, 3);

  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_delete").at(-1), { password: "pw" });
  assert.deepEqual(await listed(), ["Personal*", "Oak Street Office"]);
  assert.equal(text(".gate h1"), "Blankey is locked");
});

test("the only workspace cannot be deleted", async () => {
  await unlock();
  await click($("#settings-btn"));
  assert.equal($("#workspace-delete").disabled, false);
  backend.db.workspaces = backend.db.workspaces.slice(0, 1);
  await click($("#lock-btn"));
  await unlock();
  await click($("#settings-btn"));
  assert.equal($("#workspace-delete").disabled, true);
});
