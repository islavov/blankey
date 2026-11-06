import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { t } from "../i18n.js";
import { html } from "../lib.js";
import { describe, kindLabel } from "./source.js";

/** One blank: its label and name, and the source with the current value. Click the source to type a value or pick one. */
export function FillRow({ v, binding, row, suggested, ask, profileName, active, editing, items, onSelect, onEdit, onTyped }) {
  const d = describe(v, binding, row, suggested, ask, profileName);
  const tip = v.contexts.map(([template, context]) => `${template}: ${context}`).join("\n\n");
  const element = useRef();
  useEffect(() => {
    if (active) element.current.scrollIntoView({ block: "nearest" });
  }, [active]);

  return html`
    <div ref=${element} class=${`fill-row ${d.empty ? "is-empty" : ""} ${active ? "active" : ""} ${editing ? "editing" : ""}`} data-var=${v.name}>
      <div class="fill-field" data-help="fill.field" title=${tip} onClick=${onSelect}>
        <div class="label">${v.label || v.name}</div>
        <div class="var">${v.name}</div>
      </div>
      ${editing
        ? html`<div class="source editing" data-help="fill.source">
            <${SourceEdit} value=${d.empty ? binding?.value ?? "" : d.text} hint=${ask} items=${items ?? []} onDone=${onTyped} />
          </div>`
        : html`
            <div class="source" data-help="fill.source" tabindex="0" onClick=${onEdit}>
              <span class=${`chip ${d.kind}`}>${kindLabel(d.kind)}</span>
              ${d.path && html`<span class="path">${d.path}</span>`}
              <span class="val">${d.text}</span><span class="caret">⌄</span>
            </div>`}
    </div>`;
}

const isPick = (item) => typeof item === "object" && "action" in item;

/** The sources whose label has the typed text, under their group's heading; all of them before anything is typed. */
function matching(items, query) {
  if (!query) return items;
  const shown = [];
  let head = null;
  for (const item of items) {
    if (item === "-") continue;
    if (item.head) head = item;
    else if (isPick(item) && item.label.toLowerCase().includes(query)) {
      if (head) shown.push(head);
      head = null;
      shown.push(item);
    }
  }
  return shown;
}

/**
 * Type a value, or pick a source from the list under the input, which narrows as you type. Enter keeps the typed
 * text, or the highlighted pick; leaving keeps the text only when it changed, so looking never turns a profile field
 * into a typed value; Escape leaves the blank as it was. onDone(value | null, viaEnter); a pick runs its own action.
 */
function SourceEdit({ value, hint, items, onDone }) {
  const input = useRef();
  const done = useRef(false);
  const [text, setText] = useState(value);
  const [highlight, setHighlight] = useState(-1);
  useEffect(() => {
    input.current.focus();
    input.current.select();
  }, []);
  const shown = matching(items, text === value ? "" : text.trim().toLowerCase());
  const picks = shown.filter(isPick);

  const finish = (commit, enter = false) => {
    if (done.current) return;
    done.current = true;
    const typed = input.current.value.trim();
    onDone(commit && typed !== value.trim() ? typed : null, enter);
  };
  const pick = (item) => {
    if (done.current) return;
    done.current = true;
    onDone(null, false);
    item.action();
  };
  const key = (e) => {
    e.stopPropagation();
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const last = picks.length - 1;
      setHighlight((h) => (e.key === "ArrowDown" ? Math.min(last, h + 1) : Math.max(-1, h - 1)));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (picks[highlight]) pick(picks[highlight]);
      else finish(true, true);
    } else if (e.key === "Escape") finish(false);
  };

  return html`
    <input type="text" ref=${input} value=${text} placeholder=${hint || t("Type a value")} spellcheck="false"
      onInput=${(e) => {
        setText(e.target.value);
        setHighlight(-1);
      }}
      onKeyDown=${key} onBlur=${() => finish(true)} />
    ${shown.length > 0 &&
    html`<div class="source-list">
      ${shown.map((item, i) => {
        if (item === "-") return html`<div key=${i} class="source-sep"></div>`;
        if (item.head) return html`<div key=${i} class="source-head">${item.head}</div>`;
        const cls = `source-pick${item.checked ? " checked" : ""}${item === picks[highlight] ? " active" : ""}`;
        // the input keeps the focus, so a pick is not taken for a typed value; it runs on click, after the mousedown
        // that would close a menu it opens
        return html`<div key=${i} class=${cls} onMouseDown=${(e) => e.preventDefault()} onClick=${() => pick(item)}>${item.label}</div>`;
      })}
    </div>`}`;
}
