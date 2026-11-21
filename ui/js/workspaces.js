// Workspaces: separate sets of data, each with its own master password. One is open at a time; opening another
// relaunches Blankey into it. The rail on the left lists them and adds one, new or restored from a local backup.

import { call, invoke } from "./api.js";
import { t } from "./i18n.js";
import { html, useStore } from "./lib.js";
import { dialog, toast } from "./ui/feedback.js";
import { popup } from "./ui/menu.js";

/** The workspace that is open now. */
export const openWorkspace = (status) => status.workspaces.find((w) => w.id === status.workspace);

/** Up to two letters for a workspace's square: the first letters of its first two words. */
export function initials(name) {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || "?").slice(0, 2);
  return letters.toUpperCase();
}

/** "Clients 2026-11-20.blankey" names the workspace "Clients". */
export function nameFromBackup(path) {
  const file = path.split(/[\\/]/).pop();
  return file.replace(/\.blankey$/i, "").replace(/\s*\d{4}-\d{2}-\d{2}$/, "").trim();
}

function named(name) {
  if (!name.trim()) throw t("Enter a name for the workspace.");
  return name.trim();
}

const createWorkspace = () =>
  dialog({
    title: t("New workspace"),
    fields: [{ name: "name", label: t("Name") }],
    ok: t("Create"),
    submit: ({ name }) => invoke("workspace_create", { name: named(name) }),
  });

async function restoreWorkspace() {
  const path = await call("backup_pick").catch(() => null);
  if (!path) return;
  await dialog({
    title: t("Restore from backup"),
    fields: [
      { name: "name", label: t("Name"), value: nameFromBackup(path) },
      { name: "password", label: t("Master password of the backup"), type: "password" },
    ],
    ok: t("Restore"),
    submit: ({ name, password }) => invoke("workspace_restore", { path, name: named(name), password }),
  });
}

/** Saves the open workspace as one file, encrypted with its master password. */
export async function backupWorkspace() {
  let saved = null;
  await dialog({
    title: t("Local backup"),
    fields: [{ name: "password", label: t("Master password"), type: "password" }],
    ok: t("Back up"),
    submit: async ({ password }) => {
      saved = await invoke("backup_create", { password });
    },
  });
  if (saved) toast(t("Saved to {path}", { path: saved }), 4000);
}

/** Deletes the open workspace for good; its master password is asked for when it has one. */
export function deleteWorkspace(status) {
  const { name } = openWorkspace(status);
  const text = t("All profiles, cases, templates and documents of this workspace are permanently deleted from this computer. This cannot be undone.");
  const fields = status.initialized ? [{ name: "password", label: t("Master password"), type: "password" }] : [];
  return dialog({
    title: t("Delete {name}?", { name }),
    text,
    fields,
    ok: t("Delete workspace"),
    danger: true,
    submit: ({ password }) => invoke("workspace_delete", { password: password ?? null }),
  });
}

export function WorkspaceRail() {
  const { status } = useStore();
  const add = (e) => {
    const rect = e.currentTarget.getBoundingClientRect();
    popup(rect.right + 6, rect.top, [
      { label: t("New workspace…"), action: createWorkspace },
      { label: t("Restore from backup…"), action: restoreWorkspace },
    ], 200);
  };
  return html`
    <div class="rail" data-help="sidebar.workspaces">
      <div class="rail-top" data-tauri-drag-region></div>
      ${status.workspaces.map((w) => html`
        <button class=${`rail-item ${w.id === status.workspace ? "active" : ""}`} title=${w.name} aria-label=${w.name}
          onClick=${() => w.id !== status.workspace && call("workspace_switch", { id: w.id }).catch(() => {})}>${initials(w.name)}</button>`)}
      <button class="rail-add" id="workspace-add" title=${t("Add a workspace")} aria-label=${t("Add a workspace")} onClick=${add}>+</button>
    </div>`;
}
