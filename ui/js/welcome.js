// First use, after the recovery key: what Blankey Online adds and the way to sign in, or, for a user who stays
// offline, how an AI assistant on this computer connects. It ends on the Cases page, which points at the sample case.

import { useEffect, useState } from "../vendor/preact-htm.js";
import { call, refreshRelay } from "./api.js";
import { enterApp, LanguagePick, Logo } from "./gate.js";
import { t } from "./i18n.js";
import { html, Icon, useStore } from "./lib.js";
import { stateLabel } from "./pages/settings.js";
import { toast } from "./ui/feedback.js";

const ONLINE = [
  ["magic", () => t("An AI assistant inside every case")],
  ["person", () => t("ID cards and other known documents read straight into profiles")],
  ["links", () => t("Links that collect details and files from your clients")],
  ["connections", () => t("Web assistants such as claude.ai reach Blankey on this computer")],
];

/** Leaves the wizard for the Cases page, where New case makes the sample case. */
async function finish() {
  await call("first_use_set", { stage: "case" });
  await enterApp();
}

export function Welcome() {
  const [page, setPage] = useState("online");
  useEffect(() => {
    refreshRelay().catch(() => {});
  }, []);
  return html`
    <div class="gate welcome" data-tauri-drag-region>
      <${LanguagePick} />
      <div class="gate-card wide">
        <${Logo} />
        ${page === "online" ? html`<${Online} onOffline=${() => setPage("agent")} />` : html`<${Agent} onBack=${() => setPage("online")} />`}
      </div>
    </div>`;
}

function Online({ onOffline }) {
  const { relay } = useStore();
  const state = relay?.state || "signed_out";
  const waiting = state === "signing_in" || state === "connecting";
  const out = state === "signed_out" || waiting;
  const signIn = async () => {
    await call("relay_login");
    await refreshRelay();
  };
  return html`
    <h1>${t("Blankey is better online")}</h1>
    <p class="muted">${t("Your data stays encrypted on this computer. A Blankey Online account adds:")}</p>
    <ul class="welcome-points">
      ${ONLINE.map(([icon, text]) => html`<li><${Icon} name=${icon} /> ${text()}</li>`)}
    </ul>
    <div class="welcome-actions">
      ${out
        ? html`
            ${waiting && html`<p class="muted" id="welcome-waiting">${t("Finish signing in in your browser.")}</p>`}
            <button class="primary" id="welcome-signin" disabled=${waiting} onClick=${signIn}>${t("Sign in to Blankey Online")}</button>
            <button class="link" onClick=${onOffline}>${t("Continue offline")}</button>`
        : html`
            <p class="welcome-account">
              <span class=${`dot ${state === "connected" ? "up" : "down"}`}></span>
              ${state === "connected" ? relay.account : stateLabel(state)}
            </p>
            <button class="primary" onClick=${finish}>${t("Continue")}</button>`}
    </div>`;
}

function Agent({ onBack }) {
  const { status } = useStore();
  // ChatGPT reaches only addresses on the internet, which Blankey has through Blankey Online
  const [chatgpt, setChatgpt] = useState(false);
  const claude = async () => {
    const name = await call("connect_claude_desktop");
    toast(t("Blankey was added to {name}. Restart Claude Desktop to see it.", { name }), 5000);
  };
  const copy = async () => {
    await call("copy_text", { text: status.mcp_url });
    toast(t("Address copied."));
  };
  return html`
    <h1>${t("Connect an AI assistant")}</h1>
    <p class="muted">${t("Without Blankey Online, an assistant on this computer works with Blankey through MCP. Connect yours:")}</p>
    <div class="welcome-agents">
      <button onClick=${claude}>Claude Desktop</button>
      <button onClick=${() => setChatgpt(true)}>ChatGPT</button>
    </div>
    ${chatgpt &&
    html`<p class="muted" id="welcome-chatgpt">
      ${t("ChatGPT connects only to addresses on the internet. Sign in to Blankey Online, then add its connector URL from Settings in ChatGPT.")}${" "}
      <button class="link" onClick=${onBack}>${t("Sign in to Blankey Online")}</button>
    </p>`}
    <p class="welcome-address">
      <span class="muted">${t("Any other assistant:")}</span>
      <span class="mono selectable" id="welcome-mcp">${status.mcp_url}</span>
      <button class="link" onClick=${copy}>${t("Copy")}</button>
    </p>
    <div class="welcome-actions">
      <button class="primary" onClick=${finish}>${t("Continue")}</button>
      <button class="link" onClick=${onBack}>${t("Back")}</button>
    </div>`;
}
