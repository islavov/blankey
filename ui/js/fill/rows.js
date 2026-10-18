import { useEffect, useRef } from "../../vendor/preact-htm.js";
import { t } from "../i18n.js";
import { html } from "../lib.js";
import { describe, kindLabel } from "./source.js";

/** One blank: its label and name, and the source with the current value. Click the source to change it. */
export function FillRow({ v, binding, row, suggested, active, typing, onSelect, onMenu, onTyped }) {
  const d = describe(v, binding, row, suggested);
  const tip = v.contexts.map(([template, context]) => `${template}: ${context}`).join("\n\n");
  const element = useRef();
  useEffect(() => {
    if (active) element.current.scrollIntoView({ block: "nearest" });
  }, [active]);

  return html`
    <div ref=${element} class=${`fill-row ${d.empty ? "is-empty" : ""} ${active ? "active" : ""}`} data-var=${v.name}>
      <div class="fill-field" title=${tip} onClick=${onSelect}>
        <div class="label">${v.label || v.name}</div>
        <div class="var">${v.name}</div>
      </div>
      ${typing
        ? html`<div class="source"><${TypeValue} value=${binding?.value ?? ""} onDone=${onTyped} /></div>`
        : html`
            <div class="source" tabindex="0" onClick=${(e) => onMenu(e.currentTarget)}>
              <span class=${`chip ${d.kind}`}>${kindLabel(d.kind)}</span>
              ${d.path && html`<span class="path">${d.path}</span>`}
              <span class="val">${d.text}</span><span class="caret">⌄</span>
            </div>`}
    </div>`;
}

/** Inline input: Enter or leaving the field keeps the value, Escape cancels. onDone(value | null) */
function TypeValue({ value, onDone }) {
  const input = useRef();
  const done = useRef(false);
  useEffect(() => {
    input.current.focus();
    input.current.select();
  }, []);
  const finish = (commit) => {
    if (done.current) return;
    done.current = true;
    onDone(commit ? input.current.value.trim() : null);
  };
  const key = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") finish(true);
    if (e.key === "Escape") finish(false);
  };
  return html`<input type="text" ref=${input} value=${value} placeholder=${t("Type a value")} spellcheck="false"
    onKeyDown=${key} onBlur=${() => finish(true)} />`;
}
