import { t } from "../i18n.js";
import { html, update, useStore } from "../lib.js";

/** Toolbar (title, subtitle, action buttons, search) above the page content. actions: [label, onClick, class?, help?] */
export function Page({ title, subtitle = "", actions = [], children }) {
  const { search } = useStore();
  return html`
    <header class="toolbar" data-tauri-drag-region>
      <div class="toolbar-titles" data-tauri-drag-region>
        <h2 data-tauri-drag-region>${title}</h2>
        <div class="muted small" data-tauri-drag-region>${subtitle}</div>
      </div>
      <div class="toolbar-actions">
        <span id="page-actions">
          ${actions.map(([label, onClick, cls = "", help]) => html`<button class=${cls} data-help=${help} onClick=${onClick}>${label}</button>`)}
        </span>
        <input id="search" type="search" data-help="search" placeholder=${t("Search")} spellcheck="false" value=${search}
          onInput=${(e) => update({ search: e.target.value })} />
      </div>
    </header>
    <section class="page">${children}</section>`;
}

export const Empty = ({ children }) => html`<div class="empty">${children}</div>`;
