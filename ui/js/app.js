import { useEffect, useState } from "../vendor/preact-htm.js";
import { invoke, refreshRelay } from "./api.js";
import { t } from "./i18n.js";
import { HelpBox, helpEnabled } from "./help.js";
import { html, Icon, update, useStore } from "./lib.js";
import { Activity } from "./pages/activity.js";
import { Documents } from "./pages/documents.js";
import { FillSets } from "./pages/fillsets.js";
import { Links } from "./pages/links.js";
import { Profiles } from "./pages/profiles.js";
import { relaySignIn, Settings, stateLabel } from "./pages/settings.js";
import { Templates } from "./pages/templates.js";
import { currentTheme, setTheme } from "./theme.js";

// titles are functions: they are read at render time, in the current language
const PAGES = {
  activity: { title: () => t("Activity"), component: Activity },
  fillsets: { title: () => t("Fill sets"), component: FillSets },
  documents: { title: () => t("Documents"), component: Documents },
  templates: { title: () => t("Templates"), component: Templates },
  profiles: { title: () => t("Profiles"), component: Profiles },
  links: { title: () => t("Collection links"), component: Links },
  settings: { title: () => t("Settings"), component: Settings },
};

const SECTIONS = [
  [() => t("Assistant"), ["activity"]],
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
        ${helpEnabled() && html`<${HelpBox} page=${page} />`}
        <div class="sidebar-foot">
          <${RelayStatus} />
          <div class="foot-actions">
            <button id="lock-btn" data-help="sidebar.lock" title=${t("Lock the vault")} onClick=${() => invoke("lock")}><${Icon} name="lock" /> ${t("Lock")}</button>
            <${ThemeSwitch} />
            <button id="settings-btn" data-help="sidebar.settings" class=${`icon-btn ${page === "settings" ? "active" : ""}`} title=${t("Settings")}
              aria-label=${t("Settings")} onClick=${() => update({ page: "settings", search: "" })}>
              <${Icon} name="settings" />
            </button>
          </div>
        </div>
      </aside>
      <main class="main" data-help=${`page.${page}`}><${Current} key=${page} /></main>
    </div>`;
}

function NavItem({ id, active, pending }) {
  return html`
    <div class=${`nav-item ${active ? "active" : ""}`} data-help=${`nav.${id}`} onClick=${() => update({ page: id, search: "" })}>
      <${Icon} name=${id} />${PAGES[id].title()}
      ${pending > 0 && html`<span class="badge">${pending}</span>`}
    </div>`;
}

/** The relay connection above the sidebar buttons: the signed-in account, or a way to sign in. */
function RelayStatus() {
  const { relay } = useStore();
  useEffect(() => {
    refreshRelay().catch(() => {});
  }, []);
  if (!relay) return null;
  if (relay.state === "signing_in") {
    return html`<button id="relay-btn" class="foot-status" data-help="sidebar.relay" disabled><${Icon} name="connections" /> ${t("Signing in…")}</button>`;
  }
  if (!relay.signed_in) {
    return html`
      <button id="relay-btn" class="foot-status" data-help="sidebar.relay" title=${t("Sign in to Blankey Online so web agents such as claude.ai can reach Blankey")}
        onClick=${() => relaySignIn().catch(() => {})}><${Icon} name="connections" /> ${t("Sign in to Blankey Online")}</button>`;
  }
  const account = relay.account ?? t("Signed in");
  const title = t("Blankey Online: {account}, {state}", { account, state: stateLabel(relay.state) });
  const dot = relay.state === "connected" ? "up" : "down";
  return html`
    <button id="relay-btn" class="foot-status" data-help="sidebar.relay" title=${title} onClick=${() => update({ page: "settings", search: "" })}>
      <span class=${`dot ${dot}`}></span><span class="relay-name">${account}</span>
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
    <button class="icon-btn" data-help="sidebar.theme" title=${switchLabel} aria-label=${switchLabel} onClick=${toggle}>
      <${Icon} name=${theme === "dark" ? "sun" : "moon"} />
    </button>`;
}
