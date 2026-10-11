import { call, withBusy } from "../api.js";
import { openFill } from "../fill/form.js";
import { html, matches, plural, reload, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table } from "../ui/table.js";

export function FillSets() {
  const sets = useLoad(() => call("list_fill_sets"), []);
  const [selected, setSelected] = useRemembered("fillsets", null);
  if (!sets) return null;

  const need = (action) => () => (selected ? action(selected) : toast("Select a fill set"));

  const generate = (id) =>
    withBusy(async () => {
      const ids = await call("generate_fill_set", { id });
      toast(`${plural(ids.length, "document")} generated`);
    });

  const exportSet = async (id, bundle) => {
    const path = await call("export_fill_set", { id, bundle });
    if (path) toast(`Exported to ${path}`, 4000);
  };

  const importSet = async () => {
    const id = await call("import_fill_set");
    if (!id) return;
    setSelected(id);
    toast("Fill set imported");
    reload();
  };

  const remove = async (id) => {
    const set = sets.find((s) => s.id === id);
    if (!(await confirmDialog(`Delete the fill set "${set.name}"?`, "Generated documents are kept."))) return;
    await call("delete_fill_set", { id });
    setSelected(null);
    reload();
  };

  const visible = sets.filter((s) => matches(s.name, ...s.templates, ...s.profiles));
  return html`
    <${Page} title="Fill sets" subtitle=${plural(sets.length, "set")} actions=${[
      ["Open", need((id) => openFill({ fill_set_id: id }))],
      ["Generate", need(generate)],
      ["Export", need((id) => exportSet(id, false))],
      ["Export bundle", need((id) => exportSet(id, true))],
      ["Import", importSet],
      ["Delete", need(remove), "danger"],
    ]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            selected=${[selected]}
            onSelect=${([id]) => setSelected(id)}
            onOpen=${(id) => openFill({ fill_set_id: id })}
            columns=${[
              { title: "Name", cell: (s) => html`<b>${s.name}</b>` },
              { title: "Templates", cell: (s) => s.templates.join(", ") },
              { title: "Profiles", cell: (s) => s.profiles.join(", ") },
              { title: "Updated", cell: (s) => html`<span class="muted">${when(s.updated_at)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>No fill sets yet. Ask your AI assistant to fill in a template.</${Empty}>`}
    </${Page}>`;
}
