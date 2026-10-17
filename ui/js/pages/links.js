import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call, invoke } from "../api.js";
import { html, matches, plural, reload, state, useLoad, useRemembered, useStore, when } from "../lib.js";
import { confirmDialog, toast } from "../ui/feedback.js";
import { openSheet, SheetHead } from "../ui/sheet.js";
import { Empty, Page } from "../ui/page.js";
import { Select } from "../ui/select.js";
import { Table } from "../ui/table.js";
import { relaySignIn } from "./connections.js";

const STATES = { open: "Open", expired: "Expired", revoked: "Revoked", full: "Full" };
const DAYS = [1, 7, 30];

const copyLink = (url) => call("copy_text", { text: url }).then(() => toast("Link copied."));
const shortUrl = (url) => url.replace(/^https?:\/\//, "").replace(/#.*$/, "");

/** Links that ask someone else for data: they fill a form on the relay, sealed to this computer. */
export function Links() {
  const { relay } = useStore();
  const links = useLoad(() => call("list_links"), []);
  const [selected, setSelected] = useRemembered("links", null);
  if (!links || !relay) return null;

  const link = links.find((l) => l.link_id === selected);
  const need = (action) => () => (link ? action(link) : toast("Select a link"));

  const create = async () => {
    await openLinkSheet();
    reload();
  };

  const revoke = async (l) => {
    const text = "The form stops taking answers, and anything sent to it that has not arrived yet can no longer be opened.";
    if (!(await confirmDialog(`Revoke "${l.title}"?`, text, "Revoke"))) return;
    await call("revoke_link", { linkId: l.link_id });
    reload();
  };

  if (!relay.signed_in && !links.length) {
    return html`
      <${Page} title="Collection links" subtitle="Ask someone for their data">
        <${Empty}>
          Sign in to the relay to make collection links.<br />
          <span class="small">The person you send a link to fills in a form. What they send is encrypted in their browser and only opens on this computer.</span>
          <div style="margin-top:14px"><button class="primary" onClick=${() => relaySignIn().catch(() => {})}>Sign in</button></div>
        </${Empty}>
      </${Page}>`;
  }

  const rows = links.map((l) => ({ ...l, id: l.link_id })).filter((l) => matches(l.title, l.profile_name, ...l.fields));
  return html`
    <${Page} title="Collection links" subtitle=${plural(links.length, "link")} actions=${[
      ["New link", create, "primary"],
      ["Copy link", need((l) => copyLink(l.url))],
      ["Revoke", need(revoke), "danger"],
    ]}>
      ${rows.length
        ? html`<${Table}
            rows=${rows}
            selected=${[selected]}
            onSelect=${([id]) => setSelected(id)}
            onOpen=${(id) => copyLink(links.find((l) => l.link_id === id).url)}
            columns=${[
              { title: "Title", cell: (l) => l.title },
              {
                title: "Link",
                cell: (l) => html`<span class="mono">${shortUrl(l.url)}</span>
                  <button class="link" style="margin-left:8px" onClick=${() => copyLink(l.url)}>Copy</button>`,
                cls: "nowrap",
              },
              { title: "Fields", cell: (l) => html`<span class="muted">${l.fields.join(", ") || "Files only"}</span>` },
              { title: "Profile", cell: (l) => l.profile_name || "", cls: "nowrap" },
              { title: "Expires", cell: (l) => html`<span class="muted">${when(l.expires_at)}</span>`, cls: "nowrap" },
              { title: "Received", cell: (l) => `${l.submissions}/${l.max_submissions}`, cls: "nowrap" },
              { title: "Status", cell: (l) => html`<span class=${`status ${l.state}`}>${STATES[l.state] || l.state}</span>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>No collection links yet.<br /><span class="small">Make one to ask someone for the data of a profile, with files.</span></${Empty}>`}
    </${Page}>`;
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
    if (isNew && !newName.trim()) return setProblem("Name the new profile");
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
    toast("Link created and copied.", 4000);
    close();
  };

  const types = state.status.field_types.map((t) => [t, t]);
  const profileOptions = [...profiles.map((p) => [String(p.id), p.name]), ["new", "New profile…"]];
  return html`
    <div class="sheet-head">
      <${SheetHead} icon="links" title=${requestId ? "Your AI assistant drafted a collection link" : "New collection link"} reason=${draft.reason} />
    </div>
    <div class="sheet-body">
      <form class="input-form link-form" autocomplete="off" onSubmit=${(e) => (e.preventDefault(), create())}>
        <label>Title</label>
        <input type="text" name="title" value=${title} placeholder="e.g. Details for your loan application" onInput=${(e) => setTitle(e.target.value)} />
        <div class="hint muted small">The heading of the form and the name of the browser tab.</div>
        <label>From</label>
        <input type="text" name="from" value=${from} placeholder="e.g. Maria at Example Lending" onInput=${(e) => setFrom(e.target.value)} />
        <div class="hint muted small">Shown as "Requested by", next to the email of your relay account.</div>
        <label>Note</label>
        <textarea name="note" rows="2" value=${note} placeholder="e.g. Please send these by Friday. A photo of each page is fine." onInput=${(e) => setNote(e.target.value)}></textarea>
        <div class="hint muted small">Shown under the heading, above the fields.</div>
        <label>Save into</label>
        <${Select} name="profile" value=${profileId ?? ""} options=${profileOptions} onChange=${(v) => setProfileId(v === "new" ? "new" : Number(v))} />
        ${profileId === "new" && html`
          <label>Profile name</label>
          <input type="text" name="new_profile" value=${newName} onInput=${(e) => setNewName(e.target.value)} />`}
        <label class="top">Fields</label>
        <table class="fields link-fields">
          <thead><tr><th></th><th>Key</th><th>Label</th><th>Type</th><th>Required</th><th></th></tr></thead>
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
                  <td><button type="button" class="remove ghost" title="Remove field" onClick=${() => setCustom(custom.filter((x) => x.uid !== r.uid))}>✕</button></td>
                </tr>`
            )}
          </tbody>
        </table>
        <span></span>
        <div class="check-row">
          <button type="button" onClick=${() => setCustom([...custom, customRow()])}>Add field</button>
          <span class="muted small">Tip: click a key to ask for it, or press a checkbox or key and drag down the column to tick or untick several at once.</span>
        </div>
        <label>Files</label>
        <div class="check-row">
          <input type="number" name="max_files" min="0" max="10" style="width:70px" value=${maxFiles} onInput=${(e) => setMaxFiles(e.target.value)} />
          <input type="text" name="kinds" placeholder="Any kind, or e.g. .pdf, image/*" value=${kinds} onInput=${(e) => setKinds(e.target.value)} />
        </div>
        <div class="hint muted small">Up to 50 MB each.</div>
        <label>Expires after</label>
        <${Select} name="days" value=${days} options=${DAYS.map((d) => [String(d), plural(d, "day")])} onChange=${setDays} />
        <label>Submissions</label>
        <input type="number" name="max_submissions" min="1" max="100" style="width:70px" value=${maxSubmissions} onInput=${(e) => setMaxSubmissions(e.target.value)} />
      </form>
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problem}</div>
    </div>
    <div class="sheet-foot">
      <span class="note">The form shows only the title, note and labels. What people send is encrypted in their browser and opens only on this computer.</span>
      <button onClick=${cancel}>Cancel</button>
      <button class="primary" onClick=${create}>Create link</button>
    </div>`;
}
