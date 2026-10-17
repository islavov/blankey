import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke, isLocked, refreshStatus } from "./api.js";
import { openFill } from "./fill/form.js";
import { html, megabytes, PLACEHOLDERS, plural, reload, update, useLoad } from "./lib.js";
import { openLinkSheet } from "./pages/links.js";
import { confirmDialog, toast } from "./ui/feedback.js";
import { openSheet, SheetHead } from "./ui/sheet.js";
import { Select } from "./ui/select.js";

let processing = false;

/** Opens the waiting requests one after another; asks to unlock first when the vault is locked. */
export async function processRequests() {
  if (processing) return;
  processing = true;
  try {
    for (;;) {
      let detail;
      try {
        detail = await invoke("next_request");
      } catch (e) {
        if (isLocked(e)) {
          update({ gateReason: "Your AI assistant sent a request. Unlock Blankey to continue." });
          await refreshStatus();
        } else toast(String(e), 5000);
        return;
      }
      if (!detail) return;
      const { request } = detail;
      await invoke("set_busy", { busy: true });
      try {
        if (request.kind === "profile_input") await openSheet((close) => html`<${ProfileInput} ...${detail} close=${close} />`);
        else if (request.kind === "file") await openSheet((close) => html`<${FileRequest} request=${request} close=${close} />`);
        else if (request.kind === "link_submission") {
          // a submission that cannot be opened is dropped by the backend, and the loop moves on
          const submission = await call("open_submission", { id: request.id }).catch(() => null);
          if (submission) await openSheet((close) => html`<${LinkSubmission} submission=${submission} close=${close} />`);
        } else if (request.kind === "collection_link") await openLinkSheet(request.payload, request.id);
        else if (request.kind === "fill") {
          const p = request.payload;
          await openFill({
            templates: p.templates || [],
            profiles: p.profiles || {},
            name: p.name || "",
            values: p.values || {},
            paths: p.paths || {},
            fill_set_id: p.fill_set_id ?? null,
            request_id: request.id,
          });
        } else {
          await invoke("cancel_request", { id: request.id });
        }
      } finally {
        await invoke("set_busy", { busy: false });
      }
      // still waiting: the sheet was closed without an answer (e.g. the vault locked)
      const still = await invoke("next_request").catch(() => null);
      if (still?.request.id === request.id) return;
    }
  } finally {
    processing = false;
    await refreshAfterRequests();
  }
}

export async function refreshAfterRequests() {
  const status = await refreshStatus();
  if (status.unlocked) reload();
}

const TRUE_WORDS = ["1", "true", "да", "yes", "x"];

/** The assistant asks for profile fields; the values are typed here and never sent back. */
function ProfileInput({ request, profile_name, existing, close }) {
  const payload = request.payload;
  const specs = payload.fields || [];
  const form = useRef();
  const [problems, setProblems] = useState([]);
  useEffect(() => form.current.querySelector("input")?.focus(), []);

  const cancel = async () => {
    await invoke("cancel_request", { id: request.id });
    close();
  };

  const save = async (e) => {
    e?.preventDefault();
    const elements = form.current.elements;
    const values = {};
    specs.forEach((spec, i) => {
      const input = elements[`f${i}`];
      values[spec.key] = input.type === "checkbox" ? (input.checked ? "yes" : "") : input.value;
    });
    const name = elements.__profile?.value ?? null;
    const result = await call("resolve_profile_input", { id: request.id, profileName: name, values });
    setProblems(result);
    if (!result.length) {
      toast("Saved");
      close();
    }
  };

  const field = (spec, i) => {
    const label = (spec.label || spec.key) + (spec.required ? " *" : "");
    const current = existing[spec.key] || "";
    if (spec.type === "bool") {
      return html`
        <label>${label}</label>
        <label class="check-row">
          <input type="checkbox" name=${`f${i}`} checked=${TRUE_WORDS.includes(current.trim().toLowerCase())} />
          <span class="muted small">${spec.hint || ""}</span>
        </label>`;
    }
    return html`
      <label title=${spec.key}>${label}</label>
      <input type="text" name=${`f${i}`} value=${current} placeholder=${spec.hint || PLACEHOLDERS[spec.type] || ""} spellcheck="false" />`;
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="person" title=${`${plural(specs.length, "field")} for ${profile_name || "a new profile"}`} reason=${payload.reason} />
    </div>
    <div class="sheet-body">
      <form class="input-form" autocomplete="off" ref=${form} onSubmit=${save}>
        ${payload.profile_id == null &&
        html`<label>New profile</label><input type="text" name="__profile" value=${payload.new_profile?.name || ""} />`}
        ${specs.map(field)}
      </form>
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problems.join("\n")}</div>
    </div>
    <div class="sheet-foot">
      <span class="note">Your data stays in Blankey. The AI assistant only sees the field names and how long the values are.</span>
      <button onClick=${cancel}>Cancel</button>
      <button class="primary" onClick=${() => save()}>Save</button>
    </div>`;
}

/** The assistant asks for a document; Blankey keeps a copy of the picked file and gives the assistant only its id. */
function FileRequest({ request, close }) {
  const { kind = "", reason } = request.payload;

  const cancel = async () => {
    await invoke("cancel_request", { id: request.id });
    close();
  };

  const choose = async () => {
    const file = await call("resolve_file_request", { id: request.id }).catch(() => null);
    if (!file) return;
    toast(`Shared ${file.name}`);
    close();
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="doc" title=${`Your AI assistant asks for a ${kind.toUpperCase()} file`} reason=${reason} />
    </div>
    <div class="sheet-body"></div>
    <div class="sheet-foot">
      <span class="note">Blankey keeps a copy of the file you choose. The assistant can read that file, nothing else on this computer.</span>
      <button onClick=${cancel}>Cancel</button>
      <button class="primary" onClick=${choose}>Choose file…</button>
    </div>`;
}

const TRUE_VALUE = (value) => TRUE_WORDS.includes(String(value).trim().toLowerCase());

/** Someone filled in a collection link: check what they sent, then save it into a profile. */
function LinkSubmission({ submission, close }) {
  const { link, fields, files } = submission;
  const profiles = useLoad(() => call("list_profiles"), []);
  const [values, setValues] = useState(submission.values);
  const [profileId, setProfileId] = useState(link.profile_id ?? "new");
  const [newName, setNewName] = useState(link.new_profile || "");
  const [keep, setKeep] = useState(files.map((f) => f.index));
  const [problems, setProblems] = useState([]);
  if (!profiles) return null;

  const discard = async () => {
    const text = "What was sent is deleted from this computer. The sender is not told.";
    if (!(await confirmDialog("Discard this submission?", text, "Discard"))) return;
    await call("cancel_request", { id: submission.request_id });
    close();
  };

  const save = async () => {
    const isNew = profileId === "new";
    const result = await call("resolve_submission", {
      id: submission.request_id,
      profileId: isNew ? null : Number(profileId),
      profileName: isNew ? newName : null,
      values,
      keep,
    });
    setProblems(result);
    if (!result.length) {
      toast("Saved");
      close();
    }
  };

  const set = (key, value) => setValues({ ...values, [key]: value });
  const field = (spec) => {
    const label = (spec.label || spec.key) + (spec.required ? " *" : "");
    if (spec.type === "bool") {
      return html`
        <label>${label}</label>
        <label class="check-row">
          <input type="checkbox" name=${spec.key} checked=${TRUE_VALUE(values[spec.key] || "")} onChange=${(e) => set(spec.key, e.target.checked ? "yes" : "")} />
        </label>`;
    }
    return html`
      <label title=${spec.key}>${label}</label>
      <input type="text" name=${spec.key} value=${values[spec.key] || ""} placeholder=${PLACEHOLDERS[spec.type] || ""} spellcheck="false"
        onInput=${(e) => set(spec.key, e.target.value)} />`;
  };

  const toggle = (index, on) => setKeep(on ? [...keep, index] : keep.filter((i) => i !== index));
  const options = [...profiles.map((p) => [String(p.id), p.name]), ["new", "New profile…"]];
  return html`
    <div class="sheet-head">
      <${SheetHead} icon="links" title=${`Submission for ${link.title}`}
        reason=${`${plural(fields.length, "field")}, ${plural(files.length, "file")}`} />
    </div>
    <div class="sheet-body">
      <form class="input-form" autocomplete="off" onSubmit=${(e) => (e.preventDefault(), save())}>
        <label>Save into</label>
        <${Select} name="profile" value=${profileId} options=${options} onChange=${(v) => setProfileId(v === "new" ? "new" : Number(v))} />
        ${profileId === "new" && html`
          <label>Profile name</label>
          <input type="text" name="__profile" value=${newName} onInput=${(e) => setNewName(e.target.value)} />`}
        ${fields.map(field)}
      </form>
      ${files.length > 0 && html`
        <div class="files-list">
          ${files.map(
            (f) => html`
              <label class="file-row" key=${f.index}>
                ${f.thumbnail ? html`<img class="thumb" src=${f.thumbnail} alt="" />` : html`<span class="thumb"></span>`}
                <span class="name" title=${f.name}>${f.name}</span>
                <span class="muted small">${megabytes(f.size)}</span>
                <span class="check-row small"><input type="checkbox" class="keep" checked=${keep.includes(f.index)} onChange=${(e) => toggle(f.index, e.target.checked)} /> Keep</span>
              </label>`
          )}
        </div>`}
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problems.join("\n")}</div>
    </div>
    <div class="sheet-foot">
      <span class="note">Kept files are encrypted in the vault and listed under Documents.</span>
      <button onClick=${discard} class="danger">Discard</button>
      <button onClick=${close}>Later</button>
      <button class="primary" onClick=${save}>Save</button>
    </div>`;
}
