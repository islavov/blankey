import { useEffect, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { openFill } from "../fill/form.js";
import { html, KIND_NAMES, matches, plural, reload, state, useLoad, useRemembered, useStore } from "../lib.js";
import { decodePreview, Preview } from "../preview.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";

const copyId = async (id) => {
  await call("copy_text", { text: id });
  toast(`Copied ${id}`);
};

export function Templates() {
  const templates = useLoad(() => call("list_templates"), []);
  const [selected, setSelected] = useRemembered("templates", null);
  if (!templates) return null;

  const visible = templates.filter((t) => matches(t.name, t.id, t.description));
  const current = visible.some((t) => t.id === selected) ? selected : visible[0]?.id;
  const need = (action) => () => (current ? action(current) : toast("Select a template"));

  const remove = async (id) => {
    const t = templates.find((x) => x.id === id);
    let text = "Its folder and examples are removed. Generated documents are kept.";
    if (t.used_by.length) text += ` Fill sets using it will no longer open: ${t.used_by.join(", ")}.`;
    if (!(await confirmDialog(`Delete the template "${t.name}"?`, text))) return;
    await call("delete_template", { id });
    setSelected(null);
    reload();
  };

  return html`
    <${Page} title="Templates" subtitle=${plural(templates.length, "template")} actions=${[
      ["Fill", need((id) => openFill({ templates: [id] }))],
      ["Copy ID", need(copyId)],
      ["Show in Finder", () => call("show_template_folder", { id: current })],
      ["Delete", need(remove), "danger"],
    ]}>
      ${templates.length
        ? html`
            <div class="split">
              <div class="list">
                ${visible.map((t) => html`<${Card} key=${t.id} t=${t} active=${t.id === current} onSelect=${() => setSelected(t.id)} />`)}
              </div>
              <div class="detail">${current && html`<${TemplateDetail} template=${templates.find((t) => t.id === current)} />`}</div>
            </div>`
        : html`<${Empty}>
            No templates yet. Ask your AI assistant to make one from a document.<br />
            <span class="small mono selectable">${state.status.data_dir}/templates</span>
          </${Empty}>`}
    </${Page}>`;
}

function Card({ t, active, onSelect }) {
  const meta = [KIND_NAMES[t.kind] || t.kind, plural(t.blanks, "blank")];
  if (t.used_by.length) meta.push(plural(t.used_by.length, "fill set"));
  return html`
    <div class=${`card ${active ? "active" : ""}`} onClick=${onSelect} onDblClick=${() => openFill({ templates: [t.id] })}>
      <div class="title">${t.name}</div>
      <div class="muted small">${meta.join("  ·  ")}</div>
      ${t.description && html`<div class="desc muted">${t.description}</div>`}
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

  const { template: t, data } = shown;
  const info = [plural(data.blanks, "blank"), `roles: ${Object.keys(data.roles).join(", ") || "-"}`];
  if (t.used_by.length) info.push(`used by: ${t.used_by.join(", ")}`);
  return html`
    <div class="detail-head">
      <div style="flex:1;min-width:0">
        <h3>${t.name}</h3>
        <div class="muted small"><span class="mono selectable">${t.id}</span>${info.map((part) => ` \u00a0·\u00a0 ${part}`)}</div>
      </div>
      <button onClick=${() => copyId(t.id)}>Copy ID</button>
    </div>
    <${Preview} key=${t.id} preview=${data.preview} interactive=${false} style="margin-top:12px" />
    <h4>Bindings</h4>
    <table class="kv selectable">
      <tbody>
        ${Object.entries(data.fields).map(
          ([name, expr]) => html`
            <tr key=${name}>
              <td>
                <span class="mono">${name}</span>
                ${data.labels[name] && html`<div class="muted small">${data.labels[name]}</div>`}
              </td>
              <td class="mono">${expr || html`<span class="muted">(asked at fill time)</span>`}</td>
            </tr>`
        )}
      </tbody>
    </table>
    ${data.examples.length > 0 &&
    html`<h4>Examples</h4>${data.examples.map((e) => html`<span class="pill">${e}</span>`)}`}`;
}
