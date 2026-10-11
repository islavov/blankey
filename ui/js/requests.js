import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke, isLocked, refreshStatus } from "./api.js";
import { openFill } from "./fill/form.js";
import { html, PLACEHOLDERS, plural, reload, update } from "./lib.js";
import { toast } from "./ui/feedback.js";
import { openSheet, SheetHead } from "./ui/sheet.js";

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
