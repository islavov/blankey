import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, click, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp();
  await goTo("Templates");
  await click(byText(".filters .chip", "Folders"));
  await settle();
});

/** Selects `needle` inside the text node it sits in and lets go of the mouse. */
async function selectText(needle) {
  const texts = (el) => [...el.childNodes].flatMap((n) => (n.nodeType === Node.TEXT_NODE ? [n] : texts(n)));
  const node = texts($(".sheet .paper")).find((n) => n.textContent.includes(needle));
  const start = node.textContent.indexOf(needle);
  const range = document.createRange();
  range.setStart(node, start);
  range.setEnd(node, start + needle.length);
  window.getSelection().removeAllRanges();
  window.getSelection().addRange(range);
  $(".sheet .fill-preview").dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  await settle();
}

const row = (value) => $$(".hidden-row").find((r) => $(".value", r).textContent === value);
const corrected = () => backend.called("pii_correct").at(-1)?.correction;

test("the user reviews what the assistant will not see in a library file", async () => {
  await click($(".detail .check-pii"));
  assert.equal(text(".sheet h2"), "Personal data in Lease agreement.docx");
  assert.deepEqual(backend.called("pii_review").at(-1), { source: "lib:4" });
  assert.ok(row("Jane Example") && !row("Jane Example").classList.contains("shown"));

  // a selection offers to hide it; Cancel closes the menu and changes nothing
  await selectText("77-ABC");
  assert.ok($(".review-menu .hide-selection"));
  await click($(".review-menu .menu-cancel"));
  assert.equal($(".review-menu"), null);
  assert.equal(backend.called("pii_correct").length, 0);

  // hidden as a span of the block in UTF-16 offsets
  await selectText("77-ABC");
  await click($(".review-menu .hide-selection"));
  const start = "Tenant Jane Example, card ".length;
  assert.deepEqual(corrected(), { action: "hide", block: 0, start, end: start + 6 });
  assert.ok(row("77-ABC").querySelector(".remove"), "an added value can be removed");
  assert.equal(row("Jane Example").querySelector(".remove"), null, "a detected one cannot");

  // a row highlights its places in the preview
  await click(row("77-ABC"));
  assert.ok($(".sheet mark[data-e='marked:2']").classList.contains("focus"));
  assert.ok(!$(".sheet mark[data-e='person:1']").classList.contains("focus"));

  // clicking a mark asks what to do instead of acting
  await click($(".sheet mark[data-e='person:1']"));
  assert.equal(backend.called("pii_correct").length, 1);
  assert.equal(text(".review-menu .menu-title"), "Jane Example");
  await click($(".review-menu .menu-show"));
  assert.deepEqual(corrected(), { action: "show", entity: "person:1" });
  assert.ok(row("Jane Example").classList.contains("shown"));
  assert.ok($(".sheet mark.shown[data-v='0']"), "a value shown to the assistant stays marked");

  // the eye toggles it back
  await click(row("Jane Example").querySelector(".toggle"));
  assert.deepEqual(corrected(), { action: "hide", text: "Jane Example" });
  assert.ok(!row("Jane Example").classList.contains("shown"));

  await click(row("77-ABC").querySelector(".remove"));
  assert.deepEqual(corrected(), { action: "forget", text: "77-ABC" });
  assert.equal(row("77-ABC"), undefined);

  await type($(".hide-text"), "30 days");
  await submit($(".hide-typed"));
  assert.deepEqual(corrected(), { action: "hide", text: "30 days" });

  await click(byText(".sheet-foot button", "Done"));
  assert.equal($(".sheet"), null);
});
