import assert from "node:assert/strict";
import { test } from "node:test";
import { $, startApp, submit, text, type } from "./setup.js";

test("unlock: Touch ID is tried once, a wrong password is refused, the right one opens the app", async () => {
  const backend = await startApp({ unlocked: false });
  assert.equal(text(".gate h1"), "Blankey is locked");
  assert.equal(backend.called("unlock_quick").length, 1);

  const form = $(".gate form");
  await type($("input[name=password]", form), "nope");
  await submit(form);
  assert.equal(text(".gate .error"), "Wrong password or key");
  assert.equal($(".app"), null);

  await type($("input[name=password]", form), "pw");
  await submit(form);
  assert.ok($(".app"));
  assert.equal(text(".toolbar h2"), "Activity");
  assert.equal(backend.called("unlock_quick").length, 1);
});
