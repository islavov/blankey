// The cases workflow in its order: a case, the client's files, the profiles read from them, a template filled in,
// the documents that come out, and the collection links that bring in data from other people.

const OAK = ".case-item[data-case='1']";
const NEW_CASE = "Lease, flat on Park Lane";

export default {
  id: "cases",
  async run(app) {
    await app.click(OAK);
    await app.tab("Files");
    await app.shot("case");

    await app.action("New case");
    await app.fill("#modal-root input[name=title]", NEW_CASE);
    await app.shot("new-case", { target: ".modal-card" });
    await app.click("#modal-root button[type=submit]");
    await app.tab("Files");
    await app.shot("empty-files");

    await app.backend((backend) => (backend.db.pickPaths = ["/Users/demo/Documents/id-card-anna.jpg", "/Users/demo/Documents/lease-draft.docx"]));
    await app.action("Import…");
    await app.fill(app.page.locator(".import-row input").nth(0), "ID card of the tenant");
    await app.fill(app.page.locator(".import-row input").nth(1), "Draft of the lease from the landlord");
    await app.shot("import");
    await app.click(".sheet-foot button.primary");
    await app.shot("reading");
    await app.wait(app.row("Anna Demo"));
    await app.shot("read");
    await app.menu(app.row("Anna Demo"), "Check values…");
    await app.shot("check-values");
    await app.press("Escape");
    await app.menu(app.row("lease-draft"), "Personal data…");
    await app.shot("personal-data");
    await app.press("Escape");

    await app.tab("Profiles");
    await app.menu(app.row("Anna Demo"), "Role…");
    await app.fill("#modal-root input[name=role]", "tenant");
    await app.shot("role", { target: ".modal-card" });
    await app.click("#modal-root button[type=submit]");
    await app.shot("profiles");

    await app.click(OAK);
    await app.tab("Templates");
    await app.shot("templates");
    await app.dblclick(app.row("Sale contract"));
    await app.wait(".fill-preview .blank.is-empty");
    await app.shot("fill");
    await app.click(".fill-row[data-var=price] .source");
    await app.shot("fill-source");
    await app.click(".source-list .source-pick");
    await app.press("Escape");
    await app.click(".fill-row[data-var=deed_date] .source");
    await app.click(".source-list .source-pick");
    await app.press("Escape");
    await app.wait(".fill-preview .blank[data-var=deed_date]:not(.is-empty)");
    await app.shot("fill-ready");
    await app.click({ text: "Save and Generate" });
    await app.shot("documents", { toast: true });
    await app.menu(app.page.locator("tr.fresh"), "Open");
    await app.shot("viewer");
    await app.press("Escape");
    await app.tab("Fill sets");
    await app.shot("fill-sets");
    await app.tab("Activity");
    await app.shot("activity");

    await app.goTo("Collection links");
    await app.shot("links");
    await app.action("New link");
    await app.shot("new-link");
    await app.press("Escape");

    await app.goTo("Cases");
    await app.click(app.page.locator(".case-item").filter({ hasText: NEW_CASE }));
    await app.action("Delete case");
    await app.shot("delete-case", { target: ".modal-card" });
    await app.press("Escape");
  },
};
