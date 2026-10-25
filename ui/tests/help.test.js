import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, action, byText, check, click, goTo, hover, settle, startApp, text } from "./setup.js";

// longer than the wait before falling back from a control to the pane around it
const FALLBACK = 200;

let helpText;
before(async () => {
  await startApp();
  ({ helpText } = await import(new URL("../js/help.js", import.meta.url).href));
});

const shown = () => [text(".help-title"), text(".help-text")];

test("the help box sits above the sidebar buttons and starts with what the page is for", () => {
  const order = [...$(".sidebar").children].map((el) => el.id || el.className);
  assert.deepEqual(order.slice(-3), ["nav", "help-box", "sidebar-foot"]);
  assert.deepEqual(shown(), helpText("page.activity"));
});

test("hovering a nav item, a page action or a sidebar button shows what it does", async () => {
  await hover($$(".nav-item").find((el) => el.textContent.trim() === "Fill sets"));
  assert.deepEqual(shown(), helpText("nav.fillsets"));
  await hover($$(".nav-item").find((el) => el.textContent.trim() === "Collection links"));
  assert.ok(text(".help-text").includes("only be opened on this computer"), "a nav item explains the idea");

  await hover(byText(".nav-section", "Library"), FALLBACK);
  assert.deepEqual(shown(), helpText("page.activity"), "section titles have no help of their own");

  await goTo("Fill sets");
  await hover(byText("#page-actions button", "Generate"));
  assert.deepEqual(shown(), helpText("fillsets.generate"));

  await hover($("#lock-btn"));
  assert.equal(text(".help-title"), "Lock");
});

test("leaving a control for its pane waits a moment, then shows the pane", async () => {
  await hover(byText("#page-actions button", "Delete"));
  await hover($(".page"));
  assert.deepEqual(shown(), helpText("fillsets.delete"), "a gap between controls keeps the help");
  await settle(FALLBACK);
  assert.deepEqual(shown(), helpText("page.fillsets"));

  await hover($("#search"));
  document.body.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: null }));
  await settle(FALLBACK);
  assert.deepEqual(shown(), helpText("page.fillsets"), "the pointer left the window");
});

test("a button without a help key shows its own title", async () => {
  const button = document.createElement("button");
  button.title = "Do something";
  $(".page").append(button);
  await hover(button);
  assert.deepEqual(shown(), ["Do something", ""]);
  button.remove();
});

test("every help key on the pages and in Settings has a text", async () => {
  const keys = new Set();
  const collect = () => $$("[data-help]").forEach((el) => keys.add(el.getAttribute("data-help")));
  for (const page of ["Activity", "Fill sets", "Documents", "Templates", "Profiles", "Collection links"]) {
    await goTo(page);
    collect();
  }
  await goTo("Templates");
  await click(byText(".filters button", "Folders"));
  await settle();
  collect();
  await click(byText(".filters button", "Templates"));
  await click($("#settings-btn"));
  collect();
  assert.ok(keys.size > 40, `found ${keys.size} keys`);
  assert.deepEqual([...keys].filter((key) => !helpText(key)), []);
});

test("in a sheet the footer note shows what is hovered, and its own note again after", async () => {
  await goTo("Templates");
  await action("Fill");
  await settle(300);
  const note = text(".sheet-foot .note");
  assert.ok(note.startsWith("Your data stays in Blankey."));

  const keys = $$(".sheet [data-help]").map((el) => el.getAttribute("data-help"));
  assert.ok(keys.includes("fill.source"));
  assert.deepEqual(keys.filter((key) => !helpText(key)), []);

  await hover(byText(".sheet-foot button", "Save and Generate"));
  const [title, help] = helpText("fill.generate");
  assert.equal(text(".sheet-foot .note"), `${title}: ${help}`);
  assert.equal(text(".help-title"), "Templates", "the sidebar box, under the sheet, keeps the page's help");

  await hover($(".sheet-head h2"), FALLBACK);
  assert.equal(text(".sheet-foot .note"), note);
  await click(byText(".sheet-foot button", "Cancel"));
  assert.equal($(".sheet"), null);
});

test("closing the box turns help off until it is turned on in Settings", async () => {
  await click($(".help-close"));
  assert.equal($(".help-box"), null);
  assert.equal(text("#toast"), "Help is off. Turn it on again in Settings.");
  assert.equal(localStorage.getItem("blankey.help"), "off");

  await click($("#settings-btn"));
  assert.equal($("#help-box").checked, false);
  await check($("#help-box"), true);
  assert.ok($(".help-box"));
  assert.equal(localStorage.getItem("blankey.help"), "on");
  assert.deepEqual(shown(), helpText("page.settings"));

  await check($("#help-box"), false);
  assert.equal($(".help-box"), null);
});
