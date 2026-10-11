import { html } from "../lib.js";

/** A selectable table. columns: [{title, cell(row) -> content, cls}]; cmd/ctrl-click adds rows when `multi`. */
export function Table({ rows, columns, selected = [], onSelect, onOpen, multi = false, rowClass = () => "" }) {
  const click = (e, id) => {
    if (multi && (e.metaKey || e.ctrlKey)) {
      onSelect(selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id]);
    } else onSelect([id]);
  };
  return html`
    <table class="grid">
      <thead><tr>${columns.map((c) => html`<th>${c.title}</th>`)}</tr></thead>
      <tbody>
        ${rows.map(
          (row) => html`
            <tr key=${row.id} class=${`${selected.includes(row.id) ? "selected" : ""} ${rowClass(row)}`}
              onClick=${(e) => click(e, row.id)} onDblClick=${() => onOpen?.(row.id)}>
              ${columns.map((c) => html`<td class=${c.cls || ""}>${c.cell(row)}</td>`)}
            </tr>`
        )}
      </tbody>
    </table>`;
}
