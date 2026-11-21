// Workspaces: separate sets of data, each with its own master password. One is open at a time; opening another
// relaunches Blankey into it. The lock screen has a select box with the open one, which lists them and adds one, new or restored from a
// local backup.

import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke } from "./api.js";
import { t } from "./i18n.js";
import { html, useStore } from "./lib.js";
import { dialog, toast } from "./ui/feedback.js";

/** The workspace that is open now. */
export const openWorkspace = (status) => status.workspaces.find((w) => w.id === status.workspace);

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

/** Up to two letters for a workspace's square: the first letters of its first two words. */
export function initials(name) {
  const words = name.trim().split(/\s+/).filter(Boolean);
  const letters = words.length > 1 ? words[0][0] + words[1][0] : (words[0] || "?").slice(0, 2);
  return letters.toUpperCase();
}

const CHEVRON = '<path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>';
const CHECK = '<path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>';
const Glyph = ({ cls, path }) => html`<svg class=${cls} viewBox="0 0 16 16" dangerouslySetInnerHTML=${{ __html: path }}></svg>`;

/** On the lock screen: a select box with the open workspace; its list opens another one or adds one. Opening
 * relaunches Blankey, which comes up locked in that workspace, so this is where workspaces are changed. */
export function WorkspacePick() {
  const { status } = useStore();
  const [open, setOpen] = useState(false);
  const box = useRef();
  useEffect(() => {
    if (!open) return undefined;
    const outside = (e) => !box.current.contains(e.target) && setOpen(false);
    // before the window's own Escape, which is for menus, dialogs and sheets
    const escape = (e) => e.key === "Escape" && (e.preventDefault(), setOpen(false));
    document.addEventListener("mousedown", outside);
    document.addEventListener("keydown", escape, true);
    return () => {
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("keydown", escape, true);
    };
  }, [open]);

  const choose = (action) => {
    setOpen(false);
    action();
  };
  const switchTo = (id) => id !== status.workspace && call("workspace_switch", { id }).catch(() => {});
  const { name } = openWorkspace(status);
  return html`
    <div class="ws-select" ref=${box}>
      <button type="button" id="workspace-pick" class=${`ws-box ${open ? "open" : ""}`} aria-haspopup="listbox" aria-expanded=${open}
        onClick=${() => setOpen(!open)}>
        <span class="ws-avatar">${initials(name)}</span>
        <span class="ws-text"><span class="ws-label">${t("Workspace")}</span><span class="ws-name">${name}</span></span>
        <${Glyph} cls="ws-chevron" path=${CHEVRON} />
      </button>
      ${open && html`
        <div class="ws-list" role="listbox">
          ${status.workspaces.map((w) => html`
            <div class=${`ws-option ${w.id === status.workspace ? "checked" : ""}`} role="option" aria-selected=${w.id === status.workspace}
              onClick=${() => choose(() => switchTo(w.id))}>
              <span class="ws-avatar">${initials(w.name)}</span><span class="ws-name">${w.name}</span>
              ${w.id === status.workspace && html`<${Glyph} cls="ws-check" path=${CHECK} />`}
            </div>`)}
          <div class="ws-sep"></div>
          <div class="ws-option action" onClick=${() => choose(createWorkspace)}><span class="ws-avatar">+</span><span class="ws-name">${t("New workspace…")}</span></div>
          <div class="ws-option action" onClick=${() => choose(restoreWorkspace)}><span class="ws-avatar">↺</span><span class="ws-name">${t("Restore from backup…")}</span></div>
        </div>`}
    </div>`;
}
