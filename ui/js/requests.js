import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke, isLocked, refreshStatus } from "./api.js";
import { openFill } from "./fill/form.js";
import { problemText, t, tn } from "./i18n.js";
import { html, placeholder, reload, update } from "./lib.js";
import { openLinkSheet } from "./pages/links.js";
import { openSubmission } from "./submission.js";
import { toast } from "./ui/feedback.js";
import { PiiReview } from "./review.js";
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
      toast(t("Saved"));
      close();
    }
  };

  const useSuggestion = (i, value) => {
    const input = form.current.elements[`f${i}`];
    input.value = value;
    input.classList.add("suggested");
    input.focus();
  };

  const field = (spec, i) => {
    const label = (spec.label || spec.key) + (spec.required ? " *" : "");
    const existingValue = existing[spec.key] || "";
    const suggested = spec.suggested == null ? "" : String(spec.suggested);
    const prefill = !existingValue.trim() && suggested.trim() !== "";
    const current = prefill ? suggested : existingValue;
    const offer = !prefill && suggested.trim() !== "" && suggested !== existingValue;
    if (spec.type === "bool") {
      return html`
        <label>${label}</label>
        <label class="check-row">
          <input type="checkbox" name=${`f${i}`} checked=${TRUE_WORDS.includes(current.trim().toLowerCase())} />
          <span class="muted small">${spec.hint || ""}</span>
          ${prefill && html`<span class="chip Suggested">${t("Suggested")}</span>`}
        </label>`;
    }
    return html`
      <label title=${spec.key}>${label}</label>
      <input type="text" name=${`f${i}`} class=${prefill ? "suggested" : ""} title=${prefill ? t("Suggested by the assistant") : ""}
        value=${current} placeholder=${spec.hint || placeholder(spec.type)} spellcheck="false"
        onInput=${(e) => e.target.classList.remove("suggested")} />
      ${offer &&
      html`<div class="hint">
        <button type="button" class="chip Suggested" onClick=${() => useSuggestion(i, suggested)}>
          ${t("Use suggestion: {value}", { value: suggested })}
        </button>
      </div>`}`;
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="person" title=${profile_name ? tn(specs.length, "{n} field for {profile}", "{n} fields for {profile}", { profile: profile_name }) : tn(specs.length, "{n} field for a new profile", "{n} fields for a new profile")} reason=${payload.reason} />
    </div>
    <div class="sheet-body">
      <form class="input-form" autocomplete="off" ref=${form} onSubmit=${save}>
        ${payload.profile_id == null &&
        html`<label>${t("New profile")}</label><input type="text" name="__profile" value=${payload.new_profile?.name || ""} />`}
        ${specs.map(field)}
      </form>
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problems.map(problemText).join("\n")}</div>
    </div>
    <div class="sheet-foot">
      <span class="note">${t("Your data stays in Blankey. The AI assistant only sees the field names and how long the values are.")}</span>
      <button onClick=${cancel}>${t("Cancel")}</button>
      <button class="primary" onClick=${() => save()}>${t("Save")}</button>
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
        <span class="note">${t("The assistant will read this file with the marked values hidden. Select text to hide it too.")}</span>
        <button onClick=${cancel}>${t("Cancel")}</button>
        <button class="primary" onClick=${allow}>${t("Allow")}</button>
      </div>`;
  }

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="doc" title=${t("Your AI assistant asks for a {kind} file", { kind: kind.toUpperCase() })} reason=${reason} />
    </div>
    ${picked && html`<div class="sheet-summary mono selectable shared-path">${picked.path}</div>`}
    <div class="sheet-body">${picked && html`<${PiiReview} source=${picked.path} />`}</div>
    <div class="sheet-foot">
      <span class="note">${picked
        ? t("The assistant will read this file with the marked values hidden. Select text to hide it too.")
        : t("Blankey keeps a copy of the file you choose. The assistant can read that file, nothing else on this computer.")}</span>
      <button onClick=${cancel}>${t("Cancel")}</button>
      <button class=${picked ? "" : "primary"} onClick=${choose}>${picked ? t("Choose another…") : t("Choose file…")}</button>
      ${picked && html`<button class="primary" onClick=${share}>${t("Share")}</button>`}
    </div>`;
}
