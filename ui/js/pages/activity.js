import { call } from "../api.js";
import { openFill } from "../fill/form.js";
import { t } from "../i18n.js";
import { html, matches, useLoad, useRemembered, when } from "../lib.js";
import { processRequests } from "../requests.js";
import { toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Table } from "../ui/table.js";

export function Activity() {
  const rows = useLoad(() => call("list_activity"), []);
  const [selected, setSelected] = useRemembered("activity", null);
  if (!rows) return null;

  const pending = rows.filter((r) => r.status === "pending").length;
  const visible = rows.filter((r) => matches(r.label, r.details, r.outcome));

  const open = (id) => {
    const row = rows.find((r) => r.id === id);
    if (!row) return toast(t("Select a request"));
    if (row.status === "pending") return processRequests();
    if (row.kind === "fill" && row.fill_set_id) return openFill({ fill_set_id: row.fill_set_id });
    if (row.document_ids.length) return call("open_documents", { ids: row.document_ids });
    toast(t("Nothing to open for this request"));
  };

  return html`
    <${Page} title=${t("Activity")} subtitle=${pending ? t("{n} waiting", { n: pending }) : t("Nothing waiting")}
      actions=${[[t("Open"), () => open(selected)]]}>
      ${visible.length
        ? html`<${Table}
            rows=${visible}
            selected=${[selected]}
            onSelect=${([id]) => setSelected(id)}
            onOpen=${open}
            rowClass=${(r) => (r.status === "pending" ? "pending" : "")}
            columns=${[
              { title: t("Request"), cell: (r) => r.label, cls: "nowrap" },
              { title: t("Details"), cell: (r) => r.details },
              { title: t("Outcome"), cell: (r) => html`<span class=${`status ${r.status}`}>${r.outcome}</span>`, cls: "nowrap" },
              { title: t("Time"), cell: (r) => html`<span class="muted">${when(r.time)}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>${t("Nothing from your AI assistant yet.")}<br /><span class="small">${t("Requests for data and documents show up here.")}</span></${Empty}>`}
    </${Page}>`;
}
