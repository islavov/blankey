// A recognized case file beside the profile it became: what the profile holds, what the document says, and where.

import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { call } from "./api.js";
import { HelpNote } from "./help.js";
import { problemsAt, problemsElsewhere, t } from "./i18n.js";
import { html, placeholder, reload } from "./lib.js";
import { Preview } from "./preview.js";
import { toast } from "./ui/feedback.js";
import { openSheet, SheetHead } from "./ui/sheet.js";

/** Opens the check of a recognized case file. Resolves when it closes. */
export const openDocumentCheck = (file) => openSheet((close) => html`<${DocumentCheck} file=${file} close=${close} />`);

/**
 * The profile's fields on the left, the document with a box on each value on the right. A field whose value in the
 * document differs from the profile's, or was left out because it failed its check, offers the document's value.
 */
function DocumentCheck({ file, close }) {
  const [check, setCheck] = useState(null);
  const [values, setValues] = useState({});
  // the field whose place in the document is outlined; a new seq scrolls it into view
  const [shown, setShown] = useState({ key: null, seq: 0 });
  const [problems, setProblems] = useState([]);
  const form = useRef();

  useEffect(() => {
    call("check_document", { id: file.id })
      .then((loaded) => {
        setValues(Object.fromEntries(loaded.fields.map((f) => [f.key, f.value])));
        setCheck(loaded);
      })
      .catch(close);
  }, []);

  const set = (key, value) => setValues((current) => ({ ...current, [key]: value }));
  const show = (key) => setShown((current) => ({ key, seq: current.seq + 1 }));
  const save = async (e) => {
    e?.preventDefault();
    const result = await call("save_document_check", { id: file.id, values });
    setProblems(result);
    if (result.length) return;
    toast(t("Saved"));
    close();
    reload();
  };

  const field = (spec) => {
    const value = values[spec.key] ?? "";
    const failed = spec.status === "failed";
    const offer = spec.read.trim() !== "" && spec.read !== value;
    const problem = problemsAt(problems, spec.key);
    return html`
      <label title=${spec.key}>${spec.label}</label>
      <input type="text" name=${spec.key} class=${problem ? "invalid" : ""} value=${value} placeholder=${placeholder(spec.type)} spellcheck="false"
        onFocus=${() => show(spec.key)} onInput=${(e) => set(spec.key, e.target.value)} />
      ${offer &&
      html`<div class="hint">
        <button type="button" class=${`chip ${failed ? "Failed" : "Suggested"}`} data-help=${failed ? "doccheck.failed" : "doccheck.read"}
          onClick=${() => set(spec.key, spec.read)}>
          ${failed ? t("Failed its check: {value}", { value: spec.read }) : t("In the document: {value}", { value: spec.read })}
        </button>
      </div>`}
      ${problem && html`<div class="field-problem">${problem}</div>`}`;
  };

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="person" title=${check?.profile.name ?? (file.description || file.name)} reason=${check?.document ?? ""} />
    </div>
    <div class="sheet-body">
      ${check
        ? html`
            <div class="fill-split doc-check">
              <div class="fill-rows">
                <form class="input-form" autocomplete="off" data-help="doccheck.fields" ref=${form} onSubmit=${save}>${check.fields.map(field)}</form>
                <div class="problems">${problemsElsewhere(problems, check.fields.map((f) => f.key))}</div>
              </div>
              <div class="fill-preview">
                <${Preview} preview=${check.preview} active=${shown.key} revealSeq=${shown.seq} help="doccheck.preview" dragMove
                  onPick=${(key) => form.current.elements[key]?.focus()} />
              </div>
            </div>`
        : html`<div class="preview-area"><div class="preview-error">${t("Loading preview…")}</div></div>`}
    </div>
    <div class="sheet-foot">
      <${HelpNote}>${t("Click a field to see where the document has it.")}</${HelpNote}>
      <button data-help="doccheck.cancel" onClick=${close}>${t("Cancel")}</button>
      <button class="primary" data-help="doccheck.save" disabled=${!check} onClick=${() => save()}>${t("Save")}</button>
    </div>`;
}
