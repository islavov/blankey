import { call, refreshRelay, refreshStatus } from "../api.js";
import { helpEnabled, setHelpEnabled } from "../help.js";
import { t } from "../i18n.js";
import { html, matches, reload, useLoad, useStore, when } from "../lib.js";
import { confirmDialog, dialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
import { Select } from "../ui/select.js";
import { Table } from "../ui/table.js";

export function stateLabel(state) {
  const labels = {
    signed_out: t("Signed out"),
    signing_in: t("Signing in…"),
    connecting: t("Connecting"),
    connected: t("Connected"),
    offline: t("Offline"),
    account_needs_attention: t("Account needs attention"),
  };
  return labels[state] || state;
}

export async function relaySignIn() {
  await call("relay_login");
  toast(t("Finish signing in in your browser."), 4000);
  await refreshRelay();
  reload();
}

function updateLabel(update) {
  const labels = {
    checking: t("Checking for updates…"),
    up_to_date: t("Up to date"),
    available: t("{version} is out", { version: update.version }),
    downloading: t("Downloading {version}…", { version: update.version }),
    ready: t("{version} is ready", { version: update.version }),
    failed: t("Could not check for updates"),
  };
  return labels[update.state] || "";
}

const copy = (text) => call("copy_text", { text }).then(() => toast(t("Copied.")));

/** This computer's settings, the local MCP server, and web agents (claude.ai) reaching Blankey through a relay. */
export function Settings() {
  const { status } = useStore();
  const info = useLoad(() => call("relay_status"), []);
  const telemetry = useLoad(() => call("telemetry_status"), []);
  const update = useLoad(() => call("update_status"), []);
  if (!info || telemetry === null || !update) return null;
  const busy = update.state === "checking" || update.state === "downloading";

  const state = info.signed_in || info.state === "signing_in" ? info.state : "signed_out";
  const signOut = async () => {
    await call("relay_logout");
    await refreshRelay();
    reload();
  };

  // the window switches when the backend announces the new language
  const setLanguage = async (code) => {
    await call("language_set", { language: code || null }).catch(() => {});
    await refreshStatus();
  };

  const useCustom = async () => {
    const values = await dialog({
      title: t("Use another online server"),
      text: t("Agents accepted through the current online server are removed. You then sign in to the new server."),
      fields: [{ name: "url", label: t("Online server URL"), value: info.official ? "" : info.url }],
      ok: t("Use server"),
    });
    if (!values?.url?.trim()) return;
    // errors are shown by call()
    await call("relay_set_url", { url: values.url.trim() }).catch(() => {});
    reload();
  };

  const resetOfficial = async () => {
    const text = t("Agents accepted through this online server are removed. You then sign in to the official server.");
    if (!(await confirmDialog(t("Use the official server?"), text, t("Use official server")))) return;
    await call("relay_set_url", { url: null }).catch(() => {});
    reload();
  };

  const revoke = async (grant) => {
    const text = t("It can no longer reach Blankey on this computer. Connect it again to ask for access.");
    if (!(await confirmDialog(t("Revoke {name}?", { name: grant.client_name }), text, t("Revoke")))) return;
    await call("relay_revoke", { grantId: grant.grant_id });
    reload();
  };

  const rows = info.grants.map((g) => ({ ...g, id: g.grant_id })).filter((g) => matches(g.client_name, g.redirect_host));
  const languages = [["", t("Default")], ...Object.entries(status.languages)];
  return html`
    <${Page} title=${t("Settings")}>
      <div class="relay">
        <div class="relay-head">${t("General")}</div>
        <div class="relay-row" data-help="settings.language">
          <span class="muted">${t("Language")}</span>
          <${Select} id="language" value=${status.device_language ?? ""} options=${languages} onChange=${setLanguage} />
        </div>
        <div class="relay-row" data-help="settings.version">
          <span class="muted">${t("Version")}</span>
          <span class="mono selectable" id="app-version">${update.current}</span>
          <span id="update-state">${updateLabel(update)}</span>
          ${update.state === "failed" && html`<span class="muted small">${update.message}</span>`}
          <span class="spacer"></span>
          ${update.state === "ready" && html`<button class="primary" onClick=${() => call("update_install")}>${t("Restart to update")}</button>`}
          ${update.state === "available" && html`<button class="primary" onClick=${() => call("update_install")}>${t("Download")}</button>`}
          <button disabled=${busy} onClick=${() => call("update_check").then(reload, reload)}>${t("Check now")}</button>
        </div>
        <div class="relay-row" data-help="settings.updates">
          <span class="muted">${t("Updates")}</span>
          <label class="check small">
            <input type="checkbox" id="auto-update" checked=${update.auto} onChange=${(e) => call("update_auto_set", { enabled: e.target.checked }).then(reload, reload)} />
            ${t("Check for new versions automatically")}
          </label>
        </div>
        <div class="relay-row" data-help="settings.statistics">
          <span class="muted">${t("Statistics")}</span>
          <label class="check small">
            <input type="checkbox" id="telemetry" checked=${telemetry} onChange=${(e) => call("telemetry_set", { enabled: e.target.checked }).then(reload, reload)} />
            ${t("Share anonymous usage statistics")}
          </label>
          <span class="muted small">${t("How many documents, templates, fill sets and profiles Blankey makes each month, its version and platform, under a random id. Never names or contents.")}</span>
        </div>
        <div class="relay-row" data-help="settings.help">
          <span class="muted">${t("Help")}</span>
          <label class="check small">
            <input type="checkbox" id="help-box" checked=${helpEnabled()} onChange=${(e) => setHelpEnabled(e.target.checked)} />
            ${t("Show the help box in the sidebar")}
          </label>
        </div>
        <div class="relay-row" data-help="settings.mcp">
          <span class="muted">${t("MCP server")}</span>
          <span class=${`dot ${status.mcp_running ? "up" : "down"}`}></span>
          <span id="mcp-state">${status.mcp_running ? t("MCP on port {port}", { port: new URL(status.mcp_url).port }) : t("MCP is down")}</span>
          <span class="mono selectable" id="mcp-url">${status.mcp_url}</span>
          <button class="link" onClick=${() => copy(status.mcp_url)}>${t("Copy")}</button>
        </div>
      </div>
      <div class="relay">
        <div class="relay-head">${t("Blankey Online")}</div>
        <div class="relay-row" data-help="settings.relay">
          <span class="muted">${t("Online server")}</span>
          <span class="mono selectable" id="relay-url">${info.url}</span>
          ${info.official && html`<span class="pill">${t("official")}</span>`}
          <span class="spacer"></span>
          ${info.signed_in
            ? html`<button onClick=${signOut}>${t("Sign out")}</button>`
            : html`<button class="primary" onClick=${relaySignIn}>${t("Sign in")}</button>`}
          <button onClick=${useCustom}>${t("Another online server…")}</button>
          ${!info.official && html`<button onClick=${resetOfficial}>${t("Reset to official")}</button>`}
        </div>
        ${info.mcp_url && html`
          <div class="relay-row" data-help="settings.connector">
            <span class="muted">${t("Connector URL")}</span>
            <span class="mono selectable" id="relay-mcp-url">${info.mcp_url}</span>
            <button class="link" onClick=${() => copy(info.mcp_url)}>${t("Copy")}</button>
          </div>`}
        ${info.signed_in && info.account && html`
          <div class="relay-row" data-help="settings.account">
            <span class="muted">${t("Account")}</span>
            <span class="selectable" id="relay-account">${info.account}</span>
          </div>`}
        <div class="relay-row" data-help="settings.status">
          <span class="muted">${t("Status")}</span>
          <span class=${`dot ${state === "connected" ? "up" : state === "signed_out" ? "" : "down"}`}></span>
          <span id="relay-state">${stateLabel(state)}</span>
          ${info.message && html`<span class="muted small">${info.message}</span>`}
          <span class="spacer"></span>
          ${info.account_url && html`<button class="link" onClick=${() => call("relay_open_account")}>${t("Account and billing")}</button>`}
        </div>
      </div>
      ${rows.length
        ? html`<div data-help="settings.agents"><${Table}
            rows=${rows}
            columns=${[
              { title: t("Agent"), cell: (g) => g.client_name },
              { title: t("Host"), cell: (g) => html`<span class="mono">${g.redirect_host}</span>` },
              { title: t("Accepted"), cell: (g) => html`<span class="muted">${when(g.accepted_at)}</span>`, cls: "nowrap" },
              { title: "", cell: (g) => (g.on_relay === false ? html`<span class="muted small">${t("gone from Blankey Online")}</span>` : ""), cls: "wide nowrap" },
              { title: "", cell: (g) => html`<button class="danger" onClick=${() => revoke(g)}>${t("Revoke")}</button>`, cls: "nowrap end" },
            ]} /></div>`
        : html`<${Empty}>${t("No web agents yet.")}<br /><span class="small">${t("Add the connector URL as a custom connector in claude.ai, then allow it when Blankey asks.")}</span></${Empty}>`}
    </${Page}>`;
}
