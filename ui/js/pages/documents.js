import { call } from "../api.js";
import { html, matches, megabytes, reload, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table } from "../ui/table.js";

const extension = (name) => (name.includes(".") ? name.split(".").pop().toUpperCase() : "File");

/** Generated documents and files collected through links, as one list; ids are "d:<id>" and "c:<id>". */
function rows(docs, collected) {
  return [
    ...docs.map((d) => ({
      id: `d:${d.id}`,
      title: d.title,
      source: "Generated",
      details: html`<span class="mono">${d.template_id}</span>`,
      search: [d.title, d.template_id, d.filename],
      kind: d.filename.endsWith(".pdf") ? "PDF" : "Word",
      size: d.pages ? `${d.pages} p.` : "-",
      created_at: d.created_at,
    })),
    ...collected.map((f) => ({
      id: `c:${f.id}`,
      title: f.name,
      source: "Collected",
      details: [f.link_title, f.profile_name].filter(Boolean).join(" · "),
      search: [f.name, f.link_title, f.profile_name],
      kind: extension(f.name),
      size: megabytes(f.size),
      created_at: f.created_at,
    })),
  ].sort((a, b) => String(b.created_at).localeCompare(String(a.created_at)));
}

const split = (ids, prefix) => ids.filter((id) => id.startsWith(prefix)).map((id) => Number(id.slice(2)));

export function Documents() {
  const docs = useLoad(() => Promise.all([call("list_documents"), call("list_collected_files")]), []);
  const [selected, setSelected] = useRemembered("documents", []);
  if (!docs) return null;
  const all = rows(...docs);

  const need = (action) => () => (selected.length ? action(selected) : toast("Select a document"));
  const open = async (ids) => {
    if (split(ids, "d:").length) await call("open_documents", { ids: split(ids, "d:") });
    if (split(ids, "c:").length) await call("open_collected_files", { ids: split(ids, "c:") });
  };

  const exportDocument = async ([id]) => {
    const command = id.startsWith("c:") ? "export_collected_file" : "export_document";
    const path = await call(command, { id: Number(id.slice(2)) });
    if (path) toast(`Saved to ${path}`, 4000);
  };

  const remove = async (ids) => {
    const chosen = all.filter((d) => ids.includes(d.id));
    const text = chosen.length === 1 ? `Delete "${chosen[0].title}"?` : `Delete ${chosen.length} documents?`;
    if (!(await confirmDialog(text, "They are removed from the vault. Copies you exported stay where you saved them."))) return;
    if (split(ids, "d:").length) await call("delete_documents", { ids: split(ids, "d:") });
    if (split(ids, "c:").length) await call("delete_collected_files", { ids: split(ids, "c:") });
    setSelected([]);
    reload();
  };

  const visible = all.filter((d) => matches(...d.search));
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
              { title: "Source", cell: (d) => d.source, cls: "nowrap" },
              { title: "Details", cell: (d) => d.details },
              { title: "Kind", cell: (d) => d.kind, cls: "nowrap" },
              { title: "Size", cell: (d) => d.size, cls: "nowrap" },
              { title: "Created", cell: (d) => html`<span class="muted">${when(d.created_at)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>No documents yet. Documents are generated from fill sets or collected through links.</${Empty}>`}
    </${Page}>`;
}
