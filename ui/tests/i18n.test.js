import assert from "node:assert/strict";
import { before, test } from "node:test";
import { profileInputRequest } from "./backend.js";
import { $, $$, byText, click, goTo, settle, startApp, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({ language: "bg", requests: [profileInputRequest(8)] });
});

test("the account's language: the sheet, its problems and the page are in Bulgarian", async () => {
  assert.equal(document.documentElement.lang, "bg");
  assert.equal(text(".sheet h2"), "2 полета за John");
  await type($("input[name=f0]"), "");
  await click(byText(".sheet-foot button", "Запази"));
  assert.equal(text(".problems"), "Email: задължително");
  await click(byText(".sheet-foot button", "Отказ"));

  assert.deepEqual(
    $$(".nav-section").map((el) => el.textContent),
    ["Асистент", "Библиотека", "Трезор"]
  );
  assert.equal(text(".toolbar h2"), "Активност");
  assert.equal(text("#lock-btn"), "Заключи");
  await goTo("Шаблони");
  assert.equal(text(".toolbar h2"), "Шаблони");
  assert.equal(text(".toolbar .muted"), "2 шаблона");
  assert.equal($("#search").placeholder, "Търсене");
});

test("a language change from the relay switches the open window", async () => {
  backend.db.language = "en";
  backend.emit("language-changed", "en");
  await settle();
  assert.equal(document.documentElement.lang, "en");
  assert.equal(text(".toolbar h2"), "Templates");
  assert.equal(text("#lock-btn"), "Lock");

  backend.emit("language-changed", "bg");
  await settle();
  assert.equal(text(".toolbar h2"), "Шаблони");
});

test("focusing the window asks the relay for the account's language", async () => {
  window.dispatchEvent(new Event("focus"));
  await settle();
  assert.ok(backend.called("focused").length > 0);
});
