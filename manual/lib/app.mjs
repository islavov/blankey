// What a scene drives the UI with: clicks and typing by selector or by the English text of a control, changes to the
// fake backend, and screenshots.

import { mkdir } from "node:fs/promises";
import { join } from "node:path";

const VIEWPORT = { width: 1180, height: 760 };
const NOW = new Date("2026-11-03T10:00:00");
// front to back: a text target is looked for in the frontmost layer that has anything in it
const LAYERS = ["#menu-root", "#modal-root", "#sheet-root", "#root"];
const CLICKABLE = "button, a, label, summary, .chip, .tab, .menu-item, .nav-item, .ws-option, .list-item";

const escaped = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

export class App {
  constructor(page, { origin, language, shotsDir, scene }) {
    this.page = page;
    this.origin = origin;
    this.language = language;
    this.shotsDir = shotsDir;
    this.scene = scene;
    this.shots = [];
    this.step = "start";
  }

  /** A page for one scene in one language, with the app's window size, light theme and a fixed date. */
  static async open(browser, options) {
    const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: 2, colorScheme: "light", reducedMotion: "reduce", locale: options.language });
    const page = await context.newPage();
    await page.clock.setFixedTime(NOW);
    const app = new App(page, options);
    page.on("pageerror", (error) => (app.pageError ??= error));
    await page.goto(`${options.origin}/manual/app.html?lang=${options.language}&start=${options.scene.start || "cases"}`);
    await page.locator("#root > *").first().waitFor();
    return app;
  }

  /** The text as the UI shows it in this run's language. */
  t(text, vars) {
    return this.page.evaluate(([text, vars]) => window.t(text, vars), [text, vars]);
  }

  /** The element for a target: a CSS selector, a locator, or `{ text, within }` for a control with that English text. */
  async find(target) {
    if (typeof target === "string") return this.page.locator(target).first();
    if (!target.text) return target.first();
    const text = new RegExp(`^\\s*${escaped(await this.t(target.text, target.vars))}\\s*$`);
    const scopes = target.within ? [target.within] : LAYERS;
    for (const scope of scopes) {
      const found = this.page.locator(scope).locator(CLICKABLE).filter({ hasText: text, visible: true });
      if (await found.count()) return found.first();
    }
    // a control whose text is one part of it, as a list option with an icon
    for (const scope of scopes) {
      const found = this.page.locator(scope).getByText(text).filter({ visible: true });
      if (await found.count()) return found.first();
    }
    throw new Error(`no control "${target.text}"`);
  }

  /** Clicks the target (see `find`). */
  async click(target) {
    await (await this.find(target)).click();
    await this.settle();
  }

  /** Double-clicks the target. */
  async dblclick(target) {
    await (await this.find(target)).dblclick();
    await this.settle();
  }

  /** Replaces the text of an input. */
  async fill(target, value) {
    await (await this.find(target)).fill(value);
    await this.settle();
  }

  /** Ticks a checkbox. */
  async check(target) {
    await (await this.find(target)).check();
    await this.settle();
  }

  /** Presses a key, such as Escape to close what is open. */
  async press(key) {
    await this.page.keyboard.press(key);
    await this.settle();
  }

  /** Scrolls the element into view. */
  async show(target) {
    await (await this.find(target)).scrollIntoViewIfNeeded();
    await this.settle();
  }

  /** Opens a page from the sidebar by its English title. */
  goTo(title) {
    return this.click({ text: title, within: "#nav" });
  }

  /** Opens Settings with the cog at the bottom of the sidebar. */
  settings() {
    return this.click("#settings-btn");
  }

  /** Presses a button of the page's toolbar. */
  action(label) {
    return this.click({ text: label, within: "#page-actions" });
  }

  /** Opens a tab of the open case; its chip may carry a count after the label. */
  async tab(label) {
    const text = new RegExp(`^\\s*${escaped(await this.t(label))}`);
    await this.click(this.page.locator(".case-tabs .chip").filter({ hasText: text }));
  }

  /** The table row that has this text in it. */
  row(text) {
    return this.page.locator("table.grid tbody tr").filter({ hasText: text }).first();
  }

  /** Waits until the element shows; with `gone`, until it no longer does. */
  async wait(target, { gone } = {}) {
    await (await this.find(target)).waitFor({ state: gone ? "hidden" : "visible" });
    await this.settle();
  }

  /** Opens a table row's menu and picks an item; without `item` the menu stays open. */
  async menu(row, item) {
    await (await this.find(row)).locator("button.more").click();
    await this.settle();
    if (item) await this.click({ text: item, within: "#menu-root" });
  }

  /** Runs `fn(backend, fixtures, arg)` in the page: changes the fake vault or emits a backend event. */
  async backend(fn, arg) {
    const result = await this.page.evaluate(`(${fn})(window.backend, window.fixtures, ${JSON.stringify(arg ?? null)})`);
    await this.settle();
    return result;
  }

  /** Waits until the UI has rendered what the last step caused, images included. */
  async settle() {
    if (this.pageError) throw new Error(`the UI threw: ${this.pageError.message}`);
    await this.page.evaluate(async () => {
      for (let i = 0; i < 3; i++) await new Promise((done) => setTimeout(done, 40));
      await Promise.all([...document.images].map((image) => image.decode().catch(() => {})));
    });
  }

  /**
   * Saves a screenshot as `<scene>-<name>`. `target` crops to an element, `mark` outlines one (both targets as in
   * `find`), `toast` keeps a message that is showing, `keepPointer` leaves the pointer where it is.
   */
  async shot(name, { target, mark, keepPointer, toast } = {}) {
    this.step = name;
    await this.page.evaluate((keepToast) => {
      if (!keepToast) document.getElementById("toast").classList.add("hidden");
      // a click on a control past the right edge scrolls its container sideways, which no user of the window can do
      for (const el of document.querySelectorAll("#root *")) if (el.scrollLeft) el.scrollLeft = 0;
    }, Boolean(toast));
    if (!keepPointer) await this.page.mouse.move(VIEWPORT.width - 2, 2);
    await this.settle();
    const marked = mark && (await this.find(mark));
    if (marked) await marked.evaluate((el) => ((el.dataset.manualOutline = el.style.outline), (el.style.outline = "3px solid #e5484d"), (el.style.outlineOffset = "3px")));
    const id = `${this.scene.id}-${name}`;
    const path = join(this.shotsDir, `${id}.png`);
    await mkdir(this.shotsDir, { recursive: true });
    const subject = target ? await this.find(target) : this.page;
    await subject.screenshot({ path, animations: "disabled" });
    if (marked) await marked.evaluate((el) => ((el.style.outline = el.dataset.manualOutline), (el.style.outlineOffset = ""), delete el.dataset.manualOutline));
    this.shots.push(id);
  }

  /** Closes the scene's page. */
  close() {
    return this.page.context().close();
  }
}
