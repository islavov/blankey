import assert from "node:assert/strict";
import { before, test } from "node:test";
import { fillRequest } from "./backend.js";
import { $, byText, click, key, settle, startApp, text, type } from "./setup.js";

const PREVIEW_DELAY = 250;

let backend;
before(async () => {
  const request = fillRequest(7);
  request.payload.ask = { egn: "EGN from the ID card", amount: "Loan amount" };
  backend = await startApp({ requests: [request] });
  await settle(PREVIEW_DELAY);
});

const row = (name) => $(`.fill-row[data-var="${name}"]`);
const lastBindings = () => backend.called("fill_evaluate").at(-1).args.bindings;

test("the first asked blank opens ready to type, with the assistant's hint", async () => {
  const input = $(".source input", row("egn"));
  assert.equal(document.activeElement, input);
  assert.equal(input.placeholder, "EGN from the ID card");
  assert.equal(text(".chip", row("amount")), "To type");
  assert.equal(text(".val", row("amount")), "Loan amount");
});

test("Enter keeps the value in the fill set and moves to the next asked blank", async () => {
  const input = $(".source input", row("egn"));
  await type(input, "8001010000");
  await key("Enter", input);
  assert.deepEqual(lastBindings().egn, { value: "8001010000" });
  assert.equal(text(".chip", row("egn")), "Value");
  assert.equal(document.activeElement, $(".source input", row("amount")));
  assert.ok(row("amount").classList.contains("active"));
  await key("Escape", document.activeElement);
});

test("a blank can read a field of any profile, not only the role's", async () => {
  await click($(".source", row("name")));
  await click(byText(".menu-item", "From another profile…"));
  await click(byText(".menu-item", "Acme (company)"));
  await settle();
  await click(byText(".menu-item", "name — profile 2 name"));
  assert.deepEqual(lastBindings().name, { path: "name", profile: 2 });
  assert.equal(text(".chip", row("name")), "Vault");
  assert.equal(text(".path", row("name")), "Acme › name");

  await click($(".source", row("who")));
  assert.ok(byText(".menu-head", "Acme"), "a profile already in use is offered directly");
  await key("Escape", document.body);
});
