import { useLayoutEffect, useRef } from "../../vendor/preact-htm.js";
import { html } from "../lib.js";

/**
 * A <select> showing `value`. options: [[value, label]]. The value is set after the options exist: this Preact
 * build applies a select's value before its children, which leaves it blank.
 */
export function Select({ value, options, onChange, ...rest }) {
  const select = useRef();
  useLayoutEffect(() => {
    select.current.value = String(value ?? "");
  });
  return html`
    <select ref=${select} ...${rest} onChange=${(e) => onChange(e.target.value)}>
      ${options.map(([optionValue, label]) => html`<option value=${optionValue}>${label}</option>`)}
    </select>`;
}
