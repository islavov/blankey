import { html, render } from "../vendor/preact-htm.js";
import { invoke, listen, refreshRelay, refreshStatus } from "./api.js";
import { App } from "./app.js";
import { enterApp, Gate, resetQuickUnlock } from "./gate.js";
import { reload, state, update, useStore } from "./lib.js";
import { processRequests } from "./requests.js";
import { closeDialog } from "./ui/feedback.js";
import { hidePopup } from "./ui/menu.js";
import { applyTheme } from "./theme.js";
import { closeSheet } from "./ui/sheet.js";

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
    update({ gateReason: "Your AI assistant sent a request. Unlock Blankey to continue." });
  }
});
listen("requests-changed", () => refreshStatus());
listen("unlock-needed", (event) => update({ gateReason: event.payload }));
listen("relay-status", () => {
  refreshRelay();
  if (state.page === "connections") reload();
});
listen("show-page", ({ payload }) => update({ page: payload, search: "" }));

(async () => {
  const status = await refreshStatus();
  if (status.unlocked) enterApp();
})();
