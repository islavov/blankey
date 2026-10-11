import { html, useEffect, useState } from "../vendor/preact-htm.js";

export { html };

export const when = (ts) => (ts ? String(ts).replace("T", " ").slice(0, 16) : "");
export const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

export const KIND_NAMES = { docx: "Word", pdf_form: "PDF", typst: "Typst" };

export const PLACEHOLDERS = {
  date: "DD.MM.YYYY",
  egn: "10 digits",
  iban: "BG00 XXXX 0000 0000 0000 00",
  number: "1 500,00",
  email: "name@example.com",
};

const ICONS = {
  activity: '<path d="M8 2.2a5.8 5.8 0 1 1-5.6 4.3" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M1.6 3.4l.9 3.2 3.1-1" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 4.8V8l2.2 1.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
  fillsets: '<rect x="2.5" y="2.5" width="11" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5 6h6M5 8.5h6M5 11h3.5" stroke="currentColor" stroke-width="1.4"/>',
  documents: '<path d="M4 1.8h5l3 3v9.4H4z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M9 1.8v3h3" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  templates: '<rect x="2" y="3.5" width="8.5" height="10.5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5.5 3.5V2h8.5v10.5h-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  profiles: '<circle cx="8" cy="5.5" r="2.8" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M2.8 14c.6-2.8 2.7-4.2 5.2-4.2s4.6 1.4 5.2 4.2" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  person: '<circle cx="8" cy="5.2" r="3" fill="currentColor"/><path d="M2.5 14.5c.6-3 2.8-4.6 5.5-4.6s4.9 1.6 5.5 4.6z" fill="currentColor"/>',
  doc: '<path d="M3.5 1.5h5.5l3.5 3.5v9.5h-9z" fill="currentColor"/><path d="M5.5 8h5M5.5 10.5h5" stroke="var(--bg)" stroke-width="1.2"/>',
  sun: '<circle cx="8" cy="8" r="3" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M8 1.5v1.6M8 12.9v1.6M1.5 8h1.6M12.9 8h1.6M3.4 3.4l1.1 1.1M11.5 11.5l1.1 1.1M3.4 12.6l1.1-1.1M11.5 4.5l1.1-1.1" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
  moon: '<path d="M13.2 9.9A5.6 5.6 0 0 1 6.1 2.8a5.6 5.6 0 1 0 7.1 7.1z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  lock: '<rect x="3.5" y="7" width="9" height="7" rx="1.5" fill="currentColor"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="currentColor" stroke-width="1.4"/>',
};

export const Icon = ({ name }) => html`<svg viewBox="0 0 16 16" dangerouslySetInnerHTML=${{ __html: ICONS[name] }}></svg>`;

// -- app-wide state ------------------------------------------------------------------------
// Only what crosses components: the rest lives in component state.

export const state = { status: null, page: "activity", search: "", gateReason: "", recoveryKey: null, revision: 0 };
const listeners = new Set();
let version = 0;

export function update(patch) {
  Object.assign(state, patch);
  version++;
  listeners.forEach((listener) => listener());
}

/** Re-renders the component whenever the app-wide state changes. */
export function useStore() {
  const [, force] = useState(0);
  const seen = version;
  useEffect(() => {
    const listener = () => force((n) => n + 1);
    listeners.add(listener);
    // effects run after paint: catch up on changes made since this render
    if (version !== seen) listener();
    return () => listeners.delete(listener);
  }, []);
  return state;
}

/** Reloads the data of the current page. */
export const reload = () => update({ revision: state.revision + 1 });

export function matches(...values) {
  const q = state.search.trim().toLowerCase();
  return !q || values.some((v) => String(v ?? "").toLowerCase().includes(q));
}

/** Result of `load()`, re-run when `deps` change or the page reloads; a late answer for old deps is dropped. */
export function useLoad(load, deps) {
  const [data, setData] = useState(null);
  const { revision } = useStore();
  useEffect(() => {
    let live = true;
    load()
      .then((result) => live && setData(result))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [...deps, revision]);
  return data;
}

const remembered = {};

/** useState that keeps its value while the page is not shown. */
export function useRemembered(key, initial) {
  const [value, setValue] = useState(key in remembered ? remembered[key] : initial);
  const set = (next) => {
    remembered[key] = next;
    setValue(next);
  };
  return [value, set];
}
