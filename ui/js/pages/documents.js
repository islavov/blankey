import { call } from "../api.js";
import { html, matches, reload, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table } from "../ui/table.js";

export function Documents() {
  const docs = useLoad(() => call("list_documents"), []);
  const [selected, setSelected] = useRemembered("documents", []);
  if (!docs) return null;

  const need = (action) => () => (selected.length ? action(selected) : toast("Select a document"));
  const open = (ids) => call("open_documents", { ids });

  const exportDocument = async ([id]) => {
    const path = await call("export_document", { id });
    if (path) toast(`Saved to ${path}`, 4000);
  };

  const remove = async (ids) => {
    const chosen = docs.filter((d) => ids.includes(d.id));
    const text = chosen.length === 1 ? `Delete "${chosen[0].title}"?` : `Delete ${chosen.length} documents?`;
    if (!(await confirmDialog(text, "They are removed from the vault. Copies you exported stay where you saved them."))) return;
    await call("delete_documents", { ids });
    setSelected([]);
    reload();
  };

  const visible = docs.filter((d) => matches(d.title, d.template_id, d.filename));
  return html`
    <${Page} title="Documents" subtitle="Encrypted in the vault" actions=${[
      ["Open", need(open)],
      ["Export", need(exportDocument)],
      ["Delete", need(remove), "danger"],
    ]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            multi
            selected=${selected}
            onSelect=${setSelected}
            onOpen=${(id) => open([id])}
            columns=${[
              { title: "Title", cell: (d) => d.title },
              { title: "Template", cell: (d) => html`<span class="mono">${d.template_id}</span>` },
              { title: "Kind", cell: (d) => (d.filename.endsWith(".pdf") ? "PDF" : "Word"), cls: "nowrap" },
              { title: "Pages", cell: (d) => d.pages || "-", cls: "nowrap" },
              { title: "Created", cell: (d) => html`<span class="muted">${when(d.created_at)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>No documents yet. Documents are generated from fill sets.</${Empty}>`}
    </${Page}>`;
}
