// UI languages: catalogs keyed by the English text (ui/i18n/<code>.json, shared with the tray), loaded through the
// backend. English needs no catalog; a missing entry shows the English text.

let current = "en";
let catalog = {};
let rules = new Intl.PluralRules("en");

const fill = (text, vars = {}) => text.replace(/\{(\w+)\}/g, (match, name) => (name in vars ? String(vars[name]) : match));

/** The translation of `text`, with `{name}` placeholders filled from `vars`. */
export function t(text, vars) {
  const entry = catalog[text];
  return fill(typeof entry === "string" && entry ? entry : text, vars);
}

/** The singular or plural form for `n`, keyed by the English singular; `{n}` is filled in. */
export function tn(n, one, other, vars = {}) {
  const entry = catalog[one];
  const form = rules.select(n) === "one" ? "one" : "other";
  const text = entry && typeof entry === "object" ? entry[form] : n === 1 ? one : other;
  return fill(text, { n, ...vars });
}

export const language = () => current;

/** Number formatting for the current language (decimal separator). */
export const number = (value, digits = 0) =>
  new Intl.NumberFormat(current, { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(value);

/** Switches the catalog; `load(code)` returns the catalog object. Callers re-render afterwards. */
export async function setLanguage(code, load) {
  catalog = code === "en" ? {} : await load(code);
  current = code;
  rules = new Intl.PluralRules(code);
  document.documentElement.lang = code;
}

/** A validation problem from the backend: the field's own label and an English message from the catalog. */
export const problemText = ({ label, message }) => (label ? `${label}: ${t(message)}` : t(message));
