import assert from "node:assert/strict";
import { test } from "node:test";
import { $, check, startApp, submit, text, type } from "./setup.js";

test("setup: passwords must match, the recovery key must be confirmed", async () => {
  const backend = await startApp({ initialized: false, unlocked: false });
  assert.equal(text(".gate h1"), "Create your vault");
  const form = $(".gate form");
  await type($("input[name=password]", form), "secret-pw");
  await type($("input[name=confirm]", form), "other");
  await submit(form);
  assert.equal(text(".gate .error"), "The passwords do not match.");

  await type($("input[name=confirm]", form), "secret-pw");
  await submit(form);
  assert.equal(text(".gate h1"), "Save your recovery key");
  assert.equal(text(".recovery"), "ABCD-EFGH-IJKL");
  assert.equal(backend.called("setup")[0].useKeyring, false);

  const proceed = $(".gate button.primary");
  assert.equal(proceed.disabled, true);
  await check($(".gate input[type=checkbox]"));
  assert.equal(proceed.disabled, false);
  await submit($(".gate form"));
  assert.ok($(".app"));
});
