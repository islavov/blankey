import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, click, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({
    workspaces: [
      { id: "default", name: "Personal", kind: "local" },
      { id: "w-2", name: "Oak Street Office", kind: "local" },
    ],
  });
});

const rail = () => $$(".rail-item").map((el) => `${el.textContent.trim()}${el.classList.contains("active") ? "*" : ""}`);

async function unlock() {
  await type($(".gate input[name=password]"), "pw");
  await submit($(".gate form"));
}

async function addMenu(label) {
  await click($("#workspace-add"));
  await click(byText(".menu-item", label));
}

test("the rail lists the workspaces by their initials and marks the open one", () => {
  assert.deepEqual(rail(), ["PE*", "OS"]);
  assert.equal($(".rail-item.active").title, "Personal");
});

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

test("a new workspace needs a name and opens on the create-vault screen", async () => {
  await click($("#workspace-add"));
  assert.deepEqual($$(".menu-item").map((el) => el.textContent), ["New workspace…", "Restore from backup…"]);
  await click(byText(".menu-item", "New workspace…"));
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Enter a name for the workspace.");
  assert.equal(backend.called("workspace_create").length, 0);

  await type($(".modal input[name=name]"), " Clients ");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_create"), [{ name: "Clients" }]);
  assert.equal(text(".gate h1"), "Create your vault");
  assert.deepEqual(rail(), ["PE", "OS", "CL*"]);
});

test("a workspace that was never set up is deleted without a password", async () => {
  await click($(".gate #workspace-delete"));
  assert.equal(text(".modal h3"), "Delete Clients?");
  assert.equal($(".modal input"), null);
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_delete"), [{ password: null }]);
  assert.deepEqual(rail(), ["PE*", "OS"]);
  assert.equal(text(".gate h1"), "Blankey is locked");
});

test("another workspace is opened from the lock screen", async () => {
  await click($$(".rail-item")[1]);
  assert.deepEqual(backend.called("workspace_switch"), [{ id: "w-2" }]);
  assert.deepEqual(rail(), ["PE", "OS*"]);
  assert.equal(text(".gate h1"), "Blankey is locked");

  await click($(".rail-item.active"));
  assert.equal(backend.called("workspace_switch").length, 1);
});

test("restore from backup: a cancelled picker opens nothing", async () => {
  backend.db.backupFile = null;
  await addMenu("Restore from backup…");
  assert.equal($(".modal"), null);
  delete backend.db.backupFile;
});

test("restore from backup adds a workspace named after the file and opens it", async () => {
  await addMenu("Restore from backup…");
  assert.equal(text(".modal h3"), "Restore from backup");
  assert.equal($(".modal input[name=name]").value, "Clients");
  await type($(".modal input[name=password]"), "nope");
  await submit($(".modal form"));
  assert.equal(text(".modal .error"), "Wrong password.");
  assert.deepEqual(rail(), ["PE", "OS*"]);

  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_restore").at(-1), { path: "/Users/test/Backups/Clients 2026-11-20.blankey", name: "Clients", password: "pw" });
  assert.deepEqual(rail(), ["PE", "OS", "CL*"]);
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
  assert.deepEqual(rail(), ["PE", "OS", "CL*"]);

  await type($(".modal input[name=password]"), "pw");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("workspace_delete").at(-1), { password: "pw" });
  assert.deepEqual(rail(), ["PE*", "OS"]);
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
