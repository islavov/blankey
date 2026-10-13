import { toast } from "./ui/feedback.js";
import { update } from "./lib.js";

export const { invoke } = window.__TAURI__.core;
export const { listen } = window.__TAURI__.event;

export const isLocked = (e) => String(e).includes("locked");

export async function refreshStatus() {
  const status = await invoke("status");
  update({ status });
  return status;
}

/** The relay sign-in shown in the sidebar; no network on the backend side. */
export async function refreshRelay() {
  update({ relay: await invoke("relay_summary") });
}

/** invoke() that shows errors as a toast and returns to the unlock screen when the vault got locked. */
export async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    if (isLocked(e)) await refreshStatus();
    else toast(String(e), 5000);
    throw e;
  }
}

export async function withBusy(action) {
  document.body.classList.add("busy");
  try {
    return await action();
  } finally {
    document.body.classList.remove("busy");
  }
}
