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
  return { block: start.block, start: start.offset, end: end.offset, range: range.cloneRange() };
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

/** Under its text, wherever the preview has scrolled; hidden while the text is scrolled out of the preview. */
function menuPlace(anchor, root) {
  const rect = anchor.getBoundingClientRect();
  const view = root?.querySelector(".preview-area")?.getBoundingClientRect();
  const out = view && (rect.bottom < view.top || rect.top > view.bottom);
  return { left: `${Math.max(8, rect.left)}px`, top: `${rect.bottom + 6}px`, visibility: out ? "hidden" : "visible" };
}

/**
 * The places of a row in the preview, in document order: the elements of each place of any of its ids (a docx
 * place split over runs is several marks with one data-n; a PDF box is one place).
 */
function placesOf(root, ids) {
  const places = new Map();
  [...(root?.querySelectorAll(MARKS) || [])]
    .filter((el) => ids.includes(itemOf(el)))
    .forEach((el, i) => {
      const n = `${itemOf(el)}#${el.dataset.n ?? i}`;
      places.set(n, [...(places.get(n) || []), el]);
    });
  return [...places.values()];
}

/**
 * What the assistant does not see in a file: the preview with marked values and their list, one row per value in
 * document order. Selecting text hides it everywhere in the file; a value can be shown to the assistant, hidden
 * again, or removed when the user added it, and its places stepped through. Corrections are kept for the file.
 */
export function PiiReview({ source }) {
  const [review, setReview] = useState(null);
  const [menu, setMenu] = useState(null);
  // the selected row by its value (ids change when a value is shown or hidden) and its current place
  const [focus, setFocus] = useState(null);
  const [typed, setTyped] = useState("");
  const [, setScrolled] = useState(0);
  const area = useRef();
  const scrolled = useRef(0);

  useEffect(() => {
    setReview(null);
    call("pii_review", { source }).then(setReview).catch(() => {});
  }, [source]);

  // how many places the selected value has in the preview, measured after each render (the preview changes when
  // a value is shown or hidden)
  const [count, setCount] = useState(0);
  const focused = review?.items.find((i) => i.key === focus?.key);
  const current = count ? Math.min(focus.place, count - 1) : 0;

  // the selected value's places are outlined, its current place stands out and comes into view when stepped to
  useEffect(() => {
    area.current?.querySelectorAll(MARKS).forEach((el) => el.classList.remove("focus", "current"));
    const all = focused ? placesOf(area.current, focused.ids) : [];
    if (all.length !== count) setCount(all.length);
    if (!focused) return;
    all.flat().forEach((el) => el.classList.add("focus"));
    const at = all[Math.min(focus.place, all.length - 1)] || [];
    at.forEach((el) => el.classList.add("current"));
    if (focus.seq !== scrolled.current) {
      scrolled.current = focus.seq;
      at[0]?.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  });

  // the menu follows its text when the preview scrolls; Escape or a click elsewhere closes it
  useEffect(() => {
    if (!menu) return;
    const key = (e) => e.key === "Escape" && closeMenu();
    const outside = (e) => !e.target.closest(".review-menu") && closeMenu();
    const scroll = () => setScrolled((n) => n + 1);
    document.addEventListener("keydown", key);
    document.addEventListener("mousedown", outside);
    document.addEventListener("scroll", scroll, true);
    return () => {
      document.removeEventListener("keydown", key);
      document.removeEventListener("mousedown", outside);
      document.removeEventListener("scroll", scroll, true);
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
  // a row acts on all its members: showing shows every hidden one, hiding hides every spelling
  const toggle = (row) =>
    correct(
      row.hidden
        ? { action: "show", entities: row.members.filter((m) => m.hidden).map((m) => m.id) }
        : { action: "hide", texts: row.members.map((m) => m.value) }
    );
  // off the list: what the user added is forgotten, a detected value counts as not personal and stays visible
  const remove = (row) => correct({ action: "remove", texts: row.members.map((m) => m.value) });
  const rowOf = (id) => review.items.find((row) => row.ids.includes(id));

  const go = (item, place, scroll = true) =>
    setFocus({ key: item.key, place, seq: scroll ? (focus?.seq || 0) + 1 : focus?.seq || 0 });
  const step = (e, delta) => {
    e.stopPropagation();
    go(focused, (current + delta + count) % count);
  };

  const click = (e) => {
    const el = e.target.closest(MARKS);
    const item = el && rowOf(itemOf(el));
    if (!item) return;
    const at = placesOf(area.current, item.ids).findIndex((place) => place.includes(el));
    go(item, Math.max(0, at), false);
    setMenu({ item, anchor: el });
  };
  const select = () => {
    const span = selectionSpan(area.current);
    if (span) setMenu({ span, anchor: span.range });
  };
  const addTyped = (e) => {
    e.preventDefault();
    if (typed.trim()) correct({ action: "hide", texts: [typed.trim()] }).then(() => setTyped(""));
  };

  const hiddenCount = review.items.filter((i) => i.hidden).length;
  return html`
    <div class="fill-split review">
      <div class="fill-rows">
        <div class="list-note muted small">${tn(hiddenCount, "{n} value hidden", "{n} values hidden")}</div>
        ${review.items.map((item) => {
          const active = item.key === focused?.key;
          return html`
          <div key=${item.key} class=${`hidden-row ${item.hidden ? "" : "shown"} ${active ? "active" : ""}`}
            onClick=${() => go(item, active ? current : 0)}>
            <span class="value selectable" title=${item.value}>${item.value}</span>
            <span class="places">
              ${active && count > 1
                ? html`
                    <button class="icon prev" title=${t("Previous place")} onClick=${(e) => step(e, -1)}>‹</button>
                    <span class="small at">${current + 1}/${count}</span>
                    <button class="icon next" title=${t("Next place")} onClick=${(e) => step(e, 1)}>›</button>`
                : html`<span class="muted small">×${item.occurrences}</span>`}
            </span>
            <button class="icon toggle" title=${item.hidden ? t("Hidden from the assistant. Click to show it.") : t("Visible to the assistant. Click to hide it.")}
              onClick=${(e) => (e.stopPropagation(), toggle(item))}><${Icon} name=${item.hidden ? "eye-off" : "eye"} /></button>
            <button class="icon remove" title=${item.user ? t("Remove") : t("Not personal data. The assistant sees it and the list no longer shows it.")}
              onClick=${(e) => (e.stopPropagation(), remove(item))}><${Icon} name="trash" /></button>
          </div>`;
        })}
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
      <div class="review-menu" style=${menuPlace(menu.anchor, area.current)}>
        ${menu.span
          ? html`<button class="primary hide-selection" onClick=${() => correct({ action: "hide", block: menu.span.block, start: menu.span.start, end: menu.span.end })}>${t("Hide from the assistant")}</button>`
          : html`
              <div class="menu-title selectable">${menu.item.value}</div>
              <button class=${`primary ${menu.item.hidden ? "menu-show" : "menu-hide"}`} onClick=${() => toggle(menu.item)}>
                ${menu.item.hidden ? t("Show to the assistant") : t("Hide from the assistant")}
              </button>
              <button class="menu-remove" onClick=${() => remove(menu.item)}>${menu.item.user ? t("Remove") : t("Not personal data")}</button>`}
        <button class="menu-cancel" onClick=${closeMenu}>${t("Cancel")}</button>
      </div>`}`;
}
