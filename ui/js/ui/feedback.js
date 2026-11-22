import { html, render, useEffect, useRef, useState } from "../../vendor/preact-htm.js";
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
// the open dialog's submit is running: it cannot be cancelled meanwhile
let running = false;

export const isDialogOpen = () => open;

export function closeDialog() {
  open = false;
  answer = null;
  running = false;
  render(null, root());
}

/** Escape anywhere: the open dialog answers Cancel. */
export const cancelDialog = () => !running && answer?.(null);

/** fields: [{name, label, value, type, options?: [[value, label]]}] -> resolves to {name: value}, or null when cancelled.
 * With `submit(values)` the dialog stays open while it runs and shows the message it throws; it closes when it succeeds. */
export function dialog({ title, text = "", fields = [], ok = t("OK"), danger = false, cancel = t("Cancel"), submit = null }) {
  return new Promise((resolve) => {
    const close = (value) => {
      closeDialog();
      resolve(value);
    };
    open = true;
    answer = close;
    render(html`<${Dialog} ...${{ title, text, fields, ok, danger, cancel }} onSubmit=${submit} onClose=${close} />`, root());
  });
}

export const confirmDialog = (title, text, ok = t("Delete")) => dialog({ title, text, ok, danger: true }).then(Boolean);
export const alertDialog = (title, text) => dialog({ title, text, ok: t("OK"), cancel: "" });

function Dialog({ title, text, fields, ok, danger, cancel, onSubmit, onClose }) {
  const form = useRef();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => (form.current.querySelector("input, select") || form.current.querySelector("button[type=submit]")).focus(), []);
  const submit = async (e) => {
    e.preventDefault();
    const values = Object.fromEntries(new FormData(form.current));
    if (!onSubmit) return onClose(values);
    setError("");
    setBusy((running = true));
    try {
      await onSubmit(values);
    } catch (err) {
      setBusy((running = false));
      // the backend's messages are English; the catalog has them
      setError(t(String(err)));
      form.current?.querySelector("input[type=password]")?.select();
      return;
    }
    running = false;
    onClose(values);
  };
  const cancelled = () => !busy && onClose(null);
  return html`
    <div class="modal">
      <form class="modal-card" autocomplete="off" ref=${form} onSubmit=${submit}
        onKeyDown=${(e) => e.key === "Escape" && (e.preventDefault(), cancelled())}>
        <h3>${title}</h3>
        ${text && html`<div class="muted" style="white-space: pre-line">${text}</div>`}
        ${fields.map(
          (f) => html`
            <label class="small muted">${f.label}</label>
            ${f.options
              ? html`<select name=${f.name}>${f.options.map(([value, label]) => html`<option value=${value} selected=${String(value) === String(f.value)}>${label}</option>`)}</select>`
              : html`<input type=${f.type || "text"} name=${f.name} value=${f.value || ""} spellcheck="false" />`}`
        )}
        ${onSubmit && html`<p class="error">${error}</p>`}
        <div class="modal-buttons">
          ${cancel && html`<button type="button" disabled=${busy} onClick=${cancelled}>${cancel}</button>`}
          <button type="submit" disabled=${busy} class=${danger ? "primary danger" : "primary"}>${ok}</button>
        </div>
      </form>
    </div>`;
}
