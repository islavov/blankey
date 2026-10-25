import { useEffect, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { openFill } from "../fill/form.js";
import { t, tn } from "../i18n.js";
import { html, KIND_NAMES, matches, reload, state, useLoad, useRemembered, useStore } from "../lib.js";
import { decodePreview, Preview } from "../preview.js";
import { openReview } from "../review.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { keepText, useSelection } from "../ui/table.js";

const copyId = async (id) => {
  await call("copy_text", { text: id });
  toast(t("Copied {id}", { id }));
};

const READINESS = {
  ready: () => t("Ready to bind"),
  blanks: () => t("Has blanks"),
  needs_ai: () => t("Needs the assistant"),
  scan: () => t("Scanned, no text"),
  unreadable: () => t("Cannot be read"),
};

/** The categories of src-tauri/src/library/categories.yaml. */
const CATEGORIES = {
  lease: () => t("Leases"),
  employment: () => t("Employment"),
  power_of_attorney: () => t("Powers of attorney"),
  sale: () => t("Sales"),
  loan: () => t("Loans"),
  nda: () => t("Confidentiality"),
  services: () => t("Services"),
  offer: () => t("Offers"),
  invoice: () => t("Invoices"),
  declaration: () => t("Declarations"),
  application: () => t("Applications"),
  claim: () => t("Claims"),
  minutes: () => t("Minutes and resolutions"),
  notice: () => t("Notices"),
  certificate: () => t("Certificates"),
  agreement: () => t("Agreements"),
  contract: () => t("Contracts"),
};

export function Templates() {
  const [view, setView] = useRemembered("templates-view", "templates");
  const switcher = html`
    <div class="filters" data-help="templates.view">
      ${[["templates", t("Templates")], ["folders", t("Folders")]].map(([id, label]) => html`
        <button class=${`chip ${view === id ? "active" : ""}`} onClick=${() => setView(id)}>${label}</button>`)}
    </div>`;
  return view === "folders" ? html`<${Folders} switcher=${switcher} />` : html`<${TemplateList} switcher=${switcher} />`;
}

function TemplateList({ switcher }) {
  const templates = useLoad(() => call("list_templates"), []);
  const [picked, pick, setSelected] = useSelection("templates");
  if (!templates) return null;

  const visible = templates.filter((tpl) => matches(tpl.name, tpl.id, tpl.description));
  const order = visible.map((tpl) => tpl.id);
  const chosen = picked.filter((id) => order.includes(id));
  const selected = chosen.length ? chosen : order.slice(0, 1);
  // the detail shows the template clicked last
  const current = selected.at(-1);
  const need = (action) => () => (current ? action(current) : toast(t("Select a template")));
  const needAll = (action) => () => (selected.length ? action(selected) : toast(t("Select a template")));

  const remove = async (ids) => {
    const chosen = templates.filter((x) => ids.includes(x.id));
    const usedBy = [...new Set(chosen.flatMap((x) => x.used_by))];
    let text = t("Its folder and examples are removed. Generated documents are kept.");
    if (usedBy.length) text += ` ${t("Fill sets using it will no longer open: {names}.", { names: usedBy.join(", ") })}`;
    const title =
      chosen.length === 1
        ? t("Delete the template \"{name}\"?", { name: chosen[0].name })
        : tn(chosen.length, "Delete {n} template?", "Delete {n} templates?");
    if (!(await confirmDialog(title, text))) return;
    for (const tpl of chosen) await call("delete_template", { id: tpl.id });
    setSelected([]);
    reload();
  };

  return html`
    <${Page} title=${t("Templates")} subtitle=${tn(templates.length, "{n} template", "{n} templates")} actions=${[
      [t("Fill"), needAll((ids) => openFill({ templates: ids })), "", "templates.fill"],
      [t("Copy ID"), need(copyId), "", "templates.copyid"],
      [t("Delete"), needAll(remove), "danger", "templates.delete"],
    ]}>
      ${switcher}
      ${templates.length
        ? html`
            <div class="split">
              <div class="list" data-help="templates.list">
                ${visible.map(
                  (tpl) => html`<${Card} key=${tpl.id} tpl=${tpl} active=${selected.includes(tpl.id)} onSelect=${(e) => pick(e, tpl.id, order, selected)} />`
                )}
              </div>
              <div class="detail">${current && html`<${TemplateDetail} template=${templates.find((tpl) => tpl.id === current)} />`}</div>
            </div>`
        : html`<${Empty}>
            ${t("No templates yet. Ask your AI assistant to make one from a document.")}<br />
            <span class="small mono selectable">${state.status.data_dir}/templates</span>
          </${Empty}>`}
    </${Page}>`;
}

function Card({ tpl, active, onSelect }) {
  const meta = [KIND_NAMES[tpl.kind] || tpl.kind, tn(tpl.blanks, "{n} blank", "{n} blanks")];
  if (tpl.used_by.length) meta.push(tn(tpl.used_by.length, "{n} fill set", "{n} fill sets"));
  return html`
    <div class=${`card ${active ? "active" : ""}`} onMouseDown=${keepText} onClick=${onSelect} onDblClick=${() => openFill({ templates: [tpl.id] })}>
      <div class="title">${tpl.name}${tpl.stale && html` <span class="status stale" title=${tpl.stale}>${t("Out of date")}</span>`}</div>
      <div class="muted small">${meta.join("  ·  ")}</div>
      ${tpl.description && html`<div class="desc muted">${tpl.description}</div>`}
    </div>`;
}

/** Keeps showing the previous template until the next one's preview is loaded and decoded. */
function TemplateDetail({ template }) {
  const { revision } = useStore();
  const [shown, setShown] = useState(null);
  useEffect(() => {
    let live = true;
    call("template_preview", { id: template.id })
      .then(async (data) => {
        await decodePreview(data.preview);
        if (live) setShown({ template, data });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [template.id, revision]);
  if (!shown) return null;

  const { template: tpl, data } = shown;
  const info = [tn(data.blanks, "{n} blank", "{n} blanks"), t("roles: {roles}", { roles: Object.keys(data.roles).join(", ") || "-" })];
  if (tpl.used_by.length) info.push(t("used by: {names}", { names: tpl.used_by.join(", ") }));
  return html`
    <div class="detail-head">
      <div style="flex:1;min-width:0">
        <h3>${tpl.name}</h3>
        <div class="muted small"><span class="mono selectable">${tpl.id}</span>${info.map((part) => ` \u00a0·\u00a0 ${part}`)}</div>
      </div>
      <button data-help="templates.copyid" onClick=${() => copyId(tpl.id)}>${t("Copy ID")}</button>
      <button data-help="templates.folder" onClick=${() => call("show_template_folder", { id: tpl.id })}>${t("Show in Finder")}</button>
    </div>
    <${Preview} key=${tpl.id} preview=${data.preview} interactive=${false} style="margin-top:12px" help="templates.preview" />
    <h4 data-help="templates.bindings">${t("Bindings")}</h4>
    <table class="kv selectable" data-help="templates.bindings">
      <tbody>
        ${Object.entries(data.fields).map(
          ([name, expr]) => html`
            <tr key=${name}>
              <td>
                <span class="mono">${name}</span>
                ${data.labels[name] && html`<div class="muted small">${data.labels[name]}</div>`}
              </td>
              <td class="mono">${expr || html`<span class="muted">${t("(asked at fill time)")}</span>`}</td>
            </tr>`
        )}
      </tbody>
    </table>
    ${data.examples.length > 0 &&
    html`<h4>${t("Examples")}</h4>${data.examples.map((e) => html`<span class="pill">${e}</span>`)}`}`;
}

/** The user's own folders: Blankey indexes them in place and the assistant makes templates from their files. */
function Folders({ switcher }) {
  const roots = useLoad(() => call("library_roots"), []);
  const files = useLoad(() => call("library_files"), []);
  const [picked, setPicked] = useRemembered("folders-file", null);
  if (!roots || !files) return null;

  const add = async () => {
    const added = await call("add_library_root");
    if (!added) return;
    toast(tn(added.report.added, "Indexed {n} file", "Indexed {n} files"));
    reload();
  };
  const remove = async (root) => {
    const title = t("Remove the folder \"{path}\"?", { path: root.path });
    const text = t("Its files stay where they are. Templates made from them keep working from their own copy.");
    if (!(await confirmDialog(title, text, t("Remove")))) return;
    await call("remove_library_root", { id: root.id });
    reload();
  };
  const rescan = async () => {
    await call("rescan_library");
    reload();
  };
  const reveal = (file) => call("show_library_file", { id: file.id });

  const visible = files.filter((f) => matches(f.title, f.rel_path, f.category && categoryName(f.category), ...f.tags));
  const current = visible.find((f) => f.id === picked) || visible[0];

  return html`
    <${Page} title=${t("Templates")} subtitle=${tn(roots.length, "{n} folder", "{n} folders")} actions=${[
      [t("Add folder…"), add, "primary", "folders.add"],
      [t("Rescan"), rescan, "", "folders.rescan"],
    ]}>
      ${switcher}
      ${roots.length
        ? html`
            <div class="split">
              <div class="list" data-help="folders.files">
                <div class="roots">
                  ${roots.map((root) => html`
                    <div class="root" key=${root.id} data-help="folders.root">
                      <div class="root-path" title=${root.path}>
                        <div class="title">${root.path.split(/[\\/]/).filter(Boolean).pop()}</div>
                        <div class="muted small">${tn(root.files, "{n} file", "{n} files")}</div>
                      </div>
                      <button class="danger" onClick=${() => remove(root)}>${t("Remove")}</button>
                    </div>`)}
                </div>
                ${visible.map((f) => html`
                  <div key=${f.id} class=${`card library-file ${current?.id === f.id ? "active" : ""}`} onMouseDown=${keepText}
                    onClick=${() => setPicked(f.id)} onDblClick=${() => reveal(f)}>
                    <div class="title">${fileName(f)}</div>
                    <div class="muted small">${fileMeta(f).join("  ·  ")}</div>
                  </div>`)}
                ${!visible.length && html`<div class="muted small list-note">${files.length ? t("No files match.") : t("Every file in these folders is a template already.")}</div>`}
              </div>
              <div class="detail">${current && html`<${FileDetail} file=${current} onReveal=${() => reveal(current)} />`}</div>
            </div>`
        : html`<${Empty}>
            ${t("Add the folders where you keep forms and model contracts. Blankey reads them in place; your AI assistant turns a file into a template when you need it.")}
          </${Empty}>`}
    </${Page}>`;
}

const categoryName = (category) => (CATEGORIES[category] || (() => category))();
const fileName = (f) => f.title || f.rel_path.split("/").pop();

function fileMeta(f) {
  const meta = [f.kind === "pdf" ? "PDF" : KIND_NAMES[f.kind] || f.kind, (READINESS[f.readiness] || (() => f.readiness))()];
  if (f.category) meta.push(categoryName(f.category));
  return meta;
}

/** Keeps showing the previous file until the next one's preview is loaded and decoded. */
function FileDetail({ file, onReveal }) {
  const { revision } = useStore();
  const [shown, setShown] = useState(null);
  useEffect(() => {
    let live = true;
    call("library_preview", { id: file.id })
      .then(async (preview) => {
        await decodePreview(preview);
        if (live) setShown({ file, preview });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [file.id, revision]);
  if (!shown) return null;

  const { file: f, preview } = shown;
  const info = fileMeta(f);
  if (f.fields) info.push(tn(f.fields, "{n} field", "{n} fields"));
  if (f.blanks) info.push(tn(f.blanks, "{n} blank", "{n} blanks"));
  return html`
    <div class="detail-head">
      <div style="flex:1;min-width:0">
        <h3>${fileName(f)}</h3>
        <div class="muted small"><span class="mono selectable">${f.rel_path}</span>${info.map((part) => ` \u00a0·\u00a0 ${part}`)}</div>
      </div>
      <button class="check-pii" data-help="folders.pii" onClick=${() => openReview(f)}>${t("Personal data…")}</button>
      <button data-help="folders.open" onClick=${() => call("open_library_file", { id: f.id })}>${t("Open")}</button>
      <button data-help="folders.reveal" onClick=${onReveal}>${t("Show in Finder")}</button>
    </div>
    <${Preview} key=${f.id} preview=${preview} interactive=${false} style="margin-top:12px" help="templates.preview" />`;
}
