import { call, withBusy } from "../api.js";
import { openFill } from "../fill/form.js";
import { t, tn } from "../i18n.js";
import { html, matches, reload, useLoad, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table, useSelection } from "../ui/table.js";

export function FillSets() {
  const sets = useLoad(() => call("list_fill_sets"), []);
  const [picked, pick, setSelected] = useSelection("fillsets");
  if (!sets) return null;
  const selected = picked.filter((id) => sets.some((s) => s.id === id));

  /** Open acts on one fill set: the one clicked last. */
  const need = (action) => () => (selected.length ? action(selected.at(-1)) : toast(t("Select a fill set")));
  const needAll = (action) => () => (selected.length ? action(selected) : toast(t("Select a fill set")));

  const generate = (ids) =>
    withBusy(async () => {
      let made = 0;
      for (const id of ids) made += (await call("generate_fill_set", { id })).length;
      toast(tn(made, "{n} document generated", "{n} documents generated"));
    });

  const remove = async (ids) => {
    const title =
      ids.length === 1
        ? t("Delete the fill set \"{name}\"?", { name: sets.find((s) => s.id === ids[0]).name })
        : tn(ids.length, "Delete {n} fill set?", "Delete {n} fill sets?");
    if (!(await confirmDialog(title, t("Generated documents are kept.")))) return;
    for (const id of ids) await call("delete_fill_set", { id });
    setSelected([]);
    reload();
  };

  const visible = sets.filter((s) => matches(s.name, ...s.templates, ...s.profiles));
  return html`
    <${Page} title=${t("Fill sets")} subtitle=${tn(sets.length, "{n} set", "{n} sets")} actions=${[
      [t("Open"), need((id) => openFill({ fill_set_id: id })), "", "fillsets.open"],
      [t("Generate"), needAll(generate), "", "fillsets.generate"],
      [t("Delete"), needAll(remove), "danger", "fillsets.delete"],
    ]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            selected=${selected}
            onPick=${pick}
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
