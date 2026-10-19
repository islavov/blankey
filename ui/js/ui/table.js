import { useRef } from "../../vendor/preact-htm.js";
import { html, useRemembered } from "../lib.js";

/**
 * A remembered selection over a list: a click selects one item, cmd/ctrl-click adds or removes one, shift-click
 * selects the range from the last clicked item (cmd/ctrl-shift adds the range). `pick(e, id, order, current)`
 * applies a click, where `order` is the ids as shown and `current` what the page treats as selected (default:
 * the remembered selection).
 */
export function useSelection(key) {
  const [selected, setSelected] = useRemembered(key, []);
  const anchor = useRef(null);
  const pick = (e, id, order, current = selected) => {
    const toggle = e.metaKey || e.ctrlKey;
    if (e.shiftKey && order.includes(anchor.current) && order.includes(id)) {
      const [from, to] = [order.indexOf(anchor.current), order.indexOf(id)];
      // ends on the clicked item, so `selected.at(-1)` is always the one clicked last
      const range = from <= to ? order.slice(from, to + 1) : order.slice(to, from + 1).reverse();
      setSelected(toggle ? [...current.filter((x) => !range.includes(x)), ...range] : range);
      return;
    }
    anchor.current = id;
    if (toggle) setSelected(current.includes(id) ? current.filter((x) => x !== id) : [...current, id]);
    else setSelected([id]);
  };
  return [selected, pick, setSelected];
}

/** Keeps shift-click from selecting the page's text. */
export const keepText = (e) => e.shiftKey && e.preventDefault();

/** A selectable table. columns: [{title, cell(row) -> content, cls}]; onPick(e, id, order) from useSelection. */
export function Table({ rows, columns, selected = [], onPick, onOpen, rowClass = () => "" }) {
  const order = rows.map((row) => row.id);
  return html`
    <table class="grid">
      <thead><tr>${columns.map((c) => html`<th>${c.title}</th>`)}</tr></thead>
      <tbody>
        ${rows.map(
          (row) => html`
            <tr key=${row.id} class=${`${selected.includes(row.id) ? "selected" : ""} ${rowClass(row)}`}
              onMouseDown=${keepText} onClick=${(e) => onPick?.(e, row.id, order)} onDblClick=${() => onOpen?.(row.id)}>
              ${columns.map((c) => html`<td class=${c.cls || ""}>${c.cell(row)}</td>`)}
            </tr>`
        )}
      </tbody>
    </table>`;
}
