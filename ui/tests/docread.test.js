import assert from "node:assert/strict";
import { before, test } from "node:test";
import { $, $$, byText, click, dblclick, goTo, settle, startApp, submit, text, type } from "./setup.js";

const file = (id, name, type, description, more = {}) => ({
  id, case_id: 1, profile_id: null, link_id: "", name, type, size: 2048, description, created_at: "2026-10-28T09:00:00", link_title: "", profile_name: null, ...more,
});

let backend;
before(async () => {
  backend = await startApp({
    profiles: [
      { id: 1, name: "John", kind: "person" },
      { id: 5, name: "Mary Example", kind: "person", global: false },
    ],
    fields: {
      1: [],
      5: [
        { key: "name", label: "Full name", type: "text", value: "Mary Example" },
        { key: "egn", label: "EGN", type: "egn", value: "" },
        { key: "id_card.number", label: "ID card number", type: "text", value: "640000001" },
      ],
    },
    cases: [{ id: 1, title: "Sale, 12 Oak Street", notes: "", created_at: "2026-10-28T09:00:00", updated_at: "2026-11-02T10:00:00" }],
    caseLinks: [
      { case_id: 1, profile_id: 5, role: "" },
      { case_id: 1, profile_id: 1, role: "buyer" },
    ],
    collected: [
      file(11, "mary-id.jpg", "image/jpeg", "Mary Example, ID card", { doctype: "bg_id_card", document: "ID card", profile_id: 5, profile_name: "Mary Example" }),
      file(12, "title-deed.pdf", "application/pdf", "Title deed, 2019"),
    ],
    reads: {
      11: [
        { key: "name", label: "Full name", read: "Mary Example", box: [300, 380, 560, 410] },
        { key: "egn", label: "EGN", type: "egn", read: "7501020019", status: "failed", box: [300, 300, 460, 330] },
        { key: "id_card.number", label: "ID card number", read: "640000007", box: [600, 470, 780, 500] },
      ],
    },
  });
  await goTo("Cases");
  // happy-dom lays nothing out: every drop lands on the open case
  document.elementFromPoint = () => $(".case-tab");
});
const onCase = { x: 0, y: 0 };

const cells = (part) => [...$$(".case-tab .grid tbody tr").find((tr) => tr.textContent.includes(part)).children].map((td) => td.textContent.trim());
const row = (part) => $$(".case-tab .grid tbody tr").find((tr) => tr.textContent.includes(part));
const menuOf = async (part) => {
  await click($(".more", row(part)));
  return $$(".menu-item").map((el) => el.textContent);
};
const closeMenu = () => document.body.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
const tab = (label) => click($$(".case-tabs .chip").find((el) => el.textContent.trim().startsWith(label)));
const profileOn = (part) => $(".file-profile", row(part))?.textContent ?? null;
const input = (key) => $(`.doc-check input[name="${key}"]`);
const chips = () => $$(".doc-check .hint .chip").map((el) => el.textContent.trim());

test("a recognized file shows what document it is and the profile it became", async () => {
  assert.deepEqual($$(".case-tab .grid thead th").map((th) => th.textContent.trim()).filter(Boolean), ["Description", "Type", "Size", "Added"], "the profile is part of the description, not a column");
  assert.equal(cells("mary-id.jpg")[2], "ID card");
  assert.equal(profileOn("mary-id.jpg"), "Mary Example");
  assert.deepEqual($$(".file-sub > *", row("mary-id.jpg")).map((el) => el.textContent), ["mary-id.jpg", "Mary Example"], "under the description: the file's name, then its profile");
  assert.equal(cells("Title deed")[2], "PDF");
  assert.equal(profileOn("Title deed"), null);

  assert.deepEqual(await menuOf("mary-id.jpg"), ["Open", "Export…", "Rename…", "Description…", "Check values…", "Make profile global", "Delete"]);
  closeMenu();
  await settle();
  assert.deepEqual(await menuOf("Title deed"), ["Open", "Export…", "Rename…", "Description…", "Personal data…", "Read as a document", "Delete"]);
  closeMenu();
  await settle();
});

test("the profile's name on the file is text, not a control; the Profiles tab says which document it came from", async () => {
  assert.equal($(".file-profile", row("mary-id.jpg")).tagName, "SPAN");
  await click($(".file-profile", row("mary-id.jpg")));
  assert.equal($(".sheet"), null, "the row opens on a double-click, like every row");

  await tab("Profiles");
  assert.equal(cells("Mary Example")[3], "ID card", "no role yet: the document stands in");
  assert.equal(cells("John")[3], "buyer");
  await tab("Files");
});

test("Check values… puts the profile beside the document and offers what the document says", async () => {
  await click($(".more", row("mary-id.jpg")));
  await click(byText(".menu-item", "Check values…"));
  assert.deepEqual(backend.called("check_document").at(-1), { id: 11 });
  assert.equal(text(".sheet h2"), "Mary Example");
  assert.ok(text(".sheet-head").includes("ID card"));
  assert.deepEqual($$(".doc-check .input-form > label").map((el) => el.textContent), ["Full name", "EGN", "ID card number"]);
  assert.deepEqual(["name", "egn", "id_card.number"].map((key) => input(key).value), ["Mary Example", "", "640000001"]);
  assert.deepEqual(chips(), ["Failed its check: 7501020019", "In the document: 640000007"], "no offer where the profile and the document agree");
  assert.ok($(".doc-check .chip.Failed"));
  assert.equal($$(".doc-check .pdf-page .box").length, 3);

  input("egn").dispatchEvent(new Event("focus"));
  await settle();
  assert.deepEqual($$(".doc-check .box.active").map((el) => el.dataset.var), ["egn"], "the focused field's place is outlined");
  await click($('.doc-check .box[data-var="name"]'));
  assert.equal(document.activeElement, input("name"), "a box leads to its field");

  await click(byText(".doc-check .chip", "In the document: 640000007"));
  assert.equal(input("id_card.number").value, "640000007");
  assert.deepEqual(chips(), ["Failed its check: 7501020019"]);
});

test("saving checks the values, then writes them into the profile", async () => {
  await type(input("egn"), "123");
  await click(byText(".sheet-foot button", "Save"));
  assert.equal(text(".doc-check .field-problem"), "Invalid EGN", "under its field");
  assert.ok(input("egn").classList.contains("invalid"));
  assert.ok($(".sheet"), "the sheet stays open on a problem");

  await type(input("egn"), "7501020018");
  await submit($(".doc-check form"));
  assert.deepEqual(backend.called("save_document_check").at(-1), {
    id: 11,
    values: { name: "Mary Example", egn: "7501020018", "id_card.number": "640000007" },
  });
  assert.equal($(".sheet"), null);
  assert.equal(text("#toast"), "Saved");
  assert.equal(backend.db.fields[5].find((f) => f.key === "egn").value, "7501020018");
});

test("the profile of a file is made global from the file", async () => {
  await click($(".more", row("mary-id.jpg")));
  await click(byText(".menu-item", "Make profile global"));
  assert.equal(text(".modal h3"), 'Make "Mary Example" global?');
  await submit($(".modal form"));
  assert.deepEqual(backend.called("make_profile_global").at(-1), { id: 5 });
  assert.ok(!(await menuOf("mary-id.jpg")).includes("Make profile global"), "once global, nothing to offer");
  closeMenu();
  await settle();
});

test("an imported file is read in the background and becomes a profile of the case", async () => {
  backend.db.reading = {
    delay: 150,
    of: (f) =>
      f.name.includes("registration") && {
        doctype: "bg_vehicle_registration",
        document: "Vehicle registration",
        profile: { name: "CA 1234 AB", kind: "vehicle" },
        fields: [{ key: "plate", label: "Plate", type: "text", value: "CA 1234 AB" }],
        reads: [{ key: "plate", label: "Plate", read: "CA 1234 AB", box: [100, 400, 300, 430] }],
      },
  };
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/car-registration.jpg", "/Users/test/notes.jpg"], position: onCase });
  await settle();
  await click(byText(".sheet-foot button", "Import 2 files"));
  assert.equal(cells("car-registration.jpg")[2], "Reading…");
  assert.equal(cells("notes.jpg")[2], "Reading…");

  await settle(250);
  assert.equal(cells("car-registration.jpg")[2], "Vehicle registration");
  assert.equal(profileOn("car-registration.jpg"), "CA 1234 AB");
  assert.equal(cells("notes.jpg")[2], "Image", "a file that is no known document stays a plain file");
  await tab("Profiles");
  assert.deepEqual(cells("CA 1234 AB").slice(1, 5), ["CA 1234 AB", "vehicle", "Vehicle registration", "Only in this case"]);
});

test("a file already in the case is read again from its menu", async () => {
  await tab("Files");
  assert.ok((await menuOf("notes.jpg")).includes("Read as a document"), "a plain image can be tried again");
  closeMenu();
  await settle();
  assert.ok(!(await menuOf("car-registration.jpg")).includes("Read as a document"), "not one that is a profile already");
  closeMenu();
  await settle();

  backend.db.reading.of = () => ({
    doctype: "bg_id_card",
    document: "ID card",
    profile: { name: "Anna Demo", kind: "person" },
    fields: [],
    reads: [],
  });
  await click($(".more", row("notes.jpg")));
  await click(byText(".menu-item", "Read as a document"));
  assert.deepEqual(backend.called("read_case_file").at(-1), { id: backend.db.collected.find((f) => f.name === "notes.jpg").id });
  assert.equal(cells("notes.jpg")[2], "Reading…");
  await settle(250);
  assert.equal(cells("notes.jpg")[2], "ID card");
  assert.equal(profileOn("notes.jpg"), "Anna Demo");
});

test("opening a recognized file shows its profile beside it; the file is dragged to move and pinched to zoom", async () => {
  await dblclick(row("car-registration.jpg"));
  assert.equal(text(".sheet h2"), "CA 1234 AB", "not the plain viewer: the profile on the left, the file on the right");
  assert.equal(backend.called("check_document").at(-1).id, backend.db.collected.find((f) => f.name === "car-registration.jpg").id);
  assert.equal(input("plate").value, "CA 1234 AB");

  const area = $(".doc-check .preview-area");
  const zoom = () => Number($(".zoom", area).style.getPropertyValue("--zoom") || 1);
  // happy-dom's wheel event does not take the modifier from its init
  const wheel = ({ ctrlKey = false, ...init }) => {
    const event = new WheelEvent("wheel", { clientX: 10, clientY: 10, bubbles: true, cancelable: true, ...init });
    Object.defineProperty(event, "ctrlKey", { value: ctrlKey });
    area.dispatchEvent(event);
  };
  wheel({ deltaY: -200 });
  assert.equal(zoom(), 1, "the wheel alone scrolls, as everywhere");
  wheel({ deltaY: -50, ctrlKey: true });
  assert.ok(zoom() > 1, "a pinch (the wheel with ctrl) zooms in");
  const closer = zoom();
  wheel({ deltaY: 80, ctrlKey: true });
  assert.ok(zoom() < closer, "and out");

  // holding the left button and dragging moves the picture the way the pointer goes
  area.scrollLeft = 100;
  area.scrollTop = 100;
  area.dispatchEvent(new MouseEvent("mousedown", { button: 0, clientX: 200, clientY: 200, bubbles: true }));
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 170, clientY: 240, bubbles: true }));
  assert.deepEqual([area.scrollLeft, area.scrollTop], [130, 60]);
  window.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  window.dispatchEvent(new MouseEvent("mousemove", { clientX: 0, clientY: 0, bubbles: true }));
  assert.deepEqual([area.scrollLeft, area.scrollTop], [130, 60], "released: the pointer moves freely again");
  await click(byText(".sheet-foot button", "Cancel"));

  await dblclick(row("Title deed"));
  assert.ok($(".sheet .file-view"), "a plain file still opens in the viewer");
  await click(byText(".sheet-head button", "Close"));
});

test("without a document config set from Blankey Online nothing is read", async () => {
  backend.db.docConfigs = { state: "none", version: "", checked_at: null, documents: 0, skipped: 0 };
  backend.emit("doc-configs-changed");
  await settle();
  backend.emit("tauri://drag-drop", { paths: ["/Users/test/scan.jpg"], position: onCase });
  await settle();
  await click(byText(".sheet-foot button", "Import 1 file"));
  assert.equal(cells("scan.jpg")[2], "Image", "no Reading…: it stays a plain file");
  assert.ok(!(await menuOf("scan.jpg")).includes("Read as a document"));
  closeMenu();
  await settle();
  assert.ok((await menuOf("mary-id.jpg")).includes("Check values…"), "what was read before stays a profile");
  closeMenu();
  await settle();

  backend.db.docConfigs = { state: "valid", version: "v1", checked_at: "2026-11-14T18:30:00Z", documents: 3, skipped: 0 };
  backend.emit("doc-configs-changed");
  await settle();
  assert.ok((await menuOf("scan.jpg")).includes("Read as a document"), "with a set again, it can be read");
  closeMenu();
  await settle();
});
