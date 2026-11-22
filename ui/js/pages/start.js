// The Cases page with no case on it: the sample case to try Blankey on, or a first case from the user's own document.

import { call } from "../api.js";
import { t } from "../i18n.js";
import { html, Icon, useLoad } from "../lib.js";

/** Two people's details going into the blanks of a document. */
const Picture = () => html`
  <svg class="start-picture" viewBox="0 0 400 170" aria-hidden="true">
    <rect class="ground" width="400" height="170" />
    <circle class="shade" cx="70" cy="150" r="70" />
    <circle class="shade" cx="350" cy="20" r="56" />
    ${[38, 94].map(
      (y, i) => html`
        <g transform=${`translate(22 ${y})`}>
          <rect class="card" width="92" height="40" rx="10" />
          <circle class=${i ? "soft" : "accent"} cx="20" cy="20" r="11" />
          <circle class="ink" cx="20" cy="17" r="4" />
          <path class="ink" d="M12.5 27a8 8 0 0 1 15 0" />
          <rect class="line" x="38" y="13" width="42" height="5" rx="2.5" />
          <rect class="line faint" x="38" y="23" width="28" height="5" rx="2.5" />
        </g>`
    )}
    <path class="flow" d="M114 58C150 58 150 76 186 76" />
    <path class="flow" d="M114 114C150 114 150 98 186 98" />
    <g transform="rotate(-3 232 85)">
      <rect class="paper" x="176" y="20" width="112" height="134" rx="9" />
      <rect class="text" x="206" y="34" width="52" height="7" rx="3.5" />
      <rect class="rule" x="190" y="54" width="84" height="5" rx="2.5" />
      <rect class="text" x="190" y="70" width="22" height="5" rx="2.5" />
      <rect class="accent" x="218" y="66" width="56" height="13" rx="6.5" />
      <rect class="text" x="190" y="92" width="22" height="5" rx="2.5" />
      <rect class="soft" x="218" y="88" width="56" height="13" rx="6.5" />
      <rect class="rule" x="190" y="114" width="84" height="5" rx="2.5" />
      <rect class="rule" x="190" y="126" width="60" height="5" rx="2.5" />
    </g>
    <circle class="accent" cx="300" cy="134" r="17" />
    <path class="check" d="M291 134.5l6 6 12-13" />
    <path class="accent" d="M338 62l3.2 8.8 8.8 3.2-8.8 3.2-3.2 8.8-3.2-8.8-8.8-3.2 8.8-3.2z" />
    <path class="soft" d="M366 104l1.9 5.1 5.1 1.9-5.1 1.9-1.9 5.1-1.9-5.1-5.1-1.9 5.1-1.9z" />
  </svg>`;

/**
 * `onSample(id)` opens a sample case, `onOwn()` asks for the user's own document; `first` is the first use, when the
 * sample is the thing to press, and `over` is true while files are dragged over the page.
 */
export function Start({ first, over, onSample, onOwn }) {
  const samples = useLoad(() => call("list_samples"), []);
  const sample = samples?.[0];
  return html`
    <div class="start">
      <div class="start-head">
        ${first && html`<div class="start-label">${t("Welcome to Blankey")}</div>`}
        <h1>${t("Fill legal documents in minutes.")}</h1>
        <p>${t("The AI never sees your clients' data.")}</p>
      </div>
      <div class="start-choices">
        ${sample &&
        html`<div class="start-card" data-help="cases.sample">
          <${Picture} />
          <div>
            <h2>${t("Try the sample case")}</h2>
            <p>${sample.description}</p>
          </div>
          <button class=${`primary ${first ? "coach" : ""}`} onClick=${() => onSample(sample.id)}>${t("Open the sample case")}</button>
        </div>`}
        <div class=${`start-card start-own ${over ? "over" : ""}`} data-help="cases.own">
          <${Icon} name="upload" />
          <div>
            <h2>${t("Start with my own document")}</h2>
            <p>${t("Drop it here. A case is created and named after the file.")}</p>
          </div>
          <button onClick=${() => onOwn()}>${t("Choose a file")}</button>
        </div>
      </div>
    </div>`;
}
