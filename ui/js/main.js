import { html, render } from "../vendor/preact-htm.js";
import { applyLanguage, invoke, listen, refreshRelay, refreshStatus } from "./api.js";
import { App } from "./app.js";
import { enterApp, Gate, resetQuickUnlock } from "./gate.js";
import { t } from "./i18n.js";
import { reload, state, update, useStore } from "./lib.js";
import { processRequests } from "./requests.js";
import { closeDialog } from "./ui/feedback.js";
import { hidePopup } from "./ui/menu.js";
import { applyTheme } from "./theme.js";
import { closeSheet } from "./ui/sheet.js";

// script errors go to the app's log, where they can be read when the window comes up blank
const report = (message) => invoke("log_error", { message: String(message) }).catch(() => {});
window.addEventListener("error", (e) => report(`${e.message} (${e.filename}:${e.lineno})`));
window.addEventListener("unhandledrejection", (e) => report(e.reason?.stack || e.reason));

applyTheme();

function Root() {
  const { status, recoveryKey } = useStore();
  if (!status) return null;
  if (recoveryKey || !status.initialized || !status.unlocked) return html`<${Gate} />`;
  return html`<${App} />`;
}

render(html`<${Root} />`, document.getElementById("root"));

// activity for auto-lock
let lastTouch = 0;
for (const type of ["keydown", "mousedown", "wheel"]) {
  document.addEventListener(
    type,
    () => {
      if (Date.now() - lastTouch > 10000) {
        lastTouch = Date.now();
        invoke("touch").catch(() => {});
      }
    },
    { passive: true }
  );
}

listen("vault-locked", async () => {
  resetQuickUnlock();
  closeSheet();
  hidePopup();
  closeDialog();
  await refreshStatus();
});
listen("vault-unlocked", () => enterApp());
listen("request-created", async () => {
  const status = await refreshStatus();
  if (status.unlocked) {
    if (state.page === "activity") reload();
    processRequests();
  } else {
    update({ gateReason: t("Your AI assistant sent a request. Unlock Blankey to continue.") });
  }
});
listen("requests-changed", () => refreshStatus());
// the backend's reasons are English; the catalog has them
listen("unlock-needed", (event) => update({ gateReason: t(event.payload) }));
listen("language-changed", ({ payload }) => applyLanguage(payload));
// the account's language may have changed on the relay while the window was in the background
window.addEventListener("focus", () => invoke("focused").catch(() => {}));
listen("relay-status", () => {
  refreshRelay();
  if (state.page === "connections") reload();
});
listen("show-page", ({ payload }) => update({ page: payload, search: "" }));

(async () => {
  const status = await refreshStatus();
  if (status.unlocked) enterApp();
})();
