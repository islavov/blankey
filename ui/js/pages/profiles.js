import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { problemText, t, tn } from "../i18n.js";
import { html, matches, megabytes, placeholder, reload, state, useLoad, useRemembered, when } from "../lib.js";
import { confirmDialog, dialog, toast } from "../ui/feedback.js";
import { Page } from "../ui/page.js";
import { Select } from "../ui/select.js";
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

let nextRow = 0;
const row = (f) => ({ uid: nextRow++, original: f.key, key: f.key, label: f.label, type: f.type, value: f.value ?? "" });
const blankRow = () => row({ key: "", label: "", type: "text", value: "" });

function ProfileDetail({ profile, onDirty, onSaved }) {
  const fields = useLoad(() => call("profile_fields", { id: profile.id }), [profile.id]);
  const [rows, setRows] = useState(null);
  const [removed, setRemoved] = useState([]);
  const [reveal, setReveal] = useRemembered("profiles.reveal", false);
  const [status, setStatus] = useState("");
  const [problems, setProblems] = useState([]);
  const [focusUid, setFocusUid] = useState(null);

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

  const save = async () => {
    const values = rows.map((r) => ({ key: r.key.trim(), label: r.label.trim(), type: r.type, value: r.value }));
    const renamed = rows.filter((r, i) => r.original && r.original !== values[i].key).map((r) => r.original);
    const result = await call("save_fields", {
      id: profile.id,
      fields: values.filter((f) => f.key),
      removed: [...removed, ...renamed],
    });
    setProblems(result);
    if (result.length) return setStatus(t("Fix the problems below"));
    toast(t("Saved"));
    onSaved();
  };

  if (!rows) return null;
  return html`
    <h3>${profile.name}</h3>
    <div class="muted small">${t("Your AI assistant sees field names, labels, types and how long the values are. Never the values.")}</div>
    <table class="fields" data-help="profiles.fields">
      <thead><tr><th>${t("Key")}</th><th>${t("Label")}</th><th>${t("Type")}</th><th>${t("Value")}</th><th></th></tr></thead>
      <tbody>
        ${rows.map(
          (r) => html`<${FieldRow} key=${r.uid} r=${r} reveal=${reveal} focus=${r.uid === focusUid}
            onChange=${(patch) => change(r.uid, patch)} onRemove=${() => drop(r)} />`
        )}
      </tbody>
    </table>
    <div class="detail-actions">
      <button data-help="profiles.add" onClick=${add}>${t("Add field")}</button>
      <label class="check-row small muted" data-help="profiles.reveal">
        <input type="checkbox" checked=${reveal} onChange=${(e) => setReveal(e.target.checked)} /> ${t("Show values")}
      </label>
      <span class="spacer"></span>
      <span class="muted small">${status}</span>
      <button class="primary" data-help="profiles.save" onClick=${save}>${t("Save")}</button>
    </div>
    <div class="problems">${problems.map(problemText).join("\n")}</div>
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
          <div class="file-row" key=${f.id}>
            <span class="name" title=${f.name}>${f.name}</span>
            <span class="muted small">${[f.link_title, when(f.created_at), megabytes(f.size)].filter(Boolean).join(" · ")}</span>
            <button class="link" onClick=${() => call("open_collected_files", { ids: [f.id] })}>${t("Open")}</button>
            <button class="link" onClick=${() => exportFile(f.id)}>${t("Export")}</button>
            <button class="link danger" onClick=${() => remove(f)}>${t("Delete")}</button>
          </div>`
      )}
    </div>`;
}

function FieldRow({ r, reveal, focus, onChange, onRemove }) {
  const keyInput = useRef();
  useEffect(() => {
    if (focus) keyInput.current.focus();
  }, [focus]);
  const input = (field) => (e) => onChange({ [field]: e.target.value });
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
        <input type="text" class=${`value ${reveal ? "" : "masked"}`} value=${r.value}
          placeholder=${placeholder(r.type)} spellcheck="false" onInput=${input("value")} />
      </td>
      <td style="width:1%"><button class="remove ghost" data-help="profiles.remove" title=${t("Remove field")} onClick=${onRemove}>✕</button></td>
    </tr>`;
}
