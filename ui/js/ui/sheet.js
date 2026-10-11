import { html, render } from "../../vendor/preact-htm.js";
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
