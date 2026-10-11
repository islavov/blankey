// Loads the real UI (ui/index.html + ui/js) into happy-dom with a fake backend. One app per test file: node --test
// runs each file in its own process, and the UI modules keep state at module level.

import { readFileSync } from "node:fs";
import { GlobalRegistrator } from "@happy-dom/global-registrator";
import { createBackend, seed } from "./backend.js";

const UI = new URL("../", import.meta.url);

export async function startApp(changes = {}) {
  GlobalRegistrator.register({ url: "http://localhost/", width: 1200, height: 800 });
  const page = readFileSync(new URL("index.html", UI), "utf8");
  document.body.innerHTML = page.match(/<body>([\s\S]*)<\/body>/)[1].replace(/<script[\s\S]*?<\/script>/g, "");
  polyfill();

  const backend = createBackend(Object.assign(seed(), changes));
  window.__TAURI__ = backend.tauri;
  await import(new URL("js/main.js", UI).href);
  await settle();
  return backend;
}

// What happy-dom leaves out and the UI uses.
function polyfill() {
  HTMLImageElement.prototype.decode ??= () => Promise.resolve();
  Element.prototype.scrollIntoView ??= function () {};
  globalThis.CSS ??= {};
  CSS.escape ??= (value) => String(value).replace(/["\\]/g, "\\$&");
  document.hasFocus = () => true;
}

const tick = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Lets pending promises, renders, effects (run after the next frame) and timers up to `ms` run. */
export async function settle(ms = 0) {
  if (ms) await tick(ms);
  for (let i = 0; i < 4; i++) await tick(20);
}

export const $ = (selector, root = document) => root.querySelector(selector);
export const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
export const text = (selector, root) => $(selector, root)?.textContent.replace(/\s+/g, " ").trim() ?? null;

/** The element whose text matches, among `selector`. */
export function byText(selector, wanted, root = document) {
  const found = $$(selector, root).find((el) => el.textContent.replace(/\s+/g, " ").trim() === wanted);
  if (!found) throw new Error(`no ${selector} with text "${wanted}"`);
  return found;
}

export async function click(element, init = {}) {
  element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, ...init }));
  element.dispatchEvent(new MouseEvent("click", { bubbles: true, ...init }));
  await settle();
}

export async function dblclick(element) {
  element.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  await settle();
}

export async function type(input, value) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

export async function choose(select, value) {
  select.value = String(value);
  select.dispatchEvent(new Event("change", { bubbles: true }));
  await settle();
}

export async function check(input, on = true) {
  input.checked = on;
  input.dispatchEvent(new Event("change", { bubbles: true }));
  await settle();
}

export async function key(name, target = document.activeElement || document.body, init = {}) {
  target.dispatchEvent(new KeyboardEvent("keydown", { key: name, bubbles: true, ...init }));
  await settle();
}

export async function submit(form) {
  form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true }));
  await settle();
}

export const goTo = (title) => click($$(".nav-item").find((el) => el.textContent.trim().startsWith(title)));
export const action = (label) => click(byText("#page-actions button", label));
