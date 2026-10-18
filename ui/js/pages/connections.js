import { call, refreshRelay } from "../api.js";
import { t } from "../i18n.js";
import { html, matches, reload, useLoad, when } from "../lib.js";
import { confirmDialog, dialog, toast } from "../ui/feedback.js";
import { Empty, Page } from "../ui/page.js";
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

/** Web agents (claude.ai) reaching Blankey through a relay. */
export function Connections() {
  const info = useLoad(() => call("relay_status"), []);
  if (!info) return null;

  const state = info.signed_in || info.state === "signing_in" ? info.state : "signed_out";
  const signOut = async () => {
    await call("relay_logout");
    await refreshRelay();
    reload();
  };

  const useCustom = async () => {
    const values = await dialog({
      title: t("Use a custom relay"),
      text: t("Agents accepted through the current relay are removed. You then sign in to the new relay."),
      fields: [{ name: "url", label: t("Relay URL"), value: info.official ? "" : info.url }],
      ok: t("Use relay"),
    });
    if (!values?.url?.trim()) return;
    // errors are shown by call()
    await call("relay_set_url", { url: values.url.trim() }).catch(() => {});
    reload();
  };

  const resetOfficial = async () => {
    const text = t("Agents accepted through this relay are removed. You then sign in to the official relay.");
    if (!(await confirmDialog(t("Use the official relay?"), text, t("Use official relay")))) return;
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
  return html`
    <${Page} title=${t("Connections")} subtitle=${t("Web agents such as claude.ai, through a relay")}
      actions=${[info.signed_in ? [t("Sign out"), signOut] : [t("Sign in"), relaySignIn, "primary"]]}>
      <div class="relay">
        <div class="relay-row">
          <span class="muted">${t("Relay")}</span>
          <span class="mono selectable" id="relay-url">${info.url}</span>
          ${info.official && html`<span class="pill">${t("official")}</span>`}
          <span class="spacer"></span>
          <button onClick=${useCustom}>${t("Use custom relay…")}</button>
          ${!info.official && html`<button onClick=${resetOfficial}>${t("Reset to official")}</button>`}
        </div>
        ${info.mcp_url && html`
          <div class="relay-row">
            <span class="muted">${t("Connector URL")}</span>
            <span class="mono selectable" id="relay-mcp-url">${info.mcp_url}</span>
            <button class="link" onClick=${() => call("copy_text", { text: info.mcp_url }).then(() => toast(t("Copied.")))}>${t("Copy")}</button>
          </div>`}
        ${info.signed_in && info.account && html`
          <div class="relay-row">
            <span class="muted">${t("Account")}</span>
            <span class="selectable" id="relay-account">${info.account}</span>
          </div>`}
        <div class="relay-row">
          <span class="muted">${t("Status")}</span>
          <span class=${`dot ${state === "connected" ? "up" : state === "signed_out" ? "" : "down"}`}></span>
          <span id="relay-state">${stateLabel(state)}</span>
          ${info.message && html`<span class="muted small">${info.message}</span>`}
          <span class="spacer"></span>
          ${info.account_url && html`<button class="link" onClick=${() => call("relay_open_account")}>${t("Account and billing")}</button>`}
        </div>
      </div>
      ${rows.length
        ? html`<${Table}
            rows=${rows}
            onSelect=${() => {}}
            columns=${[
              { title: t("Agent"), cell: (g) => g.client_name },
              { title: t("Host"), cell: (g) => html`<span class="mono">${g.redirect_host}</span>` },
              { title: t("Accepted"), cell: (g) => html`<span class="muted">${when(g.accepted_at)}</span>`, cls: "nowrap" },
              { title: "", cell: (g) => (g.on_relay === false ? html`<span class="muted small">${t("gone from the relay")}</span>` : ""), cls: "nowrap" },
              { title: "", cell: (g) => html`<button class="danger" onClick=${() => revoke(g)}>${t("Revoke")}</button>`, cls: "nowrap" },
            ]} />`
        : html`<${Empty}>${t("No web agents yet.")}<br /><span class="small">${t("Add the connector URL as a custom connector in claude.ai, then allow it when Blankey asks.")}</span></${Empty}>`}
    </${Page}>`;
}
