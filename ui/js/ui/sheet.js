import { html, render, useEffect, useRef } from "../../vendor/preact-htm.js";
import { Icon } from "../lib.js";

const root = () => document.getElementById("sheet-root");
let resolveOpen = null;

/** Shows a full-window sheet; `content(close)` renders its parts. Resolves when it closes. */
export function openSheet(content) {
  closeSheet();
  return new Promise((resolve) => {
    resolveOpen = resolve;
    render(html`<div class="sheet"><div class="sheet-drag" data-tauri-drag-region></div>${content(closeSheet)}</div>`, root());
  });
}

export const isSheetOpen = () => resolveOpen !== null;

// the open sheet's Cancel, run on Escape
let onEscape = null;

/** Escape in this sheet does what its Cancel does: cancels a request, asks about unsaved changes. */
export function useSheetEscape(handler) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    onEscape = () => latest.current();
    return () => {
      onEscape = null;
    };
  }, []);
}

/** Escape: the open sheet's Cancel, or just closing it. False when no sheet is open. */
export function escapeSheet() {
  if (!isSheetOpen()) return false;
  (onEscape || closeSheet)();
  return true;
}

export function closeSheet() {
  render(null, root());
  const resolve = resolveOpen;
  resolveOpen = null;
  resolve?.();
}

export const SheetHead = ({ icon, title, reason }) => html`
  <div class="head-text">
    <div class="head-icon"><${Icon} name=${icon} /></div>
    <div style="min-width:0">
      <h2>${title}</h2>
      ${reason && html`<div class="reason">${reason}</div>`}
    </div>
  </div>`;
