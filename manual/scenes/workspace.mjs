// Setting up a workspace: the vault and its recovery key, the first-launch pages, locking, and the workspaces
// themselves (another one, a backup, restoring one, deleting one).

const PASSWORD = "pw";

export default {
  id: "workspace",
  start: "first",
  async run(app) {
    await app.fill("input[name=password]", PASSWORD);
    await app.fill("input[name=confirm]", PASSWORD);
    await app.shot("create-vault", { target: ".gate-card" });
    await app.click(".gate button.primary");
    await app.shot("recovery-key", { target: ".gate-card" });
    await app.check(".gate input[type=checkbox]");
    await app.click(".gate button.primary");
    await app.shot("welcome", { target: ".gate-card" });
    await app.click({ text: "Continue offline" });
    await app.shot("connect-assistant", { target: ".gate-card" });
    await app.click({ text: "Continue" });
    await app.shot("first-case");
    await app.click({ text: "Skip" });

    await app.settings();
    await app.click("#backup-create");
    await app.fill("#modal-root input[name=password]", PASSWORD);
    await app.shot("backup", { target: ".modal-card" });
    await app.press("Escape");

    await app.backend((backend) => backend.db.workspaces.push({ id: "w-2", name: "Oak Street Office", kind: "local" }));
    await app.shot("lock", { target: ".sidebar-foot", mark: "#lock-btn" });
    await app.click("#lock-btn");
    await app.shot("unlock", { target: ".gate-card" });
    await app.click("#workspace-pick");
    await app.shot("workspaces");
    await app.click({ text: "New workspace…" });
    await app.fill("#modal-root input[name=name]", "Main Road Shop");
    await app.shot("new-workspace", { target: ".modal-card" });
    await app.press("Escape");
    await app.click("#workspace-pick");
    await app.click({ text: "Restore from backup…" });
    await app.fill("#modal-root input[name=password]", PASSWORD);
    await app.shot("restore", { target: ".modal-card" });
    await app.press("Escape");

    await app.fill("input[name=password]", PASSWORD);
    await app.click(".gate button.primary");
    await app.settings();
    await app.click("#workspace-delete");
    await app.shot("delete-workspace", { target: ".modal-card" });
    await app.press("Escape");
  },
};
