import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { problemsAt, problemsElsewhere, t, tn } from "../i18n.js";
import { html, Icon, matches, megabytes, placeholder, reload, state, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, dialog, toast } from "../ui/feedback.js";
import { Page } from "../ui/page.js";
import { Select } from "../ui/select.js";
import { openSheet, SheetHead, useSheetEscape } from "../ui/sheet.js";
import { keepText, useSelection } from "../ui/table.js";

export function Profiles() {
  const profiles = useLoad(() => call("list_profiles"), []);
  const [picked, pick, setSelected] = useSelection("profiles");
  const [dirty, setDirty] = useState(false);
  // bumped after a save, so the detail reloads its fields from the vault
  const [version, setVersion] = useState(0);
  if (!profiles) return null;

  const visible = profiles.filter((p) => matches(p.name, p.kind));
  const order = visible.map((p) => p.id);
  const chosen = picked.filter((id) => order.includes(id));
  const selected = chosen.length ? chosen : order.slice(0, 1);
  // the detail shows the profile clicked last
  const current = selected.at(-1);
  const profile = profiles.find((p) => p.id === current);

  const choose = async (e, id) => {
    const event = { metaKey: e.metaKey, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey };
    if (dirty && id !== current) {
      if (!(await confirmDialog(t("Discard unsaved changes?"), t("The fields of this profile were edited."), t("Discard")))) return;
    }
    setDirty(false);
    pick(event, id, order, selected);
  };

  const create = async () => {
    const values = await dialog({
      title: t("New profile"),
      fields: [
        { name: "name", label: t("Name") },
        { name: "kind", label: t("Kind (person, company, loan…)"), value: "person" },
      ],
      ok: t("Create"),
    });
    if (!values?.name?.trim()) return;
    setSelected([await call("create_profile", values)]);
    reload();
  };

  const rename = async () => {
    if (!profile) return toast(t("Select a profile"));
    const values = await dialog({
      title: t("Rename profile"),
      fields: [
        { name: "name", label: t("Name"), value: profile.name },
        { name: "kind", label: t("Kind"), value: profile.kind },
      ],
      ok: t("Save"),
    });
    if (!values) return;
    await call("update_profile", { id: profile.id, name: values.name.trim() || profile.name, kind: values.kind });
    reload();
  };

  const remove = async () => {
    if (!profile) return toast(t("Select a profile"));
    const chosen = profiles.filter((p) => selected.includes(p.id));
    const title =
      chosen.length === 1
        ? t("Delete the profile \"{name}\"?", { name: chosen[0].name })
        : tn(chosen.length, "Delete {n} profile?", "Delete {n} profiles?");
    if (!(await confirmDialog(title, t("All its fields and files are removed from the vault.")))) return;
    for (const p of chosen) await call("delete_profile", { id: p.id });
    setDirty(false);
    setSelected([]);
    reload();
  };

  const saved = () => {
    setDirty(false);
    setVersion(version + 1);
  };

  return html`
    <${Page} title=${t("Profiles")} subtitle=${t("{profiles}, values encrypted per field", { profiles: tn(profiles.length, "{n} profile", "{n} profiles") })} actions=${[
      [t("New profile"), create, "", "profiles.new"],
      [t("Rename"), rename, "", "profiles.rename"],
      [t("Delete profile"), remove, "danger", "profiles.delete"],
    ]}>
      <div class="split">
        <div class="list" data-help="profiles.list">
          ${visible.length
            ? visible.map(
                (p) => html`
                  <div key=${p.id} class=${`list-item ${selected.includes(p.id) ? "active" : ""}`} onMouseDown=${keepText}
                    onClick=${(e) => choose(e, p.id)}>
                    <div class="title">${p.name}</div>
                    <div class="muted small">${p.kind || t("profile")} · #${p.id}</div>
                  </div>`
              )
            : html`<div class="empty">${t("No profiles")}</div>`}
        </div>
        <div class="detail">
          ${profile
            ? html`<${ProfileDetail} key=${`${profile.id}:${version}`} profile=${profile} onDirty=${() => setDirty(true)} onSaved=${saved} />`
            : html`<div class="empty">${t("Create a profile to store personal data.")}</div>`}
        </div>
      </div>
    </${Page}>`;
}

/** A profile in a sheet of its own: the profiles of a case are edited from the case. Resolves when it closes. */
export const openProfileSheet = (profile) => openSheet((close) => html`<${ProfileSheet} profile=${profile} close=${close} />`);

function ProfileSheet({ profile, close }) {
  const [dirty, setDirty] = useState(false);
  const [version, setVersion] = useState(0);
  const done = async () => {
    if (dirty && !(await confirmDialog(t("Discard unsaved changes?"), t("The fields of this profile were edited."), t("Discard")))) return;
    close();
  };
  useSheetEscape(done);
  const saved = () => {
    setDirty(false);
    setVersion(version + 1);
  };
  return html`
    <div class="sheet-head"><${SheetHead} icon="person" title=${profile.name} reason=${profile.kind} /></div>
    <div class="sheet-body">
      <div class="detail profile-sheet">
        <${ProfileDetail} key=${version} profile=${profile} onDirty=${() => setDirty(true)} onSaved=${saved} />
      </div>
    </div>
    <div class="sheet-foot"><span class="spacer"></span><button class="primary" onClick=${done}>${t("Close")}</button></div>`;
}

let nextRow = 0;
const row = (f) => ({ uid: nextRow++, original: f.key, key: f.key, label: f.label, type: f.type, value: f.value ?? "" });
const blankRow = () => row({ key: "", label: "", type: "text", value: "" });

const CSV_HEAD = "key,label,type,value";
const csvCell = (text) => (/[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text);

/** The fields as CSV text under a header line, one field per line; rows left empty are skipped. */
function toCsv(rows) {
  const filled = rows.filter((r) => r.key || r.label || r.value);
  return [CSV_HEAD, ...filled.map((r) => [r.key, r.label, r.type, r.value].map(csvCell).join(","))].join("\n");
}

/** The records of CSV text as [{ line, cells }], empty lines skipped; `cells` is null where a quote is left open. */
function csvRecords(text) {
  const records = [];
  let cells = [];
  let cell = "";
  let quoted = false;
  let line = 1;
  let start = 1;
  const end = () => {
    cells.push(cell);
    if (cells.some(Boolean)) records.push({ line: start, cells });
    cells = [];
    cell = "";
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === "\n") line++;
    if (quoted) {
      if (c !== '"') cell += c;
      else if (text[i + 1] === '"') cell += text[i++];
      else quoted = false;
    } else if (c === '"' && !cell) quoted = true;
    else if (c === ",") {
      cells.push(cell);
      cell = "";
    } else if (c === "\n") {
      end();
      start = line;
    } else if (c !== "\r") cell += c;
  }
  if (quoted) records.push({ line: start, cells: null });
  else end();
  return records;
}

/** What CSV text holds, as { fields } or { error } with every bad line: the header line is optional and an empty type is text. */
function fromCsv(text) {
  const fields = [];
  const errors = [];
  const firstLine = new Map();
  for (const [i, { line, cells }] of csvRecords(text).entries()) {
    if (!cells || cells.length > 4) {
      errors.push(t("Line {line}: expected key, label, type, value", { line }));
      continue;
    }
    if (!i && cells.join(",").toLowerCase() === CSV_HEAD) continue;
    const [name, label = "", kind = "", value = ""] = cells;
    const key = name.trim();
    const type = kind.trim() || "text";
    if (!key) errors.push(t("Line {line}: the key is missing", { line }));
    else if (firstLine.has(key)) errors.push(t("Line {line}: the key \"{key}\" is already on line {first}", { line, key, first: firstLine.get(key) }));
    else firstLine.set(key, line);
    if (!state.status.field_types.includes(type)) errors.push(t("Line {line}: unknown type \"{type}\"", { line, type }));
    fields.push({ key, label, type, value });
  }
  return errors.length ? { error: errors.join("\n") } : { fields };
}

function ProfileDetail({ profile, onDirty, onSaved }) {
  const fields = useLoad(() => call("profile_fields", { id: profile.id }), [profile.id]);
  const [rows, setRows] = useState(null);
  const [removed, setRemoved] = useState([]);
  const [reveal, setReveal] = useRemembered("profiles.reveal", false);
  const [status, setStatus] = useState("");
  const [problems, setProblems] = useState([]);
  const [focusUid, setFocusUid] = useState(null);
  const [asCsv, setAsCsv] = useRemembered("profiles.csv", false);
  // the CSV text once it is edited; until then it is written from the rows
  const [csv, setCsv] = useState(null);
  const [csvError, setCsvError] = useState("");

  useEffect(() => {
    if (fields) setRows(fields.length ? fields.map(row) : [blankRow()]);
  }, [fields]);

  const edited = (next) => {
    setRows(next);
    setStatus(t("Unsaved changes"));
    onDirty();
  };
  const change = (uid, patch) => edited(rows.map((r) => (r.uid === uid ? { ...r, ...patch } : r)));
  const drop = (r) => {
    if (r.original) setRemoved([...removed, r.original]);
    edited(rows.filter((x) => x.uid !== r.uid));
  };
  const add = () => {
    const r = blankRow();
    setRows([...rows, r]);
    setFocusUid(r.uid);
  };

  /** The rows of the edited CSV text and the saved keys it leaves out; null, with the error shown, when it does not parse. */
  const csvRows = () => {
    const { fields: parsed, error = "" } = fromCsv(csv);
    setCsvError(error);
    if (error) return null;
    const saved = fields.map((f) => f.key);
    return {
      rows: parsed.map((f) => ({ ...row(f), original: saved.includes(f.key) ? f.key : "" })),
      removed: saved.filter((key) => !parsed.some((f) => f.key === key)),
    };
  };
  const editCsv = (text) => {
    setCsv(text);
    setStatus(t("Unsaved changes"));
    onDirty();
  };
  const toggleCsv = () => {
    if (asCsv && csv !== null) {
      const edit = csvRows();
      if (!edit) return;
      setRows(edit.rows.length ? edit.rows : [blankRow()]);
      setRemoved(edit.removed);
      setCsv(null);
    }
    setAsCsv(!asCsv);
  };

  const save = async () => {
    const edit = asCsv && csv !== null ? csvRows() : { rows, removed };
    if (!edit) return setStatus(t("Fix the problems below"));
    const values = edit.rows.map((r) => ({ key: r.key.trim(), label: r.label.trim(), type: r.type, value: r.value }));
    const renamed = edit.rows.filter((r, i) => r.original && r.original !== values[i].key).map((r) => r.original);
    const result = await call("save_fields", {
      id: profile.id,
      fields: values.filter((f) => f.key),
      removed: [...edit.removed, ...renamed],
    });
    setProblems(result);
    if (result.length) return setStatus(t("Fix the problems below"));
    toast(t("Saved"));
    onSaved();
  };

  if (!rows) return null;
  return html`
    <div class="fields-head">
      <h3>${profile.name}</h3>
      <button data-help="profiles.csv" onClick=${toggleCsv}>${asCsv ? t("Edit as table") : t("Edit as CSV")}</button>
    </div>
    <div class="muted small">${t("Your AI assistant sees field names, labels, types and how long the values are. Never the values.")}</div>
    ${asCsv
      ? html`<textarea class="fields-csv mono" data-help="profiles.fields" spellcheck="false" wrap="off" value=${csv ?? toCsv(rows)}
          rows=${Math.max(8, (csv ?? toCsv(rows)).split("\n").length + 1)} onInput=${(e) => editCsv(e.target.value)}></textarea>`
      : html`<table class="fields" data-help="profiles.fields">
          <thead><tr><th>${t("Key")}</th><th>${t("Label")}</th><th>${t("Type")}</th><th>${t("Value")}</th><th></th></tr></thead>
          <tbody>
            ${rows.map(
              (r) => html`<${FieldRow} key=${r.uid} r=${r} reveal=${reveal} focus=${r.uid === focusUid} problem=${problemsAt(problems, r.key.trim())}
                onChange=${(patch) => change(r.uid, patch)} onRemove=${() => drop(r)} />`
            )}
          </tbody>
        </table>`}
    <div class="detail-actions">
      ${!asCsv &&
      html`<button data-help="profiles.add" onClick=${add}>${t("Add field")}</button>
        <label class="check-row small muted" data-help="profiles.reveal">
          <input type="checkbox" checked=${reveal} onChange=${(e) => setReveal(e.target.checked)} /> ${t("Show values")}
        </label>`}
      <span class="spacer"></span>
      <span class="muted small">${status}</span>
      <button class="primary" data-help="profiles.save" onClick=${save}>${t("Save")}</button>
    </div>
    <div class="problems">${asCsv ? [csvError, problemsElsewhere(problems, [])].filter(Boolean).join("\n") : problemsElsewhere(problems, rows.map((r) => r.key.trim()))}</div>
    <${ProfileFiles} profile=${profile} />`;
}

/** Files kept with the profile: the ones the user adds and the ones its collection links brought in. */
function ProfileFiles({ profile }) {
  // bumped after a change; a page reload would drop unsaved field edits
  const [version, setVersion] = useState(0);
  const files = useLoad(() => call("list_collected_files"), [version]);
  if (!files) return null;
  const mine = files.filter((f) => f.profile_id === profile.id);

  const add = async () => {
    const added = await call("add_profile_files", { profileId: profile.id }).catch(() => 0);
    if (!added) return;
    toast(tn(added, "{n} file added", "{n} files added"));
    setVersion(version + 1);
  };
  const exportFile = async (id) => {
    const path = await call("export_collected_file", { id });
    if (path) toast(t("Saved to {path}", { path }), 4000);
  };
  const remove = async (file) => {
    const text = t("It is removed from the vault. Copies you exported stay where you saved them.");
    if (!(await confirmDialog(t("Delete \"{name}\"?", { name: file.name }), text))) return;
    await call("delete_collected_files", { ids: [file.id] });
    setVersion(version + 1);
  };
  return html`
    <div class="files-head" data-help="profiles.files">
      <h4>${t("Files")}</h4>
      <button onClick=${add}>${t("Add files…")}</button>
    </div>
    <div class="profile-files" data-help="profiles.files">
      ${mine.map(
        (f) => html`
          <div class="file-row" key=${f.id} onDblClick=${() => call("open_collected_files", { ids: [f.id] })}>
            <span class="name" title=${f.name}>${f.name}</span>
            <span class="muted small">${[f.link_title, when(f.created_at), megabytes(f.size)].filter(Boolean).join(" · ")}</span>
            <button class="link" onClick=${() => call("open_collected_files", { ids: [f.id] })}>${t("Open")}</button>
            <button class="link" onClick=${() => exportFile(f.id)}>${t("Export")}</button>
            <button class="link danger" onClick=${() => remove(f)}>${t("Delete")}</button>
          </div>`
      )}
    </div>`;
}

function FieldRow({ r, reveal, focus, problem, onChange, onRemove }) {
  const keyInput = useRef();
  useEffect(() => {
    if (focus) keyInput.current.focus();
  }, [focus]);
  const input = (field) => (e) => onChange({ [field]: e.target.value });
  // this one value shown while the others stay dots
  const [shown, setShown] = useState(false);
  return html`
    <tr>
      <td style="width:26%">
        <input type="text" class="key mono" ref=${keyInput} value=${r.key} placeholder="address.city" spellcheck="false" onInput=${input("key")} />
      </td>
      <td style="width:22%"><input type="text" class="label" value=${r.label} onInput=${input("label")} /></td>
      <td style="width:12%">
        <${Select} class="type" value=${r.type} options=${state.status.field_types.map((t) => [t, t])}
          onChange=${(type) => onChange({ type })} />
      </td>
      <td>
        <div class="value-cell">
          <input type="text" class=${`value ${reveal || shown ? "" : "masked"} ${problem ? "invalid" : ""}`} value=${r.value}
            placeholder=${placeholder(r.type)} spellcheck="false" onInput=${input("value")} />
          ${!reveal &&
          html`<button class="eye ghost" data-help="profiles.eye" title=${shown ? t("Hide value") : t("Show value")} aria-label=${shown ? t("Hide value") : t("Show value")}
            onClick=${() => setShown(!shown)}><${Icon} name=${shown ? "eye-off" : "eye"} /></button>`}
        </div>
      </td>
      <td style="width:1%"><button class="remove ghost" data-help="profiles.remove" title=${t("Remove field")} onClick=${onRemove}>✕</button></td>
    </tr>
    ${problem && html`<tr><td colspan="5" class="field-problem">${problem}</td></tr>`}`;
}
