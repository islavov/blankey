import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call } from "./api.js";
import { t, tn } from "./i18n.js";
import { html, Icon } from "./lib.js";
import { Preview } from "./preview.js";
import { openSheet, SheetHead } from "./ui/sheet.js";

/** The selection inside the preview as a span of one block: UTF-16 offsets from the runs' data-o. */
export function selectionSpan(root) {
  const selection = window.getSelection();
  if (!selection || selection.isCollapsed || !selection.rangeCount) return null;
  const range = selection.getRangeAt(0);
  const point = (node, offset) => {
    const run = (node.nodeType === Node.TEXT_NODE ? node.parentElement : node)?.closest("[data-o]");
    if (!run || !root.contains(run)) return null;
    const before = document.createRange();
    before.selectNodeContents(run);
    before.setEnd(node, offset);
    return { block: Number(run.dataset.b), offset: Number(run.dataset.o) + before.toString().length };
  };
  const start = point(range.startContainer, range.startOffset);
  const end = point(range.endContainer, range.endOffset);
  if (!start || !end || start.block !== end.block || end.offset <= start.offset) return null;
  return { block: start.block, start: start.offset, end: end.offset, rect: range.getBoundingClientRect() };
}

/** Opens the review of a library file in its own sheet. */
export const openReview = (file) =>
  openSheet((close) => html`
    <div class="sheet-head">
      <${SheetHead} icon="lock" title=${t("Personal data in {name}", { name: file.title || file.rel_path.split("/").pop() })}
        reason=${t("Your AI assistant sees hidden values masked, like Xxxx Xxxxxx.")} />
    </div>
    <div class="sheet-body"><${PiiReview} source=${`lib:${file.id}`} /></div>
    <div class="sheet-foot">
      <span class="note">${t("Select text to hide it too. Click a marked value to change it.")}</span>
      <button class="primary" onClick=${close}>${t("Done")}</button>
    </div>`);

/** The id an element of the preview stands for: a hidden entity, or a value the user let through (shown:i). */
function itemOf(el) {
  if (el.dataset.e) return el.dataset.e;
  if (el.dataset.v !== undefined) return `shown:${el.dataset.v}`;
  return el.dataset.var;
}

const MARKS = "mark[data-e], mark[data-v], [data-var]";

/**
 * What the assistant does not see in a file: the preview with marked values and their list. Selecting text hides
 * it everywhere in the file; a marked value can be shown to the assistant, hidden again, or removed when the user
 * added it. Corrections are kept for the file.
 */
export function PiiReview({ source }) {
  const [review, setReview] = useState(null);
  const [menu, setMenu] = useState(null);
  const [focus, setFocus] = useState(null);
  const [typed, setTyped] = useState("");
  const area = useRef();

  useEffect(() => {
    setReview(null);
    call("pii_review", { source }).then(setReview).catch(() => {});
  }, [source]);

  // the focused item is highlighted in the preview, its first place scrolled into view once
  useEffect(() => {
    const marks = [...(area.current?.querySelectorAll(MARKS) || [])];
    marks.forEach((el) => el.classList.toggle("focus", itemOf(el) === focus?.id));
    if (focus?.scroll) marks.find((el) => itemOf(el) === focus.id)?.scrollIntoView({ block: "center", behavior: "smooth" });
  });

  useEffect(() => {
    if (!menu) return;
    const key = (e) => e.key === "Escape" && closeMenu();
    const outside = (e) => !e.target.closest(".review-menu") && closeMenu();
    document.addEventListener("keydown", key);
    document.addEventListener("mousedown", outside);
    return () => {
      document.removeEventListener("keydown", key);
      document.removeEventListener("mousedown", outside);
    };
  }, [menu]);

  if (!review) return html`<div class="preview-area"><div class="preview-error">${t("Loading preview…")}</div></div>`;

  const closeMenu = () => {
    setMenu(null);
    window.getSelection()?.removeAllRanges();
  };
  const correct = async (correction) => {
    closeMenu();
    setReview(await call("pii_correct", { source, correction }));
  };
  const toggle = (item) => correct(item.hidden ? { action: "show", entity: item.id } : { action: "hide", text: item.value });
  const remove = (item) => correct({ action: "forget", text: item.value });

  const click = (e) => {
    const el = e.target.closest(MARKS);
    const item = el && review.items.find((i) => i.id === itemOf(el));
    if (!item) return;
    setFocus({ id: item.id });
    const rect = el.getBoundingClientRect();
    setMenu({ item, left: rect.left, top: rect.bottom + 6 });
  };
  const select = () => {
    const span = selectionSpan(area.current);
    if (span) setMenu({ span, left: span.rect.left, top: span.rect.bottom + 6 });
  };
  const addTyped = (e) => {
    e.preventDefault();
    if (typed.trim()) correct({ action: "hide", text: typed.trim() }).then(() => setTyped(""));
  };

  const hiddenCount = review.items.filter((i) => i.hidden).length;
  return html`
    <div class="fill-split review">
      <div class="fill-rows">
        <div class="list-note muted small">${tn(hiddenCount, "{n} value hidden", "{n} values hidden")}</div>
        ${review.items.map((item) => html`
          <div key=${item.id} class=${`hidden-row ${item.hidden ? "" : "shown"} ${focus?.id === item.id ? "active" : ""}`}
            onClick=${() => setFocus({ id: item.id, scroll: true })}>
            <span class="chip kind">${item.kind}</span>
            <span class="value selectable" title=${item.value}>${item.value}</span>
            <span class="muted small">×${item.occurrences}</span>
            <button class="icon toggle" title=${item.hidden ? t("Hidden from the assistant. Click to show it.") : t("Visible to the assistant. Click to hide it.")}
              onClick=${(e) => (e.stopPropagation(), toggle(item))}><${Icon} name=${item.hidden ? "eye-off" : "eye"} /></button>
            ${item.user
              ? html`<button class="icon remove" title=${t("Remove")} onClick=${(e) => (e.stopPropagation(), remove(item))}><${Icon} name="trash" /></button>`
              : html`<span></span>`}
          </div>`)}
        <form class="hide-typed" onSubmit=${addTyped}>
          <input type="text" class="hide-text" value=${typed} spellcheck="false" placeholder=${t("Text to hide…")}
            onInput=${(e) => setTyped(e.target.value)} />
          <button type="submit">${t("Hide")}</button>
        </form>
      </div>
      <div class="fill-preview" ref=${area} onMouseUp=${select} onClick=${click}>
        <${Preview} preview=${review.preview} />
      </div>
    </div>
    ${menu && html`
      <div class="review-menu" style=${{ left: `${Math.max(8, menu.left)}px`, top: `${menu.top}px` }}>
        ${menu.span
          ? html`<button class="primary hide-selection" onClick=${() => correct({ action: "hide", block: menu.span.block, start: menu.span.start, end: menu.span.end })}>${t("Hide from the assistant")}</button>`
          : html`
              <div class="menu-title selectable">${menu.item.value}</div>
              <button class=${`primary ${menu.item.hidden ? "menu-show" : "menu-hide"}`} onClick=${() => toggle(menu.item)}>
                ${menu.item.hidden ? t("Show to the assistant") : t("Hide from the assistant")}
              </button>
              ${menu.item.user && html`<button class="menu-remove" onClick=${() => remove(menu.item)}>${t("Remove")}</button>`}`}
        <button class="menu-cancel" onClick=${closeMenu}>${t("Cancel")}</button>
      </div>`}`;
}
