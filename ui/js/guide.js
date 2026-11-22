// The guide of a sample case: where the user is on the way to a first document, and what to ask the assistant next.
// The backend says which steps there are and which one is current; it reads that from what the case holds, so the
// guide moves the same way whether the built-in assistant or the user's own does the work.

import { call } from "./api.js";
import { openFill } from "./fill/form.js";
import { t } from "./i18n.js";
import { html, reload, state, update, useLoad, useStore } from "./lib.js";
import { processRequests } from "./requests.js";
import { toast } from "./ui/feedback.js";
import { openDocuments } from "./viewer.js";

/** The fill form of the case: the assistant's waiting request, else the newest fill set, else the case's template. */
async function openFillForm(info) {
  if (state.status.pending) return processRequests();
  const [set] = await call("list_fill_sets", { caseId: info.id });
  if (set) await openFill({ fill_set_id: set.id });
  else {
    const templates = (await call("list_templates", { caseId: info.id })).filter((tpl) => tpl.local);
    if (!templates.length) return toast(t("Ask the assistant to fill in the contract first."));
    await openFill({ templates: [templates[0].id], case_id: info.id });
  }
  reload();
}

/** The steps as a row: done, current and to come. */
const Steps = ({ guide }) => html`
  <ol class="guide-steps">
    ${guide.steps.map(
      (step, i) => html`
        <li class=${i < guide.step ? "done" : i === guide.step ? "current" : ""}>
          <span class="guide-dot">${i < guide.step ? "✓" : i + 1}</span>${step.title}
        </li>`
    )}
  </ol>`;

/** The last step is done: the document, and the way on to a case of the user's own. */
function Done({ info, onOwn }) {
  const docs = useLoad(() => call("list_documents", { caseId: info.id }), [info.id]);
  const doc = docs?.[0];
  return html`
    <div class="guide-body">
      <div class="guide-text">
        <h3>${t("Your first document is ready")}</h3>
        ${doc && html`<p>${doc.title}</p>`}
      </div>
      <div class="guide-actions">
        ${doc && html`<button class="primary" data-help="guide.open" onClick=${() => openDocuments([doc.id])}>${t("Open document")}</button>`}
        <button data-help="guide.own" onClick=${onOwn}>${t("Try with my own document")}</button>
      </div>
    </div>`;
}

/** The current step: what it is for and, when it has one, the prompt to give the assistant. */
function Step({ info, step }) {
  const { relay } = useStore();
  const chat = relay?.chat === true;
  const ask = () => {
    update({ caseTab: "chat" });
    call("send_chat_message", { caseId: info.id, text: step.prompt }).catch(() => {});
  };
  // an assistant outside Blankey has to be told which case
  const copy = async () => {
    const text = chat ? step.prompt : `${t("In Blankey, open the case “{title}”.", { title: info.title })} ${step.prompt}`;
    await call("copy_text", { text });
    toast(t("Copied"));
  };
  return html`
    <div class="guide-body">
      <div class="guide-text">
        <h3>${step.heading}</h3>
        <p>${step.text}</p>
      </div>
      ${step.prompt &&
      html`<div class="guide-actions">
        <div class="guide-prompt selectable" data-help="guide.prompt"><span>${t("Prompt")}</span>${step.prompt}</div>
        ${chat && html`<button class="primary" data-help="guide.ask" onClick=${ask}>${t("Ask the assistant")}</button>`}
        <button class=${chat ? "" : "primary"} data-help="guide.copy" onClick=${copy}>${t("Copy")}</button>
      </div>`}
      ${step.action === "open_fill" &&
      html`<div class="guide-actions">
        <button class="primary" data-help="guide.fill" onClick=${() => openFillForm(info)}>${t("Open the fill form")}</button>
      </div>`}
    </div>`;
}

/** `info` is the case as get_case gives it, with its guide; `onOwn` starts a case from the user's own document. */
export function CaseGuide({ info, onOwn }) {
  const { guide } = info;
  const hide = async () => {
    await call("sample_guide_hide", { caseId: info.id });
    reload();
  };
  const step = guide.steps[guide.step];
  return html`
    <section class="guide">
      <div class="guide-top" data-help="guide.steps">
        <${Steps} guide=${guide} />
        <button class="link" data-help="guide.hide" onClick=${hide}>${t("Hide guide")}</button>
      </div>
      ${step ? html`<${Step} info=${info} step=${step} />` : html`<${Done} info=${info} onOwn=${onOwn} />`}
    </section>`;
}
