import { useEffect, useRef } from "../../vendor/preact-htm.js";
import { t } from "../i18n.js";
import { html, Icon, useRemembered } from "../lib.js";
import { popup } from "./menu.js";

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

/**
 * A table with a checkbox in front of each row and a ⋮ menu at its end. checked: the checked ids; onCheck(ids or
 * current => ids); menu(row): the row's popup items; onOpen(row) on double-click. Pressing a row's checkbox, or the
 * row itself with cmd/ctrl, flips it, and holding the button down gives every row passed over the same state.
 * checks: false leaves the checkboxes out.
 */
export function CheckTable({ rows, columns, checked = [], onCheck, menu, onOpen, checks = true, rowClass = () => "" }) {
  // while the button is held after a press: the state the rows passed over get
  const paint = useRef(null);
  useEffect(() => {
    const stop = () => (paint.current = null);
    document.addEventListener("mouseup", stop);
    return () => document.removeEventListener("mouseup", stop);
  }, []);
  const ids = rows.map((row) => row.id);
  const all = ids.length > 0 && ids.every((id) => checked.includes(id));
  const set = (id, on) => onCheck((current) => (on ? (current.includes(id) ? current : [...current, id]) : current.filter((x) => x !== id)));
  const press = (e, row) => {
    e.preventDefault();
    paint.current = !checked.includes(row.id);
    set(row.id, paint.current);
  };
  const pass = (row) => paint.current !== null && set(row.id, paint.current);
  const pressRow = (e, row) => {
    if (checks && (e.metaKey || e.ctrlKey) && !e.target.closest("input, button, a, td.check")) press(e, row);
  };
  const open = (e, row) => {
    if (!e.target.closest("input, button")) onOpen?.(row);
  };
  const showMenu = (e, row) => {
    const rect = e.currentTarget.getBoundingClientRect();
    popup(rect.right - 200, rect.bottom + 2, menu(row).filter(Boolean), 200);
  };
  return html`
    <table class="grid check-grid">
      <thead>
        <tr>
          ${checks && html`<th class="check"><input type="checkbox" data-help="rows.all" checked=${all} onChange=${() => onCheck(all ? [] : ids)} /></th>`}
          ${columns.map((c) => html`<th>${c.title}</th>`)}
          <th class="row-menu"></th>
        </tr>
      </thead>
      <tbody>
        ${rows.map(
          (row) => html`
            <tr key=${row.id} class=${`${checked.includes(row.id) ? "selected" : ""} ${rowClass(row)}`}
              onMouseDown=${(e) => pressRow(e, row)} onMouseEnter=${() => pass(row)} onDblClick=${(e) => open(e, row)}>
              ${checks &&
              html`<td class="check" data-help="rows.check" onMouseDown=${(e) => press(e, row)}>
                <input type="checkbox" checked=${checked.includes(row.id)} onChange=${(e) => set(row.id, e.target.checked)} />
              </td>`}
              ${columns.map((c) => html`<td class=${c.cls || ""}>${c.cell(row)}</td>`)}
              <td class="row-menu">
                <button class="ghost more" data-help="rows.menu" title=${t("More actions")} aria-label=${t("More actions")}
                  onClick=${(e) => showMenu(e, row)}><${Icon} name="more" /></button>
              </td>
            </tr>`
        )}
      </tbody>
    </table>`;
}
