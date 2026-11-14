import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke, isLocked, refreshStatus } from "./api.js";
import { openFill } from "./fill/form.js";
import { HelpNote } from "./help.js";
import { problemsAt, problemsElsewhere, t, tn } from "./i18n.js";
import { html, placeholder, reload, update } from "./lib.js";
import { openLinkSheet } from "./pages/links.js";
import { openSubmission } from "./submission.js";
import { toast } from "./ui/feedback.js";
import { PiiReview } from "./review.js";
import { openSheet, SheetHead, useSheetEscape } from "./ui/sheet.js";

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
          update({ gateReason: t("Your AI assistant sent a request. Unlock Blankey to continue.") });
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
          await openSubmission({ id: request.id });
        } else if (request.kind === "collection_link") await openLinkSheet(request.payload, request.id);
        else if (request.kind === "fill") {
          const p = request.payload;
          await openFill({
            templates: p.templates || [],
            profiles: p.profiles || {},
            name: p.name || "",
            values: p.values || {},
            paths: p.paths || {},
            ask: p.ask || {},
            fill_set_id: p.fill_set_id ?? null,
            copy_fill_set_id: p.copy_fill_set_id ?? null,
            request_id: request.id,
            case_id: p.case_id ?? null,
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

/** Where a requested field starts: the profile's value, or the assistant's suggestion when the profile has none. */
function startValue(spec, existing) {
  const existingValue = existing[spec.key] || "";
  const suggested = spec.suggested == null ? "" : String(spec.suggested);
  const prefill = !existingValue.trim() && suggested.trim() !== "";
  return { existingValue, suggested, prefill, current: prefill ? suggested : existingValue };
}

/** The assistant asks for profile fields; the values are typed here and never sent back. */
function ProfileInput({ request, profile_name, existing, close }) {
  const payload = request.payload;
  const specs = payload.fields || [];
  const form = useRef();
  // kept in state, so a save that is refused leaves what was typed in place
  const [values, setValues] = useState(() => Object.fromEntries(specs.map((spec) => [spec.key, startValue(spec, existing).current])));
  const [name, setName] = useState(payload.new_profile?.name || "");
  // the fields still holding the assistant's suggestion
  const [suggestedKeys, setSuggestedKeys] = useState(() => specs.filter((spec) => startValue(spec, existing).prefill).map((spec) => spec.key));
  const [problems, setProblems] = useState([]);
  useEffect(() => form.current.querySelector("input")?.focus(), []);

  const cancel = async () => {
    await invoke("cancel_request", { id: request.id });
    close();
  };
  useSheetEscape(cancel);

  const save = async (e) => {
    e?.preventDefault();
    const profileName = payload.profile_id == null ? name : null;
    const result = await call("resolve_profile_input", { id: request.id, profileName, values });
    setProblems(result);
    if (!result.length) {
      toast(t("Saved"));
      close();
    }
  };

  const set = (key, value, suggested = false) => {
    setValues({ ...values, [key]: value });
    setSuggestedKeys(suggested ? [...suggestedKeys, key] : suggestedKeys.filter((k) => k !== key));
  };
  const useSuggestion = (i, key, value) => {
    set(key, value, true);
    form.current.elements[`f${i}`].focus();
  };
  const problemLine = (key) => {
    const text = problemsAt(problems, key);
    return text && html`<div class="field-problem">${text}</div>`;
  };

  const field = (spec, i) => {
    const label = (spec.label || spec.key) + (spec.required ? " *" : "");
    const { existingValue, suggested, prefill } = startValue(spec, existing);
    const fromAssistant = suggestedKeys.includes(spec.key);
    const offer = !prefill && suggested.trim() !== "" && suggested !== existingValue;
    const invalid = problemsAt(problems, spec.key) !== "";
    if (spec.type === "bool") {
      return html`
        <label>${label}</label>
        <label class="check-row">
          <input type="checkbox" name=${`f${i}`} checked=${TRUE_WORDS.includes(values[spec.key].trim().toLowerCase())}
            onChange=${(e) => set(spec.key, e.target.checked ? "yes" : "")} />
          <span class="muted small">${spec.hint || ""}</span>
          ${prefill && html`<span class="chip Suggested">${t("Suggested")}</span>`}
        </label>
        ${problemLine(spec.key)}`;
    }
    return html`
      <label title=${spec.key}>${label}</label>
      <input type="text" name=${`f${i}`} class=${`${fromAssistant ? "suggested" : ""} ${invalid ? "invalid" : ""}`}
        title=${fromAssistant ? t("Suggested by the assistant") : ""}
        value=${values[spec.key]} placeholder=${spec.hint || placeholder(spec.type)} spellcheck="false"
        onInput=${(e) => set(spec.key, e.target.value)} />
      ${problemLine(spec.key)}
      ${offer &&
      html`<div class="hint">
        <button type="button" class="chip Suggested" data-help="request.suggestion" onClick=${() => useSuggestion(i, spec.key, suggested)}>
          ${t("Use suggestion: {value}", { value: suggested })}
        </button>
      </div>`}`;
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="person" title=${profile_name ? tn(specs.length, "{n} field for {profile}", "{n} fields for {profile}", { profile: profile_name }) : tn(specs.length, "{n} field for a new profile", "{n} fields for a new profile")} reason=${payload.reason} />
    </div>
    <div class="sheet-body">
      <form class="input-form" autocomplete="off" data-help="request.fields" ref=${form} onSubmit=${save}>
        ${payload.profile_id == null &&
        html`<label>${t("New profile")}</label>
          <input type="text" name="__profile" class=${problemsAt(problems, "__profile") ? "invalid" : ""} value=${name} onInput=${(e) => setName(e.target.value)} />
          ${problemLine("__profile")}`}
        ${specs.map(field)}
      </form>
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problemsElsewhere(problems, [...specs.map((spec) => spec.key), "__profile"])}</div>
    </div>
    <div class="sheet-foot">
      <${HelpNote}>${t("Your data stays in Blankey. The AI assistant only sees the field names and how long the values are.")}</${HelpNote}>
      <button data-help="sheet.cancel" onClick=${cancel}>${t("Cancel")}</button>
      <button class="primary" data-help="request.save" onClick=${() => save()}>${t("Save")}</button>
    </div>`;
}

/** The assistant asks for a document: one the user picks (Blankey keeps a copy and gives the assistant only its id),
 * or, with `source`, one from the user's folders that the assistant may read once allowed. */
function FileRequest({ request, close }) {
  const { kind = "", reason, source, path } = request.payload;
  const [picked, setPicked] = useState(null);

  const cancel = async () => {
    await invoke("cancel_request", { id: request.id });
    close();
  };
  useSheetEscape(cancel);

  const choose = async () => {
    const file = await call("pick_request_file", { id: request.id }).catch(() => null);
    if (file) setPicked(file);
  };

  const share = async () => {
    const shared = await call("share_upload", { id: request.id, path: picked.path }).catch(() => null);
    if (!shared) return;
    toast(t("Shared {name}", { name: picked.name }));
    close();
  };

  const allow = async () => {
    const shared = await call("share_library_request", { id: request.id }).catch(() => null);
    if (!shared) return;
    toast(t("Shared {name}", { name: path.split("/").pop() }));
    close();
  };

  if (source) {
    return html`
      <div class="sheet-head">
        <${SheetHead} icon="doc" title=${t("Your AI assistant asks to read a file from your folders")} reason=${reason} />
      </div>
      <div class="sheet-summary mono selectable shared-path">${path}</div>
      <div class="sheet-body"><${PiiReview} source=${source} /></div>
      <div class="sheet-foot">
        <${HelpNote}>${t("The assistant will read this file with the marked values hidden. Select text to hide it too.")}</${HelpNote}>
        <button data-help="sheet.cancel" onClick=${cancel}>${t("Cancel")}</button>
        <button class="primary" data-help="request.allow" onClick=${allow}>${t("Allow")}</button>
      </div>`;
  }

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="doc" title=${t("Your AI assistant asks for a {kind} file", { kind: kind.toUpperCase() })} reason=${reason} />
    </div>
    ${picked && html`<div class="sheet-summary mono selectable shared-path">${picked.path}</div>`}
    <div class="sheet-body">${picked && html`<${PiiReview} source=${picked.path} />`}</div>
    <div class="sheet-foot">
      <${HelpNote}>${picked
        ? t("The assistant will read this file with the marked values hidden. Select text to hide it too.")
        : t("Blankey keeps a copy of the file you choose. The assistant can read that file, nothing else on this computer.")}</${HelpNote}>
      <button data-help="sheet.cancel" onClick=${cancel}>${t("Cancel")}</button>
      <button class=${picked ? "" : "primary"} data-help="request.choose" onClick=${choose}>${picked ? t("Choose another…") : t("Choose file…")}</button>
      ${picked && html`<button class="primary" data-help="request.share" onClick=${share}>${t("Share")}</button>`}
    </div>`;
}
