import { call, withBusy } from "../api.js";
import { openFill } from "../fill/form.js";
import { t, tn } from "../i18n.js";
import { html, matches, reload, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table } from "../ui/table.js";

export function FillSets() {
  const sets = useLoad(() => call("list_fill_sets"), []);
  const [selected, setSelected] = useRemembered("fillsets", null);
  if (!sets) return null;

  const need = (action) => () => (selected ? action(selected) : toast(t("Select a fill set")));

  const generate = (id) =>
    withBusy(async () => {
      const ids = await call("generate_fill_set", { id });
      toast(tn(ids.length, "{n} document generated", "{n} documents generated"));
    });

  const exportSet = async (id, bundle) => {
    const path = await call("export_fill_set", { id, bundle });
    if (path) toast(t("Exported to {path}", { path }), 4000);
  };

  const importSet = async () => {
    const id = await call("import_fill_set");
    if (!id) return;
    setSelected(id);
    toast(t("Fill set imported"));
    reload();
  };

  const remove = async (id) => {
    const set = sets.find((s) => s.id === id);
    if (!(await confirmDialog(t("Delete the fill set \"{name}\"?", { name: set.name }), t("Generated documents are kept.")))) return;
    await call("delete_fill_set", { id });
    setSelected(null);
    reload();
  };

  const visible = sets.filter((s) => matches(s.name, ...s.templates, ...s.profiles));
  return html`
    <${Page} title=${t("Fill sets")} subtitle=${tn(sets.length, "{n} set", "{n} sets")} actions=${[
      [t("Open"), need((id) => openFill({ fill_set_id: id }))],
      [t("Generate"), need(generate)],
      [t("Export"), need((id) => exportSet(id, false))],
      [t("Export bundle"), need((id) => exportSet(id, true))],
      [t("Import"), importSet],
      [t("Delete"), need(remove), "danger"],
    ]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            selected=${[selected]}
            onSelect=${([id]) => setSelected(id)}
            onOpen=${(id) => openFill({ fill_set_id: id })}
            columns=${[
              { title: t("Name"), cell: (s) => html`<b>${s.name}</b>` },
              { title: t("Templates"), cell: (s) => s.templates.join(", ") },
              { title: t("Profiles"), cell: (s) => s.profiles.join(", ") },
              { title: t("Updated"), cell: (s) => html`<span class="muted">${when(s.updated_at)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>${t("No fill sets yet. Ask your AI assistant to fill in a template.")}</${Empty}>`}
    </${Page}>`;
}
