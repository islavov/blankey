import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call, invoke } from "../api.js";
import { t, tn } from "../i18n.js";
import { html, matches, megabytes, reload, state, useLoad, useRemembered, useStore, when } from "../lib.js";
import { openSubmission } from "../submission.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { openSheet, SheetHead } from "../ui/sheet.js";
import { Empty, Page } from "../ui/page.js";
import { Select } from "../ui/select.js";
import { Table, useSelection } from "../ui/table.js";
import { relaySignIn } from "./connections.js";

const linkState = (state) => ({ open: t("Open"), expired: t("Expired"), revoked: t("Revoked"), full: t("Full") })[state] || state;
const DAYS = [1, 7, 30];

const copyLink = (url) => call("copy_text", { text: url }).then(() => toast(t("Link copied.")));
const shortUrl = (url) => url.replace(/^https?:\/\//, "").replace(/#.*$/, "");

const FILTERS = [
  ["all", () => t("All"), () => true],
  ["pending", () => t("To review"), (l) => l.pending > 0],
  ["open", () => t("Open"), (l) => l.state === "open"],
  ["closed", () => t("Closed"), (l) => l.state !== "open"],
];

const submissionState = (status) =>
  ({ pending: t("To review"), saved: t("Saved"), discarded: t("Discarded"), damaged: t("Damaged") })[status] || status;

/** One line per link; the link's fields and full URL are in the panel beside the table, which leaves only the
 * title, the counts and the status when it is open. */
function columns(narrow) {
  return [
    {
      title: t("Title"),
      cell: (l) => html`<div class="link-title">${l.title}</div><div class="mono muted small">${shortUrl(l.url)}</div>`,
      cls: "wide",
    },
    !narrow && { title: t("Profile"), cell: (l) => l.profile_name || "", cls: "nowrap" },
    {
      title: t("Received"),
      cell: (l) => html`${l.submissions}/${l.max_submissions}
        ${l.pending > 0 && html` <span class="status pending">${t("{n} to review", { n: l.pending })}</span>`}`,
      cls: "nowrap",
    },
    !narrow && { title: t("Last received"), cell: (l) => html`<span class="muted">${when(l.last_received) || "-"}</span>`, cls: "nowrap" },
    !narrow && { title: t("Expires"), cell: (l) => html`<span class="muted">${when(l.expires_at)}</span>`, cls: "nowrap" },
    { title: t("Status"), cell: (l) => html`<span class=${`status ${l.state}`}>${linkState(l.state)}</span>`, cls: "nowrap" },
  ].filter(Boolean);
}

/** Links that ask someone else for data: they fill a form on the relay, sealed to this computer. */
export function Links() {
  const { relay } = useStore();
  const links = useLoad(() => call("list_links"), []);
  const [picked, pick, setSelected] = useSelection("links");
  const [filter, setFilter] = useRemembered("links.filter", "all");
  if (!links || !relay) return null;

  const chosen = links.filter((l) => picked.includes(l.link_id));
  // what came in shows beside the table for a single link
  const link = chosen.length === 1 ? chosen[0] : null;
  const last = links.find((l) => l.link_id === picked.at(-1));
  const need = (action) => () => (last ? action(last) : toast(t("Select a link")));
  const needAll = (action) => () => (chosen.length ? action(chosen) : toast(t("Select a link")));

  const create = async () => {
    await openLinkSheet();
    reload();
  };

  const remove = async (chosen) => {
    const text = t("The link stops working and is removed, with what it received that you have not saved. Values and files you saved stay.");
    const title =
      chosen.length === 1 ? t("Delete \"{name}\"?", { name: chosen[0].title }) : tn(chosen.length, "Delete {n} link?", "Delete {n} links?");
    if (!(await confirmDialog(title, text))) return;
    for (const l of chosen) await call("delete_link", { linkId: l.link_id });
    setSelected([]);
    reload();
  };

  const revoke = async (chosen) => {
    const text = t("The form stops taking answers. What you already received stays here.");
    const title =
      chosen.length === 1 ? t("Revoke \"{name}\"?", { name: chosen[0].title }) : tn(chosen.length, "Revoke {n} link?", "Revoke {n} links?");
    if (!(await confirmDialog(title, text, t("Revoke")))) return;
    for (const l of chosen) await call("revoke_link", { linkId: l.link_id });
    reload();
  };

  if (!relay.signed_in && !links.length) {
    return html`
      <${Page} title=${t("Collection links")} subtitle=${t("Ask someone for their data")}>
        <${Empty}>
          ${t("Sign in to Blankey Online to make collection links.")}<br />
          <span class="small">${t("The person you send a link to fills in a form. What they send is encrypted in their browser and only opens on this computer.")}</span>
          <div style="margin-top:14px"><button class="primary" onClick=${() => relaySignIn().catch(() => {})}>${t("Sign in")}</button></div>
        </${Empty}>
      </${Page}>`;
  }

  const [, , shown] = FILTERS.find(([id]) => id === filter) || FILTERS[0];
  const rows = links
    .map((l) => ({ ...l, id: l.link_id }))
    .filter(shown)
    .filter((l) => matches(l.title, l.profile_name, l.link_id, linkState(l.state), ...l.fields));
  const pending = links.reduce((n, l) => n + l.pending, 0);
  return html`
    <${Page} title=${t("Collection links")} subtitle=${pending ? tn(pending, "{n} submission to review", "{n} submissions to review") : tn(links.length, "{n} link", "{n} links")} actions=${[
      [t("New link"), create, "primary"],
      [t("Copy link"), need((l) => copyLink(l.url))],
      [t("Revoke"), needAll(revoke)],
      [t("Delete"), needAll(remove), "danger"],
    ]}>
      <div class=${`links-split ${link ? "with-side" : ""}`}>
        <div class="links-main">
          <div class="filters">
            ${FILTERS.map(([id, label, test]) => html`
              <button class=${`chip ${filter === id ? "active" : ""}`} onClick=${() => setFilter(id)}>
                ${label()} <span class="count">${links.filter(test).length}</span>
              </button>`)}
          </div>
          ${rows.length
            ? html`<${Table}
                rows=${rows}
                selected=${picked}
                onPick=${pick}
                onOpen=${(id) => copyLink(links.find((l) => l.link_id === id).url)}
                rowClass=${(l) => (l.pending ? "pending" : "")}
                columns=${columns(!!link)} />`
            : html`<${Empty}>${links.length ? t("No links match.") : html`${t("No collection links yet.")}<br /><span class="small">${t("Make one to ask someone for the data of a profile, with files.")}</span>`}</${Empty}>`}
        </div>
        ${link && html`<${Submissions} key=${link.link_id} link=${link} close=${() => setSelected([])} />`}
      </div>
    </${Page}>`;
}

/** What came in for one link, beside the table; each opens again until it is discarded. */
function Submissions({ link, close }) {
  const submissions = useLoad(() => call("list_submissions", { linkId: link.link_id }), [link.link_id]);
  const open = async (sid) => {
    await openSubmission({ sid });
    reload();
  };
  return html`
    <aside class="side submissions">
      <div class="side-head">
        <h3>${link.title}</h3>
        <button class="ghost close" title=${t("Close")} onClick=${close}>✕</button>
      </div>
      <div class="side-link">
        <span class="mono selectable">${shortUrl(link.url)}</span>
        <button class="link" onClick=${() => copyLink(link.url)}>${t("Copy")}</button>
      </div>
      <div class="muted small">
        ${t("{count} received · {state} · expires {date}", { count: `${link.submissions}/${link.max_submissions}`, state: linkState(link.state), date: when(link.expires_at) })}
      </div>
      ${link.profile_name && html`<div class="muted small">${t("Saves into {profile}", { profile: link.profile_name })}</div>`}
      <h4>${t("Fields")}</h4>
      <div class="small">${link.fields.join(", ") || t("Files only")}</div>
      <h4>${t("Submissions")}</h4>
      ${!submissions
        ? null
        : submissions.length
          ? submissions.map(
              (s) => html`
                <div key=${s.sid} class=${`sub-item ${s.status}`} onDblClick=${() => (s.status === "pending" || s.status === "saved") && open(s.sid)}>
                  <div class="sub-main">
                    <div><span class=${`status ${s.status}`}>${submissionState(s.status)}</span> <span class="when">${when(s.received_at)}</span></div>
                    <div class="muted small">
                      ${s.status === "saved"
                        ? `${s.profile_name ? t("Saved {date} into {profile}", { date: when(s.handled_at), profile: s.profile_name }) : t("Saved {date}", { date: when(s.handled_at) })} · `
                        : ""}<span class="size">${megabytes(s.size)}</span>
                    </div>
                  </div>
                  ${(s.status === "pending" || s.status === "saved") &&
                  html`<button class=${s.status === "pending" ? "primary" : ""} onClick=${() => open(s.sid)}>${s.status === "pending" ? t("Review") : t("Open")}</button>`}
                </div>`
            )
          : html`<div class="muted small">${t("Nothing received yet. When someone fills in the form it shows up here, and on Activity.")}</div>`}
    </aside>`;
}

/** The New link sheet; `draft` prefills it (a link an agent asked for, confirmed through `requestId`). */
export function openLinkSheet(draft = {}, requestId = null) {
  return openSheet((close) => html`<${LinkSheet} draft=${draft} requestId=${requestId} close=${close} />`);
}

/**
 * Mouse handlers that paint a checkbox column: pressing flips the box, and dragging down the column gives every box
 * passed over that same value. On cells, so the whole cell (and the key next to Ask) is a target.
 */
function paintable(paint, column, checked, set) {
  return {
    onMouseDown: (e) => {
      e.preventDefault();
      paint.current = { column, value: !checked };
      set(!checked);
    },
    onMouseEnter: () => {
      if (paint.current?.column === column && checked !== paint.current.value) set(paint.current.value);
    },
  };
}

/** A paintable checkbox cell; the keyboard still toggles the box. */
const PaintCell = ({ column, checked, set, paint, cls }) => html`
  <td class="paint" ...${paintable(paint, column, checked, set)}>
    <input type="checkbox" class=${cls} checked=${checked} onChange=${(e) => set(e.target.checked)} />
  </td>`;

let nextRow = 0;
const customRow = (f = {}) => ({ uid: nextRow++, key: f.key || "", label: f.label || "", type: f.type || "text", required: !!f.required });

function LinkSheet({ draft, requestId, close }) {
  const profiles = useLoad(() => call("list_profiles"), []);
  const [title, setTitle] = useState(draft.title || "");
  const [from, setFrom] = useState(draft.from || "");
  const [note, setNote] = useState(draft.note || "");
  const [profileId, setProfileId] = useState(draft.profile_id ?? (draft.new_profile ? "new" : null));
  const [newName, setNewName] = useState(draft.new_profile || "");
  // key -> {ask, label, required} for the chosen profile's fields
  const [asked, setAsked] = useState({});
  const [profileFields, setProfileFields] = useState([]);
  const [custom, setCustom] = useState([]);
  const [maxFiles, setMaxFiles] = useState(String(draft.max_files ?? 5));
  const [kinds, setKinds] = useState((draft.kinds || []).join(", "));
  const [days, setDays] = useState(String(draft.expires_days ?? 7));
  const [maxSubmissions, setMaxSubmissions] = useState(String(draft.max_submissions ?? 1));
  const [problem, setProblem] = useState("");
  const paint = useRef(null);

  useEffect(() => {
    const stop = () => (paint.current = null);
    document.addEventListener("mouseup", stop);
    return () => document.removeEventListener("mouseup", stop);
  }, []);

  useEffect(() => {
    if (profiles && profileId === null) setProfileId(profiles[0]?.id ?? "new");
  }, [profiles]);

  // the chosen profile's fields; a draft's fields that the profile has are ticked, the others are custom rows
  useEffect(() => {
    if (profileId === null) return;
    let live = true;
    const load = profileId === "new" ? Promise.resolve([]) : call("profile_fields", { id: profileId });
    load.then((fields) => {
      if (!live) return;
      const wanted = draft.fields || [];
      const keys = new Set(fields.map((f) => f.key));
      setProfileFields(fields.map(({ key, label, type }) => ({ key, label, type })));
      setAsked(
        Object.fromEntries(
          fields.map((f) => {
            const wish = wanted.find((w) => w.key === f.key);
            return [f.key, { ask: !!wish, label: wish?.label || f.label || f.key, required: !!wish?.required }];
          })
        )
      );
      setCustom((rows) => (rows.length || !wanted.length ? rows : wanted.filter((w) => !keys.has(w.key)).map(customRow)));
    });
    return () => {
      live = false;
    };
  }, [profileId]);

  if (!profiles) return null;

  // functional updates: a drag changes several rows before the next render
  const setAsk = (key, patch) => setAsked((all) => ({ ...all, [key]: { ...all[key], ...patch } }));
  const setRow = (uid, patch) => setCustom((rows) => rows.map((r) => (r.uid === uid ? { ...r, ...patch } : r)));

  const cancel = async () => {
    if (requestId) await invoke("cancel_request", { id: requestId });
    close();
  };

  const create = async () => {
    const fields = [
      ...profileFields.filter((f) => asked[f.key]?.ask).map((f) => ({ key: f.key, label: asked[f.key].label, type: f.type, hint: "", required: asked[f.key].required })),
      ...custom.filter((r) => r.key.trim()).map((r) => ({ key: r.key.trim(), label: r.label.trim(), type: r.type, hint: "", required: r.required })),
    ];
    const isNew = profileId === "new";
    if (!title.trim()) return setProblem(t("Give the link a title"));
    if (isNew && !newName.trim()) return setProblem(t("Name the new profile"));
    const linkDraft = {
      title,
      from,
      note,
      profile_id: isNew ? null : Number(profileId),
      new_profile: isNew ? newName.trim() : null,
      fields,
      max_files: Number(maxFiles) || 0,
      kinds: kinds.split(",").map((k) => k.trim()).filter(Boolean),
      expires_days: Number(days),
      max_submissions: Number(maxSubmissions) || 1,
    };
    let link;
    try {
      link = await invoke("create_link", { draft: linkDraft, requestId });
    } catch (e) {
      return setProblem(String(e));
    }
    await call("copy_text", { text: link.url }).catch(() => {});
    toast(t("Link created and copied."), 4000);
    close();
  };

  const types = state.status.field_types.map((t) => [t, t]);
  const profileOptions = [...profiles.map((p) => [String(p.id), p.name]), ["new", t("New profile…")]];
  return html`
    <div class="sheet-head">
      <${SheetHead} icon="links" title=${requestId ? t("Your AI assistant drafted a collection link") : t("New collection link")} reason=${draft.reason} />
    </div>
    <div class="sheet-body">
      <form class="input-form link-form" autocomplete="off" onSubmit=${(e) => (e.preventDefault(), create())}>
        <label>${t("Title")}</label>
        <input type="text" name="title" value=${title} placeholder=${t("e.g. Details for your loan application")} onInput=${(e) => setTitle(e.target.value)} />
        <div class="hint muted small">${t("The heading of the form and the name of the browser tab.")}</div>
        <label>${t("From")}</label>
        <input type="text" name="from" value=${from} placeholder=${t("e.g. Maria at Example Lending")} onInput=${(e) => setFrom(e.target.value)} />
        <div class="hint muted small">${t("Shown as \"Requested by\", next to the email of your Blankey Online account.")}</div>
        <label>${t("Note")}</label>
        <textarea name="note" rows="2" value=${note} placeholder=${t("e.g. Please send these by Friday. A photo of each page is fine.")} onInput=${(e) => setNote(e.target.value)}></textarea>
        <div class="hint muted small">${t("Shown under the heading, above the fields.")}</div>
        <label>${t("Save into")}</label>
        <${Select} name="profile" value=${profileId ?? ""} options=${profileOptions} onChange=${(v) => setProfileId(v === "new" ? "new" : Number(v))} />
        ${profileId === "new" && html`
          <label>${t("Profile name")}</label>
          <input type="text" name="new_profile" value=${newName} onInput=${(e) => setNewName(e.target.value)} />`}
        <label class="top">${t("Fields")}</label>
        <table class="fields link-fields">
          <thead><tr><th></th><th>${t("Key")}</th><th>${t("Label")}</th><th>${t("Type")}</th><th>${t("Required")}</th><th></th></tr></thead>
          <tbody>
            ${profileFields.map(
              (f) => html`
                <tr key=${f.key}>
                  <${PaintCell} column="ask" cls="ask" paint=${paint} checked=${!!asked[f.key]?.ask} set=${(ask) => setAsk(f.key, { ask })} />
                  <td class="mono paint" ...${paintable(paint, "ask", !!asked[f.key]?.ask, (ask) => setAsk(f.key, { ask }))}>${f.key}</td>
                  <td><input type="text" value=${asked[f.key]?.label || ""} onInput=${(e) => setAsk(f.key, { label: e.target.value })} /></td>
                  <td class="muted">${f.type}</td>
                  <${PaintCell} column="required" cls="required" paint=${paint} checked=${!!asked[f.key]?.required} set=${(required) => setAsk(f.key, { required })} />
                  <td></td>
                </tr>`
            )}
            ${custom.map(
              (r) => html`
                <tr key=${`c${r.uid}`} class="custom">
                  <td></td>
                  <td><input type="text" class="key mono" value=${r.key} placeholder="address.city" spellcheck="false" onInput=${(e) => setRow(r.uid, { key: e.target.value })} /></td>
                  <td><input type="text" class="label" value=${r.label} onInput=${(e) => setRow(r.uid, { label: e.target.value })} /></td>
                  <td><${Select} class="type" value=${r.type} options=${types} onChange=${(type) => setRow(r.uid, { type })} /></td>
                  <${PaintCell} column="required" cls="required" paint=${paint} checked=${r.required} set=${(required) => setRow(r.uid, { required })} />
                  <td><button type="button" class="remove ghost" title=${t("Remove field")} onClick=${() => setCustom(custom.filter((x) => x.uid !== r.uid))}>✕</button></td>
                </tr>`
            )}
          </tbody>
        </table>
        <span></span>
        <div class="check-row">
          <button type="button" onClick=${() => setCustom([...custom, customRow()])}>${t("Add field")}</button>
          <span class="muted small">${t("Tip: click a key to ask for it, or press a checkbox or key and drag down the column to tick or untick several at once.")}</span>
        </div>
        <label>${t("Files")}</label>
        <div class="check-row">
          <input type="number" name="max_files" min="0" max="10" style="width:70px" value=${maxFiles} onInput=${(e) => setMaxFiles(e.target.value)} />
          <input type="text" name="kinds" placeholder=${t("Any kind, or e.g. .pdf, image/*")} value=${kinds} onInput=${(e) => setKinds(e.target.value)} />
        </div>
        <div class="hint muted small">${t("Up to 50 MB each.")}</div>
        <label>${t("Expires after")}</label>
        <${Select} name="days" value=${days} options=${DAYS.map((d) => [String(d), tn(d, "{n} day", "{n} days")])} onChange=${setDays} />
        <label>${t("Submissions")}</label>
        <input type="number" name="max_submissions" min="1" max="100" style="width:70px" value=${maxSubmissions} onInput=${(e) => setMaxSubmissions(e.target.value)} />
      </form>
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problem}</div>
    </div>
    <div class="sheet-foot">
      <span class="note">${t("The form shows only the title, note and labels. What people send is encrypted in their browser and opens only on this computer.")}</span>
      <button onClick=${cancel}>${t("Cancel")}</button>
      <button class="primary" onClick=${create}>${t("Create link")}</button>
    </div>`;
}
