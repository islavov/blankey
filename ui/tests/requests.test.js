import assert from "node:assert/strict";
import { before, test } from "node:test";
import { fileRequest, fillRequest, linkRequest, profileInputRequest, submission, submissionRequest } from "./backend.js";
import { $, $$, byText, check, choose, click, key, settle, startApp, submit, text, type } from "./setup.js";

let backend;
before(async () => {
  backend = await startApp({ requests: [profileInputRequest(8)] });
});

test("profile input: required fields are checked, values go to the vault", async () => {
  assert.equal(text(".sheet h2"), "2 fields for John");
  assert.equal(text(".sheet .reason"), "Need contact details");
  const email = $("input[name=f0]");
  assert.equal(email.value, "old@example.com");

  await type(email, "");
  await click(byText(".sheet-foot button", "Save"));
  assert.equal(text(".problems"), "Email is required");
  assert.ok($(".sheet"));

  await type(email, "john@example.com");
  await check($("input[name=f1]"));
  await click(byText(".sheet-foot button", "Save"));
  const resolved = backend.called("resolve_profile_input").at(-1);
  assert.deepEqual(resolved.values, { email: "john@example.com", vip: "yes" });
  assert.equal(resolved.profileName, null);
  assert.equal($(".sheet"), null);
});

test("a request that arrives while the app is open is shown; Escape cancels it", async () => {
  backend.db.requests.push(fillRequest(9));
  backend.emit("request-created", 9);
  await settle(300);
  assert.equal(text(".sheet h2"), "Your AI assistant wants to fill in 2 documents");
  assert.ok($$(".fill-row").length > 0);

  await key("Escape", document.body);
  const closed = backend.called("fill_close").at(-1);
  assert.equal(closed.args.request_id, 9);
  assert.equal(closed.args.fill_set_id, null);
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});

test("file request: the user picks the file; closing the picker keeps the request open", async () => {
  backend.db.requests.push(fileRequest(10));
  backend.emit("request-created", 10);
  await settle(300);
  assert.equal(text(".sheet h2"), "Your AI assistant asks for a PDF file");
  assert.equal(text(".sheet .reason"), "The blank NZOK form");

  backend.db.pickedFile = null;
  await click(byText(".sheet-foot button", "Choose file…"));
  assert.ok($(".sheet"));
  assert.equal(backend.db.requests.length, 1);

  backend.db.pickedFile = "nzok.pdf";
  await click(byText(".sheet-foot button", "Choose file…"));
  assert.deepEqual(backend.called("resolve_file_request").at(-1), { id: 10 });
  assert.equal(text("#toast"), "Shared nzok.pdf");
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});

test("file request: Cancel cancels it", async () => {
  backend.db.requests.push(fileRequest(11));
  backend.emit("request-created", 11);
  await settle(300);
  await click(byText(".sheet-foot button", "Cancel"));
  assert.deepEqual(backend.called("cancel_request").at(-1), { id: 11 });
  assert.equal($(".sheet"), null);
});

test("submission: values are prefilled and editable; unticked files are not kept", async () => {
  backend.db.submissions["s-2"] = submission(12, "s-2");
  backend.db.requests.push(submissionRequest(12));
  backend.emit("request-created", 12);
  await settle(300);
  assert.equal(text(".sheet h2"), "Submission for Tenant details");
  assert.equal($(".sheet select[name=profile]").value, "new");
  assert.equal($(".sheet input[name=__profile]").value, "New tenant");
  assert.equal($(".sheet input[name=name]").value, "Jane Roe");
  assert.ok($(".sheet input[name=pets]").checked);
  assert.equal($$(".file-row").length, 2);
  assert.ok($(".file-row img.thumb"), "images get a thumbnail");
  assert.ok($$(".file-row")[1].textContent.includes("3.0 MB"));
  await click(byText(".file-row:nth-child(2) button", "Open"));
  assert.deepEqual(backend.called("open_submission_file").at(-1), { sid: "s-2", index: 1 });
  await click(byText(".file-row:nth-child(1) button", "Export"));
  assert.deepEqual(backend.called("export_submission_file").at(-1), { sid: "s-2", index: 0 });

  await type($(".sheet input[name=name]"), "");
  await click(byText(".sheet-foot button", "Save"));
  assert.equal(text(".sheet .problems"), "Full name: required");

  await type($(".sheet input[name=name]"), "Jane A. Roe");
  await check($$(".file-row input.keep")[1], false);
  await choose($(".sheet select[name=profile]"), "1");
  await click(byText(".sheet-foot button", "Save"));
  assert.deepEqual(backend.called("resolve_submission").at(-1), {
    sid: "s-2",
    profileId: 1,
    profileName: null,
    values: { name: "Jane A. Roe", pets: "yes" },
    keep: [0],
  });
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});

test("submission: Later keeps it waiting, Discard asks and drops it", async () => {
  backend.db.submissions["s-3"] = submission(13, "s-3");
  backend.db.requests.push(submissionRequest(13));
  backend.emit("request-created", 13);
  await settle(300);
  await click(byText(".sheet-foot button", "Later"));
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 1);

  backend.emit("request-created", 13);
  await settle(300);
  await click(byText(".sheet-foot button", "Discard"));
  assert.equal(text(".modal h3"), "Discard this submission?");
  await submit($(".modal form"));
  assert.deepEqual(backend.called("discard_submission").at(-1), { sid: "s-3" });
  assert.equal(backend.db.requests.length, 0);
});

test("a submission that cannot be opened is skipped", async () => {
  backend.db.requests.push(submissionRequest(15));
  backend.emit("request-created", 15);
  await settle(300);
  assert.equal($(".sheet"), null);
  assert.equal(text("#toast"), "This submission could not be opened, so it was dropped.");
  backend.db.requests = [];
});

test("collection link drafted by the assistant: the user confirms it in the New link sheet", async () => {
  backend.db.requests.push(linkRequest(14));
  backend.emit("request-created", 14);
  await settle(300);
  assert.equal(text(".sheet h2"), "Your AI assistant drafted a collection link");
  assert.equal(text(".sheet .reason"), "Collect the tenant's contact details");
  assert.equal($(".sheet input[name=title]").value, "Contact details");
  assert.equal($(".sheet select[name=days]").value, "30");
  const ticked = $$(".link-fields tbody tr").filter((tr) => tr.querySelector("input.ask")?.checked).map((tr) => tr.querySelector("td.mono").textContent);
  assert.deepEqual(ticked, ["name"]);
  assert.equal($(".link-fields tr.custom input.key").value, "phone");

  await click(byText(".sheet-foot button", "Create link"));
  const { draft, requestId } = backend.called("create_link").at(-1);
  assert.equal(requestId, 14);
  assert.deepEqual(draft.fields.map((f) => [f.key, f.label, f.required]), [["name", "Name", true], ["phone", "Phone", false]]);
  assert.equal($(".sheet"), null);
  assert.equal(backend.db.requests.length, 0);
});
