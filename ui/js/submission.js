import { useState } from "../vendor/preact-htm.js";
import { call } from "./api.js";
import { HelpNote } from "./help.js";
import { problemText, t, tn } from "./i18n.js";
import { html, megabytes, placeholder, useLoad, when } from "./lib.js";
import { confirmDialog, toast } from "./ui/feedback.js";
import { openSheet, SheetHead } from "./ui/sheet.js";
import { Select } from "./ui/select.js";

const TRUE_WORDS = ["1", "true", "да", "yes", "x"];

/**
 * Opens a submission to a collection link: `{id}` for a waiting request, `{sid}` from the Collection links page.
 * Resolves when the sheet closes; a submission that cannot be opened shows a toast instead.
 */
export async function openSubmission(which) {
  const submission = await call("open_submission", which).catch(() => null);
  if (submission) await openSheet((close) => html`<${SubmissionSheet} submission=${submission} close=${close} />`);
}

/** What someone sent through a collection link: check it, save it into a profile, open or export its files. */
function SubmissionSheet({ submission, close }) {
  const { sid, link, fields, files, status } = submission;
  const saved = status === "saved";
  const profiles = useLoad(() => call("list_profiles"), []);
  const [values, setValues] = useState(submission.values);
  const [profileId, setProfileId] = useState(submission.profile_id ?? link.profile_id ?? "new");
  const [newName, setNewName] = useState(link.new_profile || "");
  // files already kept the first time are not ticked again, so saving again does not duplicate them
  const [keep, setKeep] = useState(saved ? [] : files.map((f) => f.index));
  const [problems, setProblems] = useState([]);
  if (!profiles) return null;

  const discard = async () => {
    const text = t("What was sent is deleted from this computer and can no longer be opened. Values and files you saved stay.");
    if (!(await confirmDialog(t("Discard this submission?"), text, t("Discard")))) return;
    await call("discard_submission", { sid });
    close();
  };

  const save = async () => {
    const isNew = profileId === "new";
    const result = await call("resolve_submission", {
      sid,
      profileId: isNew ? null : Number(profileId),
      profileName: isNew ? newName : null,
      values,
      keep,
    });
    setProblems(result);
    if (!result.length) {
      toast(t("Saved"));
      close();
    }
  };

  const exportFile = async (index) => {
    const path = await call("export_submission_file", { sid, index });
    if (path) toast(t("Saved to {path}", { path }), 4000);
  };

  const set = (key, value) => setValues({ ...values, [key]: value });
  const field = (spec) => {
    const label = (spec.label || spec.key) + (spec.required ? " *" : "");
    if (spec.type === "bool") {
      const checked = TRUE_WORDS.includes(String(values[spec.key] || "").trim().toLowerCase());
      return html`
        <label>${label}</label>
        <label class="check-row">
          <input type="checkbox" name=${spec.key} checked=${checked} onChange=${(e) => set(spec.key, e.target.checked ? "yes" : "")} />
        </label>`;
    }
    return html`
      <label title=${spec.key}>${label}</label>
      <input type="text" name=${spec.key} value=${values[spec.key] || ""} placeholder=${placeholder(spec.type)} spellcheck="false"
        onInput=${(e) => set(spec.key, e.target.value)} />`;
  };

  const toggle = (index, on) => setKeep(on ? [...keep, index] : keep.filter((i) => i !== index));
  const options = [...profiles.map((p) => [String(p.id), p.name]), ["new", t("New profile…")]];
  const summary = [
    t("Received {date}", { date: when(submission.received_at) }),
    saved && t("saved {date}", { date: when(submission.handled_at) }),
    `${tn(fields.length, "{n} field", "{n} fields")}, ${tn(files.length, "{n} file", "{n} files")}`,
  ].filter(Boolean);
  return html`
    <div class="sheet-head">
      <${SheetHead} icon="links" title=${t("Submission for {title}", { title: link.title })} reason=${summary.join(" · ")} />
    </div>
    <div class="sheet-body">
      <form class="input-form" autocomplete="off" onSubmit=${(e) => (e.preventDefault(), save())}>
        <label data-help="submission.profile">${t("Save into")}</label>
        <${Select} name="profile" data-help="submission.profile" value=${profileId} options=${options} onChange=${(v) => setProfileId(v === "new" ? "new" : Number(v))} />
        ${profileId === "new" && html`
          <label>${t("Profile name")}</label>
          <input type="text" name="__profile" value=${newName} onInput=${(e) => setNewName(e.target.value)} />`}
        ${fields.map(field)}
      </form>
      ${files.length > 0 && html`
        <div class="files-list" data-help="submission.files">
          ${files.map(
            (f) => html`
              <div class="file-row" key=${f.index}>
                ${f.thumbnail ? html`<img class="thumb" src=${f.thumbnail} alt="" />` : html`<span class="thumb"></span>`}
                <span class="name" title=${f.name}>${f.name}</span>
                <span class="muted small">${megabytes(f.size)}</span>
                <button class="link" onClick=${() => call("open_submission_file", { sid, index: f.index })}>${t("Open")}</button>
                <button class="link" onClick=${() => exportFile(f.index)}>${t("Export")}</button>
                <label class="check-row small" title=${t("Keep an encrypted copy with the profile")}>
                  <input type="checkbox" class="keep" checked=${keep.includes(f.index)} onChange=${(e) => toggle(f.index, e.target.checked)} /> ${t("Keep")}
                </label>
              </div>`
          )}
        </div>`}
      <div class="problems" style="max-width:640px;margin:0 auto;padding:0 24px">${problems.map(problemText).join("\n")}</div>
    </div>
    <div class="sheet-foot">
      <${HelpNote}>${t("Kept files are encrypted and stored with the profile. This submission stays here until you discard it.")}</${HelpNote}>
      <button onClick=${discard} class="danger" data-help="submission.discard">${t("Discard")}</button>
      <button data-help=${saved ? undefined : "submission.later"} onClick=${close}>${saved ? t("Close") : t("Later")}</button>
      <button class="primary" data-help="submission.save" onClick=${save}>${saved ? t("Save again") : t("Save")}</button>
    </div>`;
}
