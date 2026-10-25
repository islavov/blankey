import { call } from "../api.js";
import { t, tn } from "../i18n.js";
import { html, matches, reload, useLoad, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table, useSelection } from "../ui/table.js";

export function Documents() {
  const docs = useLoad(() => call("list_documents"), []);
  const [picked, pick, setSelected] = useSelection("documents");
  if (!docs) return null;
  const selected = picked.filter((id) => docs.some((d) => d.id === id));

  const need = (action) => () => (selected.length ? action(selected) : toast(t("Select a document")));
  const open = (ids) => call("open_documents", { ids });

  const exportDocument = async ([id]) => {
    const path = await call("export_document", { id });
    if (path) toast(t("Saved to {path}", { path }), 4000);
  };

  const remove = async (ids) => {
    const chosen = docs.filter((d) => ids.includes(d.id));
    const text = chosen.length === 1 ? t("Delete \"{name}\"?", { name: chosen[0].title }) : tn(chosen.length, "Delete {n} document?", "Delete {n} documents?");
    if (!(await confirmDialog(text, t("They are removed from the vault. Copies you exported stay where you saved them.")))) return;
    await call("delete_documents", { ids });
    setSelected([]);
    reload();
  };

  const visible = docs.filter((d) => matches(d.title, d.template_id, d.filename));
  return html`
    <${Page} title=${t("Documents")} subtitle=${t("Encrypted in the vault")} actions=${[
      [t("Open"), need(open), "", "documents.open"],
      [t("Export"), need(exportDocument), "", "documents.export"],
      [t("Delete"), need(remove), "danger", "documents.delete"],
    ]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            selected=${selected}
            onPick=${pick}
            onOpen=${(id) => open([id])}
            columns=${[
              { title: t("Title"), cell: (d) => d.title },
              { title: t("Template"), cell: (d) => html`<span class="mono">${d.template_id}</span>` },
              { title: t("Kind"), cell: (d) => (d.filename.endsWith(".pdf") ? "PDF" : "Word"), cls: "nowrap" },
              { title: t("Size"), cell: (d) => (d.pages ? tn(d.pages, "{n} page", "{n} pages") : "-"), cls: "nowrap" },
              { title: t("Created"), cell: (d) => html`<span class="muted">${when(d.created_at)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>${t("No documents yet. Documents are generated from fill sets.")}</${Empty}>`}
    </${Page}>`;
}
