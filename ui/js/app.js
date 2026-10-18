import { useEffect, useState } from "../vendor/preact-htm.js";
import { invoke, refreshRelay } from "./api.js";
import { t } from "./i18n.js";
import { html, Icon, update, useStore } from "./lib.js";
import { Activity } from "./pages/activity.js";
import { Connections, relaySignIn, stateLabel } from "./pages/connections.js";
import { Documents } from "./pages/documents.js";
import { FillSets } from "./pages/fillsets.js";
import { Links } from "./pages/links.js";
import { Profiles } from "./pages/profiles.js";
import { Templates } from "./pages/templates.js";
import { currentTheme, setTheme } from "./theme.js";

// titles are functions: they are read at render time, in the current language
const PAGES = {
  activity: { title: () => t("Activity"), component: Activity },
  connections: { title: () => t("Connections"), component: Connections },
  fillsets: { title: () => t("Fill sets"), component: FillSets },
  documents: { title: () => t("Documents"), component: Documents },
  templates: { title: () => t("Templates"), component: Templates },
  profiles: { title: () => t("Profiles"), component: Profiles },
  links: { title: () => t("Collection links"), component: Links },
};

const SECTIONS = [
  [() => t("Assistant"), ["activity", "connections"]],
  [() => t("Library"), ["fillsets", "documents", "templates"]],
  [() => t("Vault"), ["profiles", "links"]],
];

export function App() {
  const { status, page } = useStore();
  const Current = PAGES[page].component;
  return html`
    <div class="app">
      <aside class="sidebar">
        <div class="sidebar-top" data-tauri-drag-region></div>
        <nav id="nav">${SECTIONS.map(([section, ids]) => html`
          <div class="nav-section">${section()}</div>
          ${ids.map((id) => html`<${NavItem} id=${id} active=${id === page} pending=${id === "activity" && status.pending} />`)}`)}
        </nav>
        <div class="sidebar-foot">
          <${McpStatus} status=${status} />
          <div class="foot-actions">
            <button id="lock-btn" title=${t("Lock the vault")} onClick=${() => invoke("lock")}><${Icon} name="lock" /> ${t("Lock")}</button>
            <${RelayButton} />
            <${ThemeSwitch} />
          </div>
        </div>
      </aside>
      <main class="main"><${Current} key=${page} /></main>
    </div>`;
}

function NavItem({ id, active, pending }) {
  return html`
    <div class=${`nav-item ${active ? "active" : ""}`} onClick=${() => update({ page: id, search: "" })}>
      <${Icon} name=${id} />${PAGES[id].title()}
      ${pending > 0 && html`<span class="badge">${pending}</span>`}
    </div>`;
}

const McpStatus = ({ status }) => html`
  <div class="mcp-status" title=${status.mcp_url}>
    <span class=${`dot ${status.mcp_running ? "up" : "down"}`}></span>
    ${status.mcp_running ? t("MCP on port {port}", { port: new URL(status.mcp_url).port }) : t("MCP is down")}
  </div>`;

/** Relay sign-in next to Lock: "Sign in" while signed out, the account's user name once signed in. */
function RelayButton() {
  const { relay } = useStore();
  useEffect(() => {
    refreshRelay().catch(() => {});
  }, []);
  if (!relay) return null;
  if (relay.state === "signing_in") {
    return html`<button id="relay-btn" disabled><${Icon} name="connections" /> ${t("Signing in…")}</button>`;
  }
  if (!relay.signed_in) {
    return html`
      <button id="relay-btn" title=${t("Sign in to the relay so web agents such as claude.ai can reach Blankey")}
        onClick=${() => relaySignIn().catch(() => {})}><${Icon} name="connections" /> ${t("Sign in")}</button>`;
  }
  const name = relay.account ? relay.account.split("@")[0] : t("Signed in");
  const title = `${relay.account ?? t("Relay")}: ${stateLabel(relay.state)}`;
  const dot = relay.state === "connected" ? "up" : "down";
  return html`
    <button id="relay-btn" title=${title} onClick=${() => update({ page: "connections", search: "" })}>
      <span class=${`dot ${dot}`}></span><span class="relay-name">${name}</span>
    </button>`;
}

function ThemeSwitch() {
  const [theme, setThemeState] = useState(currentTheme());
  const next = theme === "dark" ? "light" : "dark";
  const switchLabel = next === "dark" ? t("Switch to dark mode") : t("Switch to light mode");
  const toggle = () => {
    setTheme(next);
    setThemeState(next);
  };
  return html`
    <button class="icon-btn" title=${switchLabel} aria-label=${switchLabel} onClick=${toggle}>
      <${Icon} name=${theme === "dark" ? "sun" : "moon"} />
    </button>`;
}
