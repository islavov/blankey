import { html, render, useEffect, useRef } from "../../vendor/preact-htm.js";
import { t } from "../i18n.js";

export function toast(message, ms = 2600) {
  const el = document.getElementById("toast");
  el.textContent = message;
  el.classList.remove("hidden");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => el.classList.add("hidden"), ms);
}

const root = () => document.getElementById("modal-root");
let open = false;
// answers the open dialog
let answer = null;

export const isDialogOpen = () => open;

export function closeDialog() {
  open = false;
  answer = null;
  render(null, root());
}

/** Escape anywhere: the open dialog answers Cancel. */
export const cancelDialog = () => answer?.(null);

/** fields: [{name, label, value, type, options?: [[value, label]]}] -> resolves to {name: value}, or null when cancelled */
export function dialog({ title, text = "", fields = [], ok = t("OK"), danger = false, cancel = t("Cancel") }) {
  return new Promise((resolve) => {
    const close = (value) => {
      closeDialog();
      resolve(value);
    };
    open = true;
    answer = close;
    render(html`<${Dialog} ...${{ title, text, fields, ok, danger, cancel }} onClose=${close} />`, root());
  });
}

export const confirmDialog = (title, text, ok = t("Delete")) => dialog({ title, text, ok, danger: true }).then(Boolean);
export const alertDialog = (title, text) => dialog({ title, text, ok: t("OK"), cancel: "" });

function Dialog({ title, text, fields, ok, danger, cancel, onClose }) {
  const form = useRef();
  useEffect(() => (form.current.querySelector("input, select") || form.current.querySelector("button[type=submit]")).focus(), []);
  const submit = (e) => {
    e.preventDefault();
    onClose(Object.fromEntries(new FormData(e.currentTarget)));
  };
  return html`
    <div class="modal">
      <form class="modal-card" autocomplete="off" ref=${form} onSubmit=${submit}
        onKeyDown=${(e) => e.key === "Escape" && (e.preventDefault(), onClose(null))}>
        <h3>${title}</h3>
        ${text && html`<div class="muted" style="white-space: pre-line">${text}</div>`}
        ${fields.map(
          (f) => html`
            <label class="small muted">${f.label}</label>
            ${f.options
              ? html`<select name=${f.name}>${f.options.map(([value, label]) => html`<option value=${value} selected=${String(value) === String(f.value)}>${label}</option>`)}</select>`
              : html`<input type=${f.type || "text"} name=${f.name} value=${f.value || ""} spellcheck="false" />`}`
        )}
        <div class="modal-buttons">
          ${cancel && html`<button type="button" onClick=${() => onClose(null)}>${cancel}</button>`}
          <button type="submit" class=${danger ? "primary danger" : "primary"}>${ok}</button>
        </div>
      </form>
    </div>`;
}
