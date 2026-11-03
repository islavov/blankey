import { useEffect, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { openFill } from "../fill/form.js";
import { HelpNote } from "../help.js";
import { t, tn } from "../i18n.js";
import { html, KIND_NAMES, matches, megabytes, reload, state, update, useLoad, useStore, when } from "../lib.js";
import { openReview } from "../review.js";
import { confirmDialog, dialog, isDialogOpen, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { isSheetOpen, openSheet, SheetHead } from "../ui/sheet.js";
import { CheckTable, keepText } from "../ui/table.js";
import { openActivity } from "./activity.js";
import { generateFillSets } from "./fillsets.js";
import { openProfileSheet } from "./profiles.js";

// the case the page shows, where dropped files go
let shown = null;
// adds dropped files to the open Import sheet
let importing = null;

/** Files dropped on the window: into the Import sheet when it is open, else a new import into the open case. */
export function dropFiles(paths) {
  if (!paths.length) return;
  if (importing) return importing(paths);
  if (isSheetOpen() || isDialogOpen()) return;
  if (state.page !== "cases" || !shown) return toast(t("Open a case to import files into it"));
  openImport(shown, paths);
}

export function Cases() {
  const { caseId, dragging } = useStore();
  const cases = useLoad(() => call("list_cases"), []);
  if (!cases) return null;

  const visible = cases.filter((c) => matches(c.title, ...c.profiles));
  const current = visible.find((c) => c.id === caseId) || visible[0] || null;
  shown = current;

  const create = async () => {
    const values = await dialog({
      title: t("New case"),
      fields: [{ name: "title", label: t("Title (sale of an apartment, lease…)") }],
      ok: t("Create"),
    });
    if (!values?.title?.trim()) return;
    const id = await call("create_case", { title: values.title.trim() });
    update({ caseId: id, caseTab: "files", search: "" });
    reload();
  };

  const remove = async () => {
    if (!current) return toast(t("Select a case"));
    const info = await call("get_case", { id: current.id });
    const { counts } = info;
    const parts = [
      tn(counts.files, "{n} file", "{n} files"),
      tn(counts.fill_sets, "{n} fill set", "{n} fill sets"),
      tn(counts.documents, "{n} document", "{n} documents"),
      tn(counts.templates, "{n} template", "{n} templates"),
    ];
    const only = info.profiles.filter((p) => !p.global && p.cases === 1).map((p) => p.name);
    const text = [
      t("Deleted with it: {items}.", { items: parts.join(", ") }),
      only.length && t("Profiles that exist only in this case are deleted too: {names}.", { names: only.join(", ") }),
      t("Global profiles and the profiles of other cases stay."),
    ];
    if (!(await confirmDialog(t("Delete the case \"{name}\"?", { name: info.title }), text.filter(Boolean).join("\n")))) return;
    await call("delete_case", { id: info.id });
    update({ caseId: null });
    reload();
  };

  return html`
    <${Page} title=${t("Cases")} subtitle=${tn(cases.length, "{n} case", "{n} cases")} actions=${[
      [t("New case"), create, "primary", "cases.new"],
      [t("Import…"), () => (current ? importInto(current) : toast(t("Make a case first"))), "", "cases.import"],
      [t("Delete case"), remove, "danger", "cases.delete"],
    ]}>
      ${cases.length
        ? html`
            <div class="split cases">
              <div class="list" data-help="cases.list">
                ${visible.map(
                  (c) => html`
                    <div key=${c.id} class=${`list-item case-item ${c.id === current?.id ? "active" : ""}`} onMouseDown=${keepText}
                      onClick=${() => update({ caseId: c.id })}>
                      <div class="title">${c.title}</div>
                      ${c.profiles.length > 0 && html`<div class="muted small line">${c.profiles.join(", ")}</div>`}
                      <div class="muted small line">${tn(c.files, "{n} file", "{n} files")} · ${when(c.updated_at)}</div>
                    </div>`
                )}
                ${!visible.length && html`<div class="muted small list-note">${t("No cases match.")}</div>`}
              </div>
              <div class="detail case-detail">${current && html`<${CaseDetail} key=${current.id} caseId=${current.id} />`}</div>
            </div>`
        : html`<${Empty}>
            ${t("No cases yet. A case keeps one matter together: the people in it, the documents they brought and what you fill in for them.")}
            <div style="margin-top:14px"><button class="primary" onClick=${create}>${t("New case")}</button></div>
          </${Empty}>`}
      ${dragging &&
      html`<div class="drop-overlay">
        <div class="drop-text">${current ? t("Drop the files into “{title}”", { title: current.title }) : t("Make a case first")}</div>
      </div>`}
    </${Page}>`;
}

function CaseDetail({ caseId }) {
  const { caseTab } = useStore();
  const info = useLoad(() => call("get_case", { id: caseId }), [caseId]);
  if (!info) return null;

  const tabs = [
    ["files", t("Files"), info.counts.files],
    ["profiles", t("Profiles"), info.counts.profiles],
    ["templates", t("Templates"), info.counts.templates],
    ["fillsets", t("Fill sets"), info.counts.fill_sets],
    ["documents", t("Documents"), info.counts.documents],
    ["activity", t("Activity"), info.counts.activity],
  ];
  const content = {
    files: () => html`<${CaseFiles} info=${info} />`,
    profiles: () => html`<${CaseProfiles} info=${info} />`,
    templates: () => html`<${CaseTemplates} info=${info} />`,
    fillsets: () => html`<${CaseFillSets} info=${info} />`,
    documents: () => html`<${CaseDocuments} info=${info} />`,
    activity: () => html`<${CaseActivity} info=${info} />`,
  };
  return html`
    <${CaseHead} info=${info} />
    <div class="filters case-tabs" data-help="case.tabs">
      ${tabs.map(([id, label, n]) => html`
        <button class=${`chip ${caseTab === id ? "active" : ""}`} onClick=${() => update({ caseTab: id })}>
          ${label} <span class="count">${n}</span>
        </button>`)}
    </div>
    <div class="case-tab">${(content[caseTab] || content.files)()}</div>`;
}

/** What the user pastes to the AI assistant to name a case; the assistant passes the number as case_id. */
export const caseLink = (id) => `blankey://case/${id}`;

/** The title and notes, saved when they lose focus, and the case's link for the AI assistant. */
function CaseHead({ info }) {
  const copyLink = async () => {
    const link = caseLink(info.id);
    await call("copy_text", { text: link });
    toast(t("Copied {id}", { id: link }));
  };
  const save = async (patch) => {
    await call("update_case", { id: info.id, ...patch });
    reload();
  };
  const saveTitle = (e) => {
    const title = e.target.value.trim();
    if (!title) e.target.value = info.title;
    else if (title !== info.title) save({ title });
  };
  return html`
    <div class="case-head">
      <div class="case-title-row">
        <input type="text" class="case-title" data-help="case.title" value=${info.title} spellcheck="false"
          onKeyDown=${(e) => e.key === "Enter" && e.target.blur()} onBlur=${saveTitle} />
        <button class="copy-case" data-help="case.copy" onClick=${copyLink}>${t("Copy link")}</button>
      </div>
      <textarea class="case-notes" data-help="case.notes" rows="2" placeholder=${t("Notes")} value=${info.notes}
        onBlur=${(e) => e.target.value !== info.notes && save({ notes: e.target.value })}></textarea>
    </div>`;
}

// -- tabs --------------------------------------------------------------------------------

/**
 * A case tab: its own actions on top, or, once rows are checked, what to do with them. Each row has a ⋮ menu for
 * itself. actions and bulk: [label, onClick, class?, help?], bulk's onClick gets the checked rows.
 */
function TabTable({ rows, columns, actions = [], bulk = [], menu, onOpen, empty, rowClass }) {
  const [checked, setChecked] = useState([]);
  const live = checked.filter((id) => rows.some((r) => r.id === id));
  const chosen = rows.filter((r) => live.includes(r.id));
  const checks = bulk.length > 0;
  const button = ([label, onClick, cls = "", help]) => html`<button class=${cls} data-help=${help} onClick=${onClick}>${label}</button>`;
  return html`
    ${(checks || actions.length > 0) &&
    html`<div class="tab-actions">
      ${live.length
        ? html`
            <span class="checked-count">${tn(live.length, "{n} selected", "{n} selected")}</span>
            ${bulk.map(([label, act, cls, help]) => button([label, () => act(chosen), cls, help]))}
            <button class="ghost" data-help="rows.clear" onClick=${() => setChecked([])}>${t("Clear")}</button>`
        : actions.map(button)}
    </div>`}
    ${rows.length
      ? html`<${CheckTable} rows=${rows} columns=${columns} checked=${live} onCheck=${setChecked} checks=${checks}
          menu=${menu} onOpen=${onOpen} rowClass=${rowClass} />`
      : html`<${Empty}>${empty}</${Empty}>`}`;
}

// -- files -------------------------------------------------------------------------------

const READABLE = ["application/pdf", "application/vnd.openxmlformats-officedocument.wordprocessingml.document"];

function fileKind(type) {
  if (type === "application/pdf") return "PDF";
  if (type.includes("wordprocessingml")) return "Word";
  if (type.startsWith("image/")) return t("Image");
  return t("File");
}

const fileLabel = (f) => f.description || f.name;

function CaseFiles({ info }) {
  const files = useLoad(() => call("list_collected_files", { caseId: info.id }), [info.id]);
  if (!files) return null;

  const open = (file) => call("open_collected_files", { ids: [file.id] });
  const describe = async (file) => {
    const values = await dialog({
      title: t("Description of {name}", { name: file.name }),
      fields: [{ name: "description", label: t("What it is and whose"), value: file.description }],
      ok: t("Save"),
    });
    if (!values) return;
    await call("update_case_file", { id: file.id, description: values.description.trim() });
    reload();
  };
  const exportFile = async (file) => {
    const path = await call("export_collected_file", { id: file.id });
    if (path) toast(t("Saved to {path}", { path }), 4000);
  };
  const remove = async (chosen) => {
    const title = chosen.length === 1 ? t("Delete \"{name}\"?", { name: fileLabel(chosen[0]) }) : tn(chosen.length, "Delete {n} file?", "Delete {n} files?");
    if (!(await confirmDialog(title, t("It is removed from the vault. Copies you exported stay where you saved them.")))) return;
    await call("delete_collected_files", { ids: chosen.map((f) => f.id) });
    reload();
  };

  return html`<${TabTable}
    rows=${files}
    actions=${[[t("Import…"), () => importInto(info), "", "cases.import"]]}
    bulk=${[[t("Delete"), remove, "danger", "casefiles.delete"]]}
    onOpen=${open}
    menu=${(f) => [
      { label: t("Open"), help: "casefiles.open", action: () => open(f) },
      { label: t("Export…"), help: "casefiles.export", action: () => exportFile(f) },
      { label: t("Description…"), help: "casefiles.describe", action: () => describe(f) },
      READABLE.includes(f.type) && { label: t("Personal data…"), help: "casefiles.pii", action: () => openReview({ source: `casefile:${f.id}`, title: fileLabel(f) }) },
      "-",
      { label: t("Delete"), danger: true, help: "casefiles.delete", action: () => remove([f]) },
    ]}
    columns=${[
      {
        title: t("Description"),
        cell: (f) => html`<div class="file-label">${fileLabel(f)}</div>${f.description && html`<div class="muted small">${f.name}</div>`}`,
        cls: "wide",
      },
      { title: t("Type"), cell: (f) => fileKind(f.type), cls: "nowrap" },
      { title: t("Size"), cell: (f) => megabytes(f.size), cls: "nowrap" },
      { title: t("Added"), cell: (f) => html`<span class="muted">${when(f.created_at)}</span>`, cls: "nowrap" },
    ]}
    empty=${t("No files yet. Drop the documents of this case here, or use Import…")} />`;
}

// -- import ------------------------------------------------------------------------------

async function importInto(info) {
  const paths = await call("pick_files").catch(() => []);
  if (paths.length) openImport(info, paths);
}

function openImport(info, paths) {
  return openSheet((close) => html`<${ImportSheet} info=${info} paths=${paths} close=${close} />`);
}

/** A file to import: its description starts as the file name, without the extension. */
const importItem = (path) => {
  const name = path.split(/[\\/]/).pop();
  return { path, name, description: name.replace(/\.[^.]+$/, "").replace(/[-_]+/g, " ").trim() };
};

function ImportSheet({ info, paths, close }) {
  const [items, setItems] = useState(() => paths.map(importItem));
  const add = (more) => setItems((current) => [...current, ...more.filter((p) => !current.some((i) => i.path === p)).map(importItem)]);
  useEffect(() => {
    importing = add;
    document.querySelector(".import-row input")?.select();
    return () => {
      importing = null;
    };
  }, []);

  const describe = (path, description) => setItems(items.map((i) => (i.path === path ? { ...i, description } : i)));
  const leaveOut = (path) => setItems(items.filter((i) => i.path !== path));
  const more = async () => add(await call("pick_files").catch(() => []));
  const save = async () => {
    const chosen = items.map(({ path, name, description }) => ({ path, description: description.trim() || name }));
    const n = await call("import_case_files", { caseId: info.id, items: chosen }).catch(() => null);
    if (n == null) return;
    toast(tn(n, "{n} file imported", "{n} files imported"));
    update({ caseId: info.id, caseTab: "files" });
    close();
    reload();
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="cases" title=${t("Import into “{title}”", { title: info.title })}
        reason=${t("Say what each file is and whose. Your AI assistant finds files by these descriptions.")} />
    </div>
    <div class="sheet-body">
      <div class="import-list">
        ${items.map(
          (i) => html`
            <div class="import-row" key=${i.path}>
              <span class="muted small import-name" title=${i.path}>${i.name}</span>
              <input type="text" data-help="import.description" value=${i.description} spellcheck="false"
                onInput=${(e) => describe(i.path, e.target.value)} />
              <button class="ghost remove" data-help="import.remove" title=${t("Leave out")} onClick=${() => leaveOut(i.path)}>✕</button>
            </div>`
        )}
        ${!items.length && html`<div class="empty">${t("No files to import.")}</div>`}
      </div>
    </div>
    <div class="sheet-foot">
      <${HelpNote}>${t("Drop more files on the window to add them.")}</${HelpNote}>
      <button onClick=${close}>${t("Cancel")}</button>
      <button data-help="import.more" onClick=${more}>${t("Add more…")}</button>
      <button class="primary" data-help="import.done" disabled=${!items.length} onClick=${save}>
        ${tn(items.length, "Import {n} file", "Import {n} files")}
      </button>
    </div>`;
}

// -- profiles ----------------------------------------------------------------------------

const ROLE_LABEL = () => t("Role in this case (buyer, seller…)");

function scopeLabel(p) {
  if (p.global) return t("Global");
  return p.cases > 1 ? tn(p.cases, "In {n} case", "In {n} cases") : t("Only in this case");
}

function CaseProfiles({ info }) {
  const create = async () => {
    const values = await dialog({
      title: t("New profile"),
      fields: [
        { name: "name", label: t("Name") },
        { name: "kind", label: t("Kind (person, company, loan…)"), value: "person" },
        { name: "role", label: ROLE_LABEL() },
      ],
      ok: t("Create"),
    });
    if (!values?.name?.trim()) return;
    await call("create_profile", { name: values.name.trim(), kind: values.kind, caseId: info.id, role: values.role.trim() });
    reload();
  };
  const addExisting = async () => {
    const options = await call("linkable_profiles", { caseId: info.id });
    if (!options.length) return toast(t("There are no other profiles to add."));
    const values = await dialog({
      title: t("Add a profile to this case"),
      fields: [
        { name: "profile", label: t("Profile"), options: options.map((p) => [p.id, p.global ? p.name : `${p.name} (${p.cases.join(", ")})`]) },
        { name: "role", label: ROLE_LABEL() },
      ],
      ok: t("Add"),
    });
    if (!values) return;
    await call("link_case_profile", { caseId: info.id, profileId: Number(values.profile), role: values.role.trim() });
    reload();
  };
  const open = async (p) => {
    await openProfileSheet(p);
    reload();
  };
  const role = async (p) => {
    const values = await dialog({ title: t("Role of {name}", { name: p.name }), fields: [{ name: "role", label: ROLE_LABEL(), value: p.role }], ok: t("Save") });
    if (!values) return;
    await call("link_case_profile", { caseId: info.id, profileId: p.id, role: values.role.trim() });
    reload();
  };
  const makeGlobal = async (p) => {
    const ok = await dialog({
      title: t("Make \"{name}\" global?", { name: p.name }),
      text: t("It shows on the Profiles page and any case can add it."),
      ok: t("Make global"),
    });
    if (!ok) return;
    await call("make_profile_global", { id: p.id });
    reload();
  };
  const remove = async (chosen) => {
    const only = chosen.filter((p) => !p.global && p.cases === 1);
    if (only.length) {
      const title = chosen.length === 1 ? t("Remove \"{name}\" from the case?", { name: chosen[0].name }) : tn(chosen.length, "Remove {n} profile from the case?", "Remove {n} profiles from the case?");
      const text =
        chosen.length === 1
          ? t("It exists only in this case, so it is deleted with its fields and files. Make it global first to keep it.")
          : t("These exist only in this case and are deleted with their fields and files: {names}. Make them global first to keep them.", { names: only.map((p) => p.name).join(", ") });
      if (!(await confirmDialog(title, text))) return;
    }
    for (const p of chosen) await call("remove_case_profile", { caseId: info.id, profileId: p.id });
    reload();
  };

  return html`<${TabTable}
    rows=${info.profiles}
    actions=${[
      [t("New profile"), create, "", "caseprofiles.new"],
      [t("Add existing…"), addExisting, "", "caseprofiles.add"],
    ]}
    bulk=${[[t("Remove from case"), remove, "danger", "caseprofiles.remove"]]}
    onOpen=${open}
    menu=${(p) => [
      { label: t("Edit"), help: "caseprofiles.edit", action: () => open(p) },
      { label: t("Role…"), help: "caseprofiles.role", action: () => role(p) },
      !p.global && { label: t("Make global"), help: "caseprofiles.global", action: () => makeGlobal(p) },
      "-",
      { label: t("Remove from case"), danger: true, help: "caseprofiles.remove", action: () => remove([p]) },
    ]}
    columns=${[
      { title: t("Name"), cell: (p) => html`<b>${p.name}</b>`, cls: "wide" },
      { title: t("Kind"), cell: (p) => p.kind, cls: "nowrap" },
      { title: t("Role"), cell: (p) => p.role || html`<span class="muted">-</span>`, cls: "nowrap" },
      { title: t("Scope"), cell: (p) => html`<span class="muted">${scopeLabel(p)}</span>`, cls: "nowrap" },
    ]}
    empty=${t("No profiles in this case yet. Make one for each person or company in it, or add one you already have.")} />`;
}

// -- templates ---------------------------------------------------------------------------

function CaseTemplates({ info }) {
  const templates = useLoad(() => call("list_templates", { caseId: info.id }), [info.id]);
  if (!templates) return null;

  const fill = async (chosen) => {
    await openFill({ templates: chosen.map((tpl) => tpl.id), case_id: info.id });
    reload();
  };
  const makeGlobal = async (tpl) => {
    const ok = await dialog({
      title: t("Make \"{name}\" global?", { name: tpl.name }),
      text: t("It shows on the Templates page and any case can use it."),
      ok: t("Make global"),
    });
    if (!ok) return;
    await call("make_template_global", { id: tpl.id });
    reload();
  };
  const remove = async (chosen) => {
    const local = chosen.filter((tpl) => tpl.local);
    if (!local.length) return toast(t("Only the templates of this case can be deleted here."));
    const title = local.length === 1 ? t("Delete the template \"{name}\"?", { name: local[0].name }) : tn(local.length, "Delete {n} template?", "Delete {n} templates?");
    let text = t("Its folder and examples are removed. Generated documents are kept.");
    if (local.length < chosen.length) text += `\n${t("Global templates stay.")}`;
    if (!(await confirmDialog(title, text))) return;
    for (const tpl of local) await call("delete_template", { id: tpl.id });
    reload();
  };

  return html`<${TabTable}
    rows=${templates}
    bulk=${[
      [t("Fill"), fill, "", "casetemplates.fill"],
      [t("Delete"), remove, "danger", "casetemplates.delete"],
    ]}
    onOpen=${(tpl) => fill([tpl])}
    menu=${(tpl) => [
      { label: t("Fill"), help: "casetemplates.fill", action: () => fill([tpl]) },
      tpl.local && { label: t("Make global"), help: "casetemplates.global", action: () => makeGlobal(tpl) },
      tpl.local && "-",
      tpl.local && { label: t("Delete"), danger: true, help: "casetemplates.delete", action: () => remove([tpl]) },
    ]}
    columns=${[
      { title: t("Name"), cell: (tpl) => html`<b>${tpl.name}</b>${tpl.description && html`<div class="muted small">${tpl.description}</div>`}`, cls: "wide" },
      { title: t("Kind"), cell: (tpl) => KIND_NAMES[tpl.kind] || tpl.kind, cls: "nowrap" },
      { title: t("Blanks"), cell: (tpl) => tpl.blanks, cls: "nowrap" },
      { title: t("Scope"), cell: (tpl) => html`<span class="muted">${tpl.local ? t("Only in this case") : t("Global")}</span>`, cls: "nowrap" },
    ]}
    empty=${t("No templates yet. Ask your AI assistant to make one from a document in this case.")} />`;
}

// -- fill sets, documents, activity ------------------------------------------------------------

function CaseFillSets({ info }) {
  const sets = useLoad(() => call("list_fill_sets", { caseId: info.id }), [info.id]);
  if (!sets) return null;

  const open = async (set) => {
    await openFill({ fill_set_id: set.id });
    reload();
  };
  const generate = async (chosen) => {
    await generateFillSets(chosen.map((s) => s.id));
    reload();
  };
  const remove = async (chosen) => {
    const title = chosen.length === 1 ? t("Delete the fill set \"{name}\"?", { name: chosen[0].name }) : tn(chosen.length, "Delete {n} fill set?", "Delete {n} fill sets?");
    if (!(await confirmDialog(title, t("Generated documents are kept.")))) return;
    for (const set of chosen) await call("delete_fill_set", { id: set.id });
    reload();
  };

  return html`<${TabTable}
    rows=${sets}
    bulk=${[
      [t("Generate"), generate, "", "fillsets.generate"],
      [t("Delete"), remove, "danger", "fillsets.delete"],
    ]}
    onOpen=${open}
    menu=${(set) => [
      { label: t("Open"), help: "fillsets.open", action: () => open(set) },
      { label: t("Generate"), help: "fillsets.generate", action: () => generate([set]) },
      "-",
      { label: t("Delete"), danger: true, help: "fillsets.delete", action: () => remove([set]) },
    ]}
    columns=${[
      { title: t("Name"), cell: (set) => html`<b>${set.name}</b>`, cls: "wide" },
      { title: t("Templates"), cell: (set) => set.templates.join(", ") },
      { title: t("Profiles"), cell: (set) => set.profiles.join(", ") },
      { title: t("Updated"), cell: (set) => html`<span class="muted">${when(set.updated_at)}</span>`, cls: "nowrap" },
    ]}
    empty=${t("No fill sets in this case yet.")} />`;
}

function CaseDocuments({ info }) {
  const docs = useLoad(() => call("list_documents", { caseId: info.id }), [info.id]);
  if (!docs) return null;

  const open = (doc) => call("open_documents", { ids: [doc.id] });
  const exportDocument = async (doc) => {
    const path = await call("export_document", { id: doc.id });
    if (path) toast(t("Saved to {path}", { path }), 4000);
  };
  const remove = async (chosen) => {
    const title = chosen.length === 1 ? t("Delete \"{name}\"?", { name: chosen[0].title }) : tn(chosen.length, "Delete {n} document?", "Delete {n} documents?");
    if (!(await confirmDialog(title, t("They are removed from the vault. Copies you exported stay where you saved them.")))) return;
    await call("delete_documents", { ids: chosen.map((d) => d.id) });
    reload();
  };

  return html`<${TabTable}
    rows=${docs}
    bulk=${[[t("Delete"), remove, "danger", "documents.delete"]]}
    onOpen=${open}
    menu=${(doc) => [
      { label: t("Open"), help: "documents.open", action: () => open(doc) },
      { label: t("Export…"), help: "documents.export", action: () => exportDocument(doc) },
      "-",
      { label: t("Delete"), danger: true, help: "documents.delete", action: () => remove([doc]) },
    ]}
    columns=${[
      { title: t("Title"), cell: (d) => d.title, cls: "wide" },
      { title: t("Template"), cell: (d) => html`<span class="mono">${d.template_id}</span>` },
      { title: t("Kind"), cell: (d) => (d.filename.endsWith(".pdf") ? "PDF" : "Word"), cls: "nowrap" },
      { title: t("Size"), cell: (d) => (d.pages ? tn(d.pages, "{n} page", "{n} pages") : "-"), cls: "nowrap" },
      { title: t("Created"), cell: (d) => html`<span class="muted">${when(d.created_at)}</span>`, cls: "nowrap" },
    ]}
    empty=${t("No documents in this case yet.")} />`;
}

function CaseActivity({ info }) {
  const rows = useLoad(() => call("list_activity", { caseId: info.id }), [info.id]);
  if (!rows) return null;
  return html`<${TabTable}
    rows=${rows}
    onOpen=${openActivity}
    menu=${(row) => [{ label: t("Open"), help: "activity.open", action: () => openActivity(row) }]}
    rowClass=${(r) => (r.status === "pending" ? "pending" : "")}
    columns=${[
      { title: t("Request"), cell: (r) => r.label, cls: "nowrap" },
      { title: t("Details"), cell: (r) => r.details, cls: "wide" },
      { title: t("Outcome"), cell: (r) => html`<span class=${`status ${r.status}`}>${r.outcome}</span>`, cls: "nowrap" },
      { title: t("Time"), cell: (r) => html`<span class="muted">${when(r.time)}</span>`, cls: "nowrap" },
    ]}
    empty=${t("Nothing from your AI assistant in this case yet.")} />`;
}
