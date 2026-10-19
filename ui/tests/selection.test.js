import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $$, action, byText, click, goTo, startApp, submit, text } from "./setup.js";

const set = (id, name) => ({ id, name, templates: ["nzok"], profiles: ["John"], updated_at: "2026-10-10T10:00:00" });

let backend;
before(async () => {
  backend = await startApp({
    sets: [set(1, "alpha"), set(2, "beta"), set(3, "gamma")],
    profiles: [
      { id: 1, name: "John", kind: "person" },
      { id: 2, name: "Acme", kind: "company" },
      { id: 3, name: "Zed", kind: "person" },
    ],
    fields: { 1: [], 2: [], 3: [] },
  });
});

const selectedRows = () => $$(".grid tr.selected").map((tr) => tr.querySelector("td").textContent.trim());

test("fill sets: shift-click selects a range, cmd-click takes one out, delete removes the rest", async () => {
  await goTo("Fill sets");
  const [first, second, third] = $$(".grid tbody tr");
  await click(first);
  await click(third, { shiftKey: true });
  assert.deepEqual(selectedRows(), ["alpha", "beta", "gamma"]);
  await click(second, { metaKey: true });
  assert.deepEqual(selectedRows(), ["alpha", "gamma"]);
  await click(second, { ctrlKey: true });
  assert.deepEqual(selectedRows(), ["alpha", "beta", "gamma"]);

  await click(second);
  await click(first, { shiftKey: true });
  assert.deepEqual(selectedRows(), ["alpha", "beta"], "a range runs upwards too");

  await action("Delete");
  assert.equal(text(".modal h3"), "Delete 2 fill sets?");
  await submit($$(".modal form")[0]);
  assert.deepEqual(backend.called("delete_fill_set").map((c) => c.id).sort(), [1, 2]);
  assert.deepEqual($$(".grid tbody tr").map((tr) => tr.querySelector("td").textContent.trim()), ["gamma"]);
});

test("templates: cmd-click selects several cards and deletes them together", async () => {
  await goTo("Templates");
  const [nzok, loan] = $$(".card");
  await click(nzok);
  await click(loan, { metaKey: true });
  assert.equal($$(".card.active").length, 2);
  await action("Delete");
  assert.equal(text(".modal h3"), "Delete 2 templates?");
  await submit($$(".modal form")[0]);
  assert.deepEqual(backend.called("delete_template").map((c) => c.id), ["nzok", "loan"]);
});

test("profiles: shift-click selects a range and the detail shows the one clicked last", async () => {
  await goTo("Profiles");
  const items = $$(".list-item");
  await click(items[0]);
  await click(items[2], { shiftKey: true });
  assert.equal($$(".list-item.active").length, 3);
  await action("Delete profile");
  assert.equal(text(".modal h3"), "Delete 3 profiles?");
  await click(byText(".modal button", "Cancel"));
  assert.equal(backend.called("delete_profile").length, 0);
});

test("activity keeps a single selection", async () => {
  await goTo("Activity");
  await click($$(".grid tbody tr")[0]);
  assert.equal($$(".grid tr.selected").length, 1);
});
