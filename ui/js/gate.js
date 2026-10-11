import { useEffect, useRef, useState } from "../vendor/preact-htm.js";
import { invoke, refreshStatus } from "./api.js";
import { html, plural, reload, state, update, useStore } from "./lib.js";
import { processRequests } from "./requests.js";
import { toast } from "./ui/feedback.js";

/** Shows the app after an unlock: Activity first when the assistant is waiting, then the waiting requests. */
export async function enterApp() {
  const status = await refreshStatus();
  if (!status.unlocked) return;
  triedQuick = false;
  update({ gateReason: "", recoveryKey: null, page: status.pending ? "activity" : state.page });
  reload();
  processRequests();
}

// Touch ID / keychain is tried once per lock, and only while the window is in front (never from a hidden window).
let triedQuick = false;

export const resetQuickUnlock = () => {
  triedQuick = false;
};

function tryQuickOnce() {
  const { status } = state;
  if (triedQuick || !document.hasFocus() || !status?.keyring_enabled || status.unlocked) return;
  triedQuick = true;
  quickUnlock();
}

async function quickUnlock() {
  if (await invoke("unlock_quick", { reason: state.gateReason ? "open your AI assistant's request" : null })) await enterApp();
}

export function Gate() {
  const { status, recoveryKey } = useStore();
  const [mode, setMode] = useState("unlock");
  if (recoveryKey) return html`<${RecoveryKey} recovery=${recoveryKey} />`;
  if (!status.initialized) return html`<${Setup} />`;
  if (mode === "recovery") return html`<${Recovery} onBack=${() => setMode("unlock")} />`;
  return html`<${Unlock} onForgot=${() => setMode("recovery")} />`;
}

/** The centered card; `onSubmit(values)` may throw a message, shown under the form. */
function GateForm({ title, text, onSubmit, children }) {
  const form = useRef();
  const [error, setError] = useState("");
  useEffect(() => form.current.querySelector("input")?.focus(), [title]);

  const submit = async (e) => {
    e.preventDefault();
    if (!onSubmit) return;
    const button = form.current.querySelector("button.primary");
    setError("");
    button.disabled = true;
    try {
      await onSubmit(Object.fromEntries(new FormData(form.current)));
    } catch (err) {
      setError(String(err));
      form.current.querySelector("input[type=password]")?.select();
    } finally {
      button.disabled = false;
    }
  };

  return html`
    <div class="gate" data-tauri-drag-region>
      <div class="gate-card">
        <img src="icon.png" alt="" class="gate-icon" />
        <h1>${title}</h1>
        <p class="muted">${text}</p>
        <form autocomplete="off" ref=${form} onSubmit=${submit}>${children}</form>
        <p class="error">${error}</p>
      </div>
    </div>`;
}

const keyringLabel = (status) => (status.biometric ? "Unlock with Touch ID" : "Unlock automatically with the keychain");

function Setup() {
  const { status } = useStore();
  const create = async ({ password, confirm, keyring }) => {
    if (password !== confirm) throw "The passwords do not match.";
    const recovery = await invoke("setup", { password, useKeyring: keyring === "on" });
    update({ recoveryKey: recovery });
  };
  return html`
    <${GateForm} title="Create your vault" onSubmit=${create}
      text="Personal data is encrypted with this password. Without it or the recovery key the data cannot be read.">
      <input type="password" name="password" placeholder="Master password (8+ characters)" />
      <input type="password" name="confirm" placeholder="Repeat password" />
      <label class="check small"><input type="checkbox" name="keyring" /> ${keyringLabel(status)}</label>
      <button class="primary">Create vault</button>
    </${GateForm}>`;
}

function Unlock({ onForgot }) {
  const { status, gateReason } = useStore();
  useEffect(() => {
    tryQuickOnce();
    window.addEventListener("focus", tryQuickOnce);
    return () => window.removeEventListener("focus", tryQuickOnce);
  }, []);

  const waiting = `${plural(status.pending, "request")} from your AI assistant ${status.pending === 1 ? "is" : "are"} waiting.`;
  const text = gateReason || (status.pending ? waiting : "Enter your master password.");
  const unlock = async ({ password }) => {
    await invoke("unlock", { password });
    await enterApp();
  };
  return html`
    <${GateForm} title="Blankey is locked" text=${text} onSubmit=${unlock}>
      <input type="password" name="password" placeholder="Master password" />
      <button class="primary">Unlock</button>
      ${status.keyring_enabled &&
      html`<button type="button" onClick=${quickUnlock}>${status.biometric ? "Use Touch ID" : "Use keychain"}</button>`}
      <button type="button" class="link" onClick=${onForgot}>Forgot password?</button>
    </${GateForm}>`;
}

function Recovery({ onBack }) {
  const reset = async ({ key, password }) => {
    await invoke("unlock_recovery", { recoveryKey: key, newPassword: password });
    await enterApp();
  };
  return html`
    <${GateForm} title="Reset password" text="Enter the recovery key you saved when the vault was created." onSubmit=${reset}>
      <input type="text" name="key" placeholder="XXXX-XXXX-…" class="mono" spellcheck="false" />
      <input type="password" name="password" placeholder="New master password" />
      <button class="primary">Unlock and set password</button>
      <button type="button" class="link" onClick=${onBack}>Back</button>
    </${GateForm}>`;
}

function RecoveryKey({ recovery }) {
  const [saved, setSaved] = useState(false);
  const copy = async () => {
    await invoke("copy_text", { text: recovery });
    toast("Recovery key copied");
  };
  return html`
    <${GateForm} title="Save your recovery key" onSubmit=${enterApp}
      text="Store it somewhere safe. It is shown only once and lets you set a new password if you forget the current one.">
      <div class="recovery">${recovery}</div>
      <button type="button" onClick=${copy}>Copy</button>
      <label class="check small">
        <input type="checkbox" checked=${saved} onChange=${(e) => setSaved(e.target.checked)} /> I saved it somewhere safe
      </label>
      <button class="primary" disabled=${!saved}>Continue</button>
    </${GateForm}>`;
}
