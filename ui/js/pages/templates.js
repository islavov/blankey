import { useEffect, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { openFill } from "../fill/form.js";
import { t, tn } from "../i18n.js";
import { html, KIND_NAMES, matches, reload, state, useLoad, useStore } from "../lib.js";
import { decodePreview, Preview } from "../preview.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { keepText, useSelection } from "../ui/table.js";

const copyId = async (id) => {
  await call("copy_text", { text: id });
  toast(t("Copied {id}", { id }));
};

export function Templates() {
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
      [t("Fill"), needAll((ids) => openFill({ templates: ids }))],
      [t("Copy ID"), need(copyId)],
      [t("Show in Finder"), () => call("show_template_folder", { id: current })],
      [t("Delete"), needAll(remove), "danger"],
    ]}>
      ${templates.length
        ? html`
            <div class="split">
              <div class="list">
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
      <div class="title">${tpl.name}</div>
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
      <button onClick=${() => copyId(tpl.id)}>${t("Copy ID")}</button>
    </div>
    <${Preview} key=${tpl.id} preview=${data.preview} interactive=${false} style="margin-top:12px" />
    <h4>${t("Bindings")}</h4>
    <table class="kv selectable">
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
