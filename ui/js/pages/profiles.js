import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call } from "../api.js";
import { html, matches, PLACEHOLDERS, plural, reload, state, useLoad, useRemembered } from "../lib.js";
import { confirmDialog, dialog, toast } from "../ui/feedback.js";
import { Page } from "../ui/page.js";
import { Select } from "../ui/select.js";

export function Profiles() {
  const profiles = useLoad(() => call("list_profiles"), []);
  const [selected, setSelected] = useRemembered("profiles", null);
  const [dirty, setDirty] = useState(false);
  // bumped after a save, so the detail reloads its fields from the vault
  const [version, setVersion] = useState(0);
  if (!profiles) return null;

  const visible = profiles.filter((p) => matches(p.name, p.kind));
  const current = visible.some((p) => p.id === selected) ? selected : visible[0]?.id;
  const profile = profiles.find((p) => p.id === current);

  const choose = async (id) => {
    if (dirty && id !== current) {
      if (!(await confirmDialog("Discard unsaved changes?", "The fields of this profile were edited.", "Discard"))) return;
    }
    setDirty(false);
    setSelected(id);
  };

  const create = async () => {
    const values = await dialog({
      title: "New profile",
      fields: [
        { name: "name", label: "Name" },
        { name: "kind", label: "Kind (person, company, loan…)", value: "person" },
      ],
      ok: "Create",
    });
    if (!values?.name?.trim()) return;
    setSelected(await call("create_profile", values));
    reload();
  };

  const rename = async () => {
    if (!profile) return toast("Select a profile");
    const values = await dialog({
      title: "Rename profile",
      fields: [
        { name: "name", label: "Name", value: profile.name },
        { name: "kind", label: "Kind", value: profile.kind },
      ],
      ok: "Save",
    });
    if (!values) return;
    await call("update_profile", { id: profile.id, name: values.name.trim() || profile.name, kind: values.kind });
    reload();
  };

  const remove = async () => {
    if (!profile) return toast("Select a profile");
    if (!(await confirmDialog(`Delete the profile "${profile.name}"?`, "All its fields are removed from the vault."))) return;
    await call("delete_profile", { id: profile.id });
    setSelected(null);
    reload();
  };

  const saved = () => {
    setDirty(false);
    setVersion(version + 1);
  };

  return html`
    <${Page} title="Profiles" subtitle=${`${plural(profiles.length, "profile")}, values encrypted per field`} actions=${[
      ["New profile", create],
      ["Rename", rename],
      ["Delete profile", remove, "danger"],
    ]}>
      <div class="split">
        <div class="list">
          ${visible.length
            ? visible.map(
                (p) => html`
                  <div key=${p.id} class=${`list-item ${p.id === current ? "active" : ""}`} onClick=${() => choose(p.id)}>
                    <div class="title">${p.name}</div>
                    <div class="muted small">${p.kind || "profile"} · #${p.id}</div>
                  </div>`
              )
            : html`<div class="empty">No profiles</div>`}
        </div>
        <div class="detail">
          ${profile
            ? html`<${ProfileDetail} key=${`${profile.id}:${version}`} profile=${profile} onDirty=${() => setDirty(true)} onSaved=${saved} />`
            : html`<div class="empty">Create a profile to store personal data.</div>`}
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
    setStatus("Unsaved changes");
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
    if (result.length) return setStatus("Fix the problems below");
    toast("Saved");
    onSaved();
  };

  if (!rows) return null;
  return html`
    <h3>${profile.name}</h3>
    <div class="muted small">Your AI assistant sees field names, labels, types and how long the values are. Never the values.</div>
    <table class="fields">
      <thead><tr><th>Key</th><th>Label</th><th>Type</th><th>Value</th><th></th></tr></thead>
      <tbody>
        ${rows.map(
          (r) => html`<${FieldRow} key=${r.uid} r=${r} reveal=${reveal} focus=${r.uid === focusUid}
            onChange=${(patch) => change(r.uid, patch)} onRemove=${() => drop(r)} />`
        )}
      </tbody>
    </table>
    <div class="detail-actions">
      <button onClick=${add}>Add field</button>
      <label class="check-row small muted">
        <input type="checkbox" checked=${reveal} onChange=${(e) => setReveal(e.target.checked)} /> Show values
      </label>
      <span class="spacer"></span>
      <span class="muted small">${status}</span>
      <button class="primary" onClick=${save}>Save</button>
    </div>
    <div class="problems">${problems.join("\n")}</div>`;
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
          placeholder=${PLACEHOLDERS[r.type] || ""} spellcheck="false" onInput=${input("value")} />
      </td>
      <td style="width:1%"><button class="remove ghost" title="Remove field" onClick=${onRemove}>✕</button></td>
    </tr>`;
}
