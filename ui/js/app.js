import { useEffect, useState } from "../vendor/preact-htm.js";
import { invoke, refreshRelay } from "./api.js";
import { html, Icon, update, useStore } from "./lib.js";
import { Activity } from "./pages/activity.js";
import { Connections, relaySignIn, STATES } from "./pages/connections.js";
import { Documents } from "./pages/documents.js";
import { FillSets } from "./pages/fillsets.js";
import { Profiles } from "./pages/profiles.js";
import { Templates } from "./pages/templates.js";
import { currentTheme, setTheme } from "./theme.js";

const PAGES = {
  activity: { title: "Activity", component: Activity },
  connections: { title: "Connections", component: Connections },
  fillsets: { title: "Fill sets", component: FillSets },
  documents: { title: "Documents", component: Documents },
  templates: { title: "Templates", component: Templates },
  profiles: { title: "Profiles", component: Profiles },
};

const SECTIONS = [
  ["Assistant", ["activity", "connections"]],
  ["Library", ["fillsets", "documents", "templates"]],
  ["Vault", ["profiles"]],
];

export function App() {
  const { status, page } = useStore();
  const Current = PAGES[page].component;
  return html`
    <div class="app">
      <aside class="sidebar">
        <div class="sidebar-top" data-tauri-drag-region></div>
        <nav id="nav">${SECTIONS.map(([section, ids]) => html`
          <div class="nav-section">${section}</div>
          ${ids.map((id) => html`<${NavItem} id=${id} active=${id === page} pending=${id === "activity" && status.pending} />`)}`)}
        </nav>
        <div class="sidebar-foot">
          <${McpStatus} status=${status} />
          <div class="foot-actions">
            <button id="lock-btn" title="Lock the vault" onClick=${() => invoke("lock")}><${Icon} name="lock" /> Lock</button>
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
      <${Icon} name=${id} />${PAGES[id].title}
      ${pending > 0 && html`<span class="badge">${pending}</span>`}
    </div>`;
}

const McpStatus = ({ status }) => html`
  <div class="mcp-status" title=${status.mcp_url}>
    <span class=${`dot ${status.mcp_running ? "up" : "down"}`}></span>
    ${status.mcp_running ? `MCP on port ${new URL(status.mcp_url).port}` : "MCP is down"}
  </div>`;

/** Relay sign-in next to Lock: "Sign in" while signed out, the account's user name once signed in. */
function RelayButton() {
  const { relay } = useStore();
  useEffect(() => {
    refreshRelay().catch(() => {});
  }, []);
  if (!relay) return null;
  if (relay.state === "signing_in") {
    return html`<button id="relay-btn" disabled><${Icon} name="connections" /> Signing in…</button>`;
  }
  if (!relay.signed_in) {
    return html`
      <button id="relay-btn" title="Sign in to the relay so web agents such as claude.ai can reach Blankey"
        onClick=${() => relaySignIn().catch(() => {})}><${Icon} name="connections" /> Sign in</button>`;
  }
  const name = relay.account ? relay.account.split("@")[0] : "Signed in";
  const title = `${relay.account ?? "Relay"}: ${STATES[relay.state]}`;
  const dot = relay.state === "connected" ? "up" : "down";
  return html`
    <button id="relay-btn" title=${title} onClick=${() => update({ page: "connections", search: "" })}>
      <span class=${`dot ${dot}`}></span><span class="relay-name">${name}</span>
    </button>`;
}

function ThemeSwitch() {
  const [theme, setThemeState] = useState(currentTheme());
  const next = theme === "dark" ? "light" : "dark";
  const toggle = () => {
    setTheme(next);
    setThemeState(next);
  };
  return html`
    <button class="icon-btn" title=${`Switch to ${next} mode`} aria-label=${`Switch to ${next} mode`} onClick=${toggle}>
      <${Icon} name=${theme === "dark" ? "sun" : "moon"} />
    </button>`;
}
