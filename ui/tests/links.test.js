import assert from "node:assert/strict";
import { before, test } from "node:test";
import { submission } from "./backend.js";
import { $, $$, action, byText, check, choose, click, goTo, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({
    collected: [
      { id: 4, name: "payslip.pdf", type: "application/pdf", size: 3 * 1024 * 1024, profile_id: 1, link_id: "XchaBj2", created_at: "2026-10-17T12:00:00+00:00", link_title: "Tenant details", profile_name: "John" },
    ],
  });
});

const rows = () => $$(".page > table.grid > tbody > tr, .links-main > table.grid > tbody > tr").map((tr) => tr.textContent.replace(/\s+/g, " ").trim());

test("collection links: listed with their counts; Copy copies the full link", async () => {
  await goTo("Collection links");
  assert.equal(text(".toolbar h2"), "Collection links");
  const [row] = rows();
  assert.ok(row.includes("Tenant details") && row.includes("relay.blankey.io/s/XchaBj2") && row.includes("2/3"), row);
  assert.ok(row.includes("2026-10-17 12:00") && row.includes("1 to review"), row);
  assert.ok(!row.includes("k4Fp9QzR2mWx7aLc"), "the fragment is not shown");
  await click(byText(".grid button", "Copy"));
  assert.deepEqual(backend.called("copy_text").at(-1), { text: "https://relay.blankey.io/s/XchaBj2#k4Fp9QzR2mWx7aLc" });
});

test("collection links: filters and search narrow the list", async () => {
  const chip = (label) => $$(".filters .chip").find((b) => b.textContent.trim().startsWith(label));
  assert.equal(chip("To review").textContent.replace(/\s+/g, " ").trim(), "To review 1");
  backend.db.links.push({ ...backend.db.links[0], link_id: "Zz9yX8w", url: "https://relay.blankey.io/s/Zz9yX8w#x", title: "Old intake", state: "expired", pending: 0, last_received: null, fields: ["Phone"] });
  await goTo("Activity");
  await goTo("Collection links");

  assert.equal(rows().length, 2);
  await click(chip("To review"));
  assert.deepEqual(rows().map((r) => r.split(" ")[0]), ["Tenant"]);
  await click(chip("Closed"));
  assert.deepEqual(rows().map((r) => r.split(" ")[0]), ["Old"]);
  await click(chip("All"));
  await type($("#search"), "phone");
  assert.deepEqual(rows().map((r) => r.split(" ")[0]), ["Old"]);
  await type($("#search"), "nothing like this");
  assert.equal(text(".empty"), "No links match.");
  await type($("#search"), "");
  backend.db.links.pop();
  await goTo("Activity");
  await goTo("Collection links");
});

test("collection links: a link's submissions open again, the waiting one to review", async () => {
  backend.db.submissions["s-1"] = submission(11, "s-1", "saved");
  backend.db.submissions["s-2"] = submission(12, "s-2");
  await click($$(".grid tbody tr").find((tr) => tr.textContent.includes("Tenant details")));
  assert.equal(text(".side h3"), "Tenant details");
  assert.ok(!$$(".links-main th").some((th) => th.textContent === "Fields"), "the table narrows beside the panel");
  const subRows = () => $$(".side .sub-item").map((item) => item.textContent.replace(/\s+/g, " ").trim());
  assert.equal(subRows().length, 2);
  assert.ok(subRows()[0].includes("To review") && subRows()[0].includes("Review"), subRows()[0]);
  assert.ok(subRows()[1].includes("Saved") && subRows()[1].includes("into John"), subRows()[1]);

  await click(byText(".submissions button", "Open"));
  assert.deepEqual(backend.called("open_submission").at(-1), { sid: "s-1" });
  assert.equal(text(".sheet h2"), "Submission for Tenant details");
  assert.ok(text(".sheet .reason").startsWith("Received 2026-10-17 12:00 · saved 2026-10-17 12:05"));
  assert.ok($$(".file-row input.keep").every((input) => !input.checked), "kept files are not ticked again");
  assert.ok(byText(".sheet-foot button", "Save again"));
  await click(byText(".sheet-foot button", "Close"));
  assert.equal($(".sheet"), null);

  await click(byText(".submissions button", "Review"));
  assert.deepEqual(backend.called("open_submission").at(-1), { sid: "s-2" });
  assert.ok($$(".file-row input.keep").every((input) => input.checked));
  await click(byText(".sheet-foot button", "Later"));

  await click($(".side .close"));
  assert.equal($(".side"), null);
  assert.ok($$(".links-main th").some((th) => th.textContent === "Fields"));
});

test("new link: ticked profile fields and custom fields make the draft", async () => {
  await action("New link");
  assert.equal(text(".sheet h2"), "New collection link");
  assert.equal($(".sheet select[name=profile]").value, "1");
  assert.deepEqual($$(".link-fields td.mono").map((td) => td.textContent), ["name", "egn"]);

  await check($$(".link-fields input.ask")[0]);
  await click(byText(".sheet button", "Add field"));
  await type($(".link-fields tr.custom input.key"), "phone");
  await type($(".link-fields tr.custom input.label"), "Phone");
  await type($(".sheet input[name=max_files]"), "2");
  await type($(".sheet input[name=kinds]"), ".pdf, image/*");
  await choose($(".sheet select[name=days]"), "30");

  await click(byText(".sheet-foot button", "Create link"));
  assert.equal(text(".sheet .problems"), "Give the link a title");
  await type($(".sheet input[name=title]"), "Lease");
  await click(byText(".sheet-foot button", "Create link"));
  const { draft, requestId } = backend.called("create_link").at(-1);
  assert.equal(requestId, null);
  assert.deepEqual(draft, {
    title: "Lease",
    from: "",
    note: "",
    profile_id: 1,
    new_profile: null,
    fields: [
      { key: "name", label: "Full name", type: "text", hint: "", required: false },
      { key: "phone", label: "Phone", type: "text", hint: "", required: false },
    ],
    max_files: 2,
    kinds: [".pdf", "image/*"],
    expires_days: 30,
    max_submissions: 1,
  });
  assert.equal(backend.called("copy_text").at(-1).text, "https://relay.blankey.io/s/Ab3dE9z#qqj_9wO1CyKX9Pbh");
  assert.equal($(".sheet"), null);
  assert.equal(rows().length, 2);
});

test("new link: pressing a checkbox and dragging down the column paints it", async () => {
  backend.db.fields[1].push({ key: "phone", label: "Phone", type: "text", value: "" });
  await action("New link");
  const cells = (cls) => $$(`.link-fields input.${cls}`).map((input) => input.parentElement);
  const states = (cls) => $$(`.link-fields input.${cls}`).map((input) => input.checked);
  const drag = async (from, over) => {
    from.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    await settle();
    for (const cell of over) {
      cell.dispatchEvent(new MouseEvent("mouseenter"));
      await settle();
    }
    document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    await settle();
  };

  const [first, second, third] = cells("ask");
  await drag(first, [second, third]);
  assert.deepEqual(states("ask"), [true, true, true]);
  // starting on a ticked box unticks the ones passed over
  await drag(second, [third]);
  assert.deepEqual(states("ask"), [true, false, false]);
  // after mouseup, passing over boxes changes nothing
  cells("ask")[1].dispatchEvent(new MouseEvent("mouseenter"));
  await settle();
  assert.deepEqual(states("ask"), [true, false, false]);
  // the required column paints on its own
  await drag(cells("required")[0], [cells("required")[1], cells("ask")[2]]);
  assert.deepEqual(states("required"), [true, true, false]);
  assert.deepEqual(states("ask"), [true, false, false]);

  // keys toggle and paint the Ask column too
  const keys = $$(".link-fields td.mono");
  await drag(keys[1], [keys[2]]);
  assert.deepEqual(states("ask"), [true, true, true]);
  await drag(keys[0], []);
  assert.deepEqual(states("ask"), [false, true, true]);

  await click(byText(".sheet-foot button", "Cancel"));
  backend.db.fields[1].pop();
});

test("new link into a new profile needs its name", async () => {
  await action("New link");
  await choose($(".sheet select[name=profile]"), "new");
  assert.equal($$(".link-fields td.mono").length, 0);
  await type($(".sheet input[name=title]"), "Intake");
  await click(byText(".sheet-foot button", "Create link"));
  assert.equal(text(".sheet .problems"), "Name the new profile");
  await type($(".sheet input[name=new_profile]"), "New client");
  await click(byText(".sheet-foot button", "Create link"));
  const { draft } = backend.called("create_link").at(-1);
  assert.equal(draft.new_profile, "New client");
  assert.equal(draft.profile_id, null);
});

test("revoke asks first", async () => {
  await click($$(".grid tbody tr").find((tr) => tr.textContent.includes("relay.blankey.io/s/XchaBj2")));
  await action("Revoke");
  assert.equal(text(".modal h3"), 'Revoke "Tenant details"?');
  await submit($(".modal form"));
  assert.deepEqual(backend.called("revoke_link").at(-1), { linkId: "XchaBj2" });
  assert.ok(rows().some((r) => r.includes("Tenant details") && r.includes("Revoked")));
});

test("documents: collected files sit with generated ones and use their own commands", async () => {
  await goTo("Documents");
  const listed = rows();
  assert.equal(listed.length, 3);
  const collected = listed.find((r) => r.includes("payslip.pdf"));
  assert.ok(collected.includes("Collected") && collected.includes("Tenant details · John") && collected.includes("3.0 MB"), collected);

  const row = $$(".grid tbody tr").find((tr) => tr.textContent.includes("payslip.pdf"));
  await click(row);
  await action("Open");
  assert.deepEqual(backend.called("open_collected_files").at(-1), { ids: [4] });
  assert.equal(backend.called("open_documents").length, 0);
  await action("Export");
  assert.deepEqual(backend.called("export_collected_file").at(-1), { id: 4 });

  await click($$(".grid tbody tr").find((tr) => tr.textContent.includes("Loan")));
  await click(row, { metaKey: true });
  await action("Delete");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("delete_collected_files").at(-1), { ids: [4] });
  assert.deepEqual(backend.called("delete_documents").at(-1), { ids: [2] });
});

test("signed out with no links: the page offers to sign in", async () => {
  await goTo("Connections");
  await action("Sign out");
  backend.db.links = [];
  await goTo("Collection links");
  await settle();
  assert.ok(text(".empty").startsWith("Sign in to the relay to make collection links."));
  await click(byText(".empty button", "Sign in"));
  assert.equal(backend.called("relay_login").length, 1);
});
