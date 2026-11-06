// The case chat. A turn runs in the backend and streams `chat-event`s; the store below keeps what each running turn
// has streamed so far, per case, whichever case or page is open, so coming back shows the progress. The stored thread
// comes from `case_chat`, fetched by the Chat tab when it opens and again when a turn is done.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke } from "./api.js";
import { language, t } from "./i18n.js";
import { html } from "./lib.js";

// -- store -------------------------------------------------------------------------------
// live turn: {turnId, userText, steps: [part], status, done, stopped, error, failed, endedAt}; steps are shaped like
// the stored assistant parts (text, tool-call with a `state`), except reasoning, which keeps its summary lines as
// `thoughts`

const running = new Set();
const live = new Map();
// case id -> turns done so far; the open chat re-fetches the thread when it grows
const ended = new Map();
// case id -> the composer's text, kept while another case is open
const drafts = new Map();
const listeners = new Set();
let changes = 0;
let runningChanges = 0;

/** Tells the subscribed components; `runningToo` when a turn started or ended. */
function changed(runningToo = false) {
  changes++;
  if (runningToo) runningChanges++;
  listeners.forEach((listener) => listener());
}

/** A turn that just started; `userText` is null when this window did not see it start. */
const newTurn = (turnId, userText) => ({ turnId, userText, steps: [], status: null, done: false, stopped: null, error: null, failed: false, endedAt: 0 });

/** Appends streamed text to the last step when it is text, else starts a new text step. */
function append(turn, text) {
  const last = turn.steps.at(-1);
  if (last?.type === "text") last.text += text;
  else turn.steps.push({ type: "text", text });
}

/** The model started thinking in a new step: the reasoning step that its thoughts go into. */
function think(turn) {
  const last = turn.steps.at(-1);
  if (last?.type === "reasoning") return last;
  const step = { type: "reasoning", thoughts: [] };
  turn.steps.push(step);
  return step;
}

/** A summary of what the model thinks now; it goes to the step's reasoning, even once the step's text has started. */
function thought(turn, text) {
  const step = turn.steps.findLast((s) => s.type === "reasoning") ?? think(turn);
  step.thoughts.push(text);
}

/** A turn ended: the case stops running, and an open chat re-fetches the thread. */
function end(caseId) {
  running.delete(caseId);
  const n = (ended.get(caseId) || 0) + 1;
  ended.set(caseId, n);
  return n;
}

/** One `chat-event` from the backend: `{case_id, turn_id, event, data}`. */
export function chatEvent({ case_id: caseId, turn_id: turnId, event, data = {} }) {
  if (event === "started") {
    running.add(caseId);
    live.set(caseId, newTurn(turnId, data.text ?? ""));
    return changed(true);
  }
  const wasRunning = running.has(caseId);
  let turn = live.get(caseId);
  if (turn?.turnId !== turnId) {
    // a turn this window did not see start (it opened or unlocked mid-turn): its end only ends it
    if (event === "done" || event === "error") {
      end(caseId);
      return changed(true);
    }
    turn = newTurn(turnId, null);
    live.set(caseId, turn);
    running.add(caseId);
  }
  if (event === "text_delta") {
    append(turn, data.text ?? "");
    turn.status = null;
  } else if (event === "thinking") {
    if (data.text) thought(turn, data.text);
  } else if (event === "tool_call") {
    turn.steps.push({ type: "tool-call", toolCallId: data.id, toolName: data.name, input: data.input, state: "running" });
    turn.status = null;
  } else if (event === "tool_done") {
    const step = turn.steps.find((s) => s.type === "tool-call" && s.toolCallId === data.id);
    if (step) step.state = data.is_error ? "failed" : "done";
    turn.status = null;
  } else if (event === "status" && data.state === "thinking") {
    // the step's live line shows it, not the status line
    think(turn);
    turn.status = null;
  } else if (event === "status") {
    turn.status = data.state;
  } else if (event === "done") {
    Object.assign(turn, { done: true, status: null, stopped: data.stopped ?? null, error: data.error ?? null, endedAt: end(caseId) });
  } else if (event === "error") {
    running.delete(caseId);
    Object.assign(turn, { failed: true, status: null, error: { code: data.code, message: data.message } });
    // nothing of the turn was stored: its text goes back into the composer to send again
    if (!drafts.get(caseId)?.trim() && turn.userText) drafts.set(caseId, turn.userText);
  }
  changed(running.has(caseId) !== wasRunning);
}

/** The cases with a turn in progress, from the backend: at startup and after an unlock. */
export async function seedChat() {
  const ids = await invoke("chat_running").catch(() => null);
  if (!ids) return;
  for (const id of running) {
    if (ids.includes(id)) continue;
    const turn = live.get(id);
    const n = end(id);
    if (turn && !turn.done && !turn.failed) Object.assign(turn, { done: true, status: null, endedAt: n });
  }
  ids.forEach((id) => running.add(id));
  changed(true);
}

/** The vault locked: what streamed is dropped with the drafts; the backend stopped the turns. */
export function resetChat() {
  running.clear();
  live.clear();
  drafts.clear();
  changed(true);
}

/** The composer's text for the case. */
const setDraft = (caseId, text) => {
  drafts.set(caseId, text);
  changed();
};

/** Re-renders the component when `stamp()` changes. */
function useStamp(stamp) {
  const [, force] = useState(0);
  const seen = stamp();
  useEffect(() => {
    let last = seen;
    const listener = () => {
      if (stamp() === last) return;
      last = stamp();
      force((n) => n + 1);
    };
    listeners.add(listener);
    // effects run after paint: catch up on changes made since this render
    listener();
    return () => listeners.delete(listener);
  }, []);
}

/** The case's live turn, whether it runs, its draft and how many of its turns ended; re-renders on every change. */
export function useChat(caseId) {
  useStamp(() => changes);
  return { running: running.has(caseId), turn: live.get(caseId) ?? null, ended: ended.get(caseId) ?? 0, draft: drafts.get(caseId) ?? "" };
}

/** The cases with a turn in progress; re-renders only when a turn starts or ends. */
export function useRunning() {
  useStamp(() => runningChanges);
  return running;
}

// -- the Chat tab -------------------------------------------------------------------------

/** The sign of a turn in progress, in the case list, the Chat tab and a running tool's card. */
export const Spinner = () => html`<span class="spinner" role="status" title=${t("The assistant is working")} aria-label=${t("The assistant is working")}></span>`;

const ERRORS = {
  chat_budget: () => t("This month's assistant budget is used up"),
  chat_not_enabled: () => t("The assistant is not enabled for your account"),
  device_offline: () => t("This computer is not connected to Blankey Online. Try again in a moment."),
  unavailable: () => t("The assistant is not available. Check that you are signed in to Blankey Online."),
  lost: () => t("The connection to the assistant was lost. Send the message again."),
};
/** What went wrong, in the user's language when the code is known. */
const errorText = (error) => ERRORS[error.code]?.() || error.message;

const STATUS = {
  compacting: () => t("Summarizing earlier messages"),
  waiting: () => t("Waiting for you in Blankey"),
};

/** The parts of a stored message; its content can also be a plain string. */
const partsOf = (message) => (typeof message.content === "string" ? [{ type: "text", text: message.content }] : message.content || []);

/** Each tool call's state, done or failed, by its id, from the tool results. */
function toolStates(messages) {
  const states = new Map();
  for (const { message } of messages) {
    if (message.role !== "tool") continue;
    for (const part of partsOf(message)) {
      if (part.type === "tool-result") states.set(part.toolCallId, String(part.output?.type ?? "").startsWith("error") ? "failed" : "done");
    }
  }
  return states;
}

/** The Chat tab of a case: the stored thread, the turn streaming now, and the composer. */
export function CaseChat({ caseId }) {
  const chat = useChat(caseId);
  const [thread, setThread] = useState(null);
  const box = useRef();
  // follows new content while the user is at the bottom
  const stick = useRef(true);

  useEffect(() => {
    let current = true;
    const at = chat.ended;
    call("case_chat", { caseId })
      .catch(() => ({ messages: [], summary: null }))
      .then((data) => current && setThread({ ...data, at }));
    return () => {
      current = false;
    };
  }, [caseId, chat.ended]);

  useLayoutEffect(() => {
    if (stick.current && box.current) box.current.scrollTop = box.current.scrollHeight;
  });
  const onScroll = (e) => {
    const el = e.currentTarget;
    stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  };

  // diffed again only when the thread changes, not on every streamed piece or keystroke
  const storedView = useMemo(() => {
    if (!thread) return null;
    const states = toolStates(thread.messages);
    return html`
      ${thread.summary &&
      html`<details class="chat-note"><summary>${t("Earlier messages are summarized")}</summary><div>${thread.summary}</div></details>`}
      ${thread.messages.map(({ seq, message }) => html`<${StoredMessage} key=${seq} message=${message} states=${states} />`)}`;
  }, [thread, language()]);

  const { turn } = chat;
  // a done turn shows from the stored thread once that is fetched again; until then, as it streamed
  const stored = turn?.done && thread?.at >= turn.endedAt;
  const empty = !thread?.messages.length && !turn && !chat.running;
  // a running tool's card and a thinking step's line have their own spinner
  const last = turn?.steps.at(-1);
  const working = chat.running && (turn?.status || (last?.state !== "running" && last?.type !== "reasoning"));

  const send = async () => {
    const text = chat.draft.trim();
    if (!text || chat.running) return;
    stick.current = true;
    setDraft(caseId, "");
    await call("send_chat_message", { caseId, text }).catch(() => {
      if (!drafts.get(caseId)) setDraft(caseId, text);
    });
  };
  const onKeyDown = (e) => {
    if (e.isComposing) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.target.blur();
    }
  };

  return html`
    <div class="chat-thread" ref=${box} onScroll=${onScroll}>
      ${thread &&
      html`
        ${storedView}
        ${turn && !stored && !turn.failed && html`<${LiveTurn} turn=${turn} />`}
        ${working && html`<div class="chat-status"><${Spinner} /> ${STATUS[turn?.status]?.() || t("The assistant is working")}</div>`}
        ${turn && html`<${TurnEnd} turn=${turn} />`}
        ${empty && html`<div class="empty">${t("Ask the assistant to work on this case. It knows the people, files and notes in it, never the values in profiles.")}</div>`}`}
    </div>
    <div class="chat-composer">
      <textarea rows="2" data-help="chat.message" placeholder=${t("Ask the assistant about this case")} value=${chat.draft}
        onInput=${(e) => setDraft(caseId, e.target.value)} onKeyDown=${onKeyDown}></textarea>
      ${chat.running
        ? html`<button class="danger" data-help="chat.stop" onClick=${() => call("stop_chat", { caseId }).catch(() => {})}>${t("Stop")}</button>`
        : html`<button class="primary" data-help="chat.send" disabled=${!chat.draft.trim()} onClick=${send}>${t("Send")}</button>`}
    </div>`;
}

/** A stored user message as a bubble, an assistant message as its parts; tool results show on their cards. */
function StoredMessage({ message, states }) {
  const parts = partsOf(message);
  if (message.role === "user") {
    const text = parts.filter((p) => p.type === "text").map((p) => p.text).join("\n");
    return text ? html`<div class="chat-msg user">${text}</div>` : null;
  }
  if (message.role !== "assistant") return null;
  return parts.map((part, i) => html`<${Part} key=${i} part=${part} state=${states.get(part.toolCallId) || "done"} />`);
}

/** The turn as it streams: the user's message, then the steps; the last one is live while the turn runs. */
function LiveTurn({ turn }) {
  return html`
    ${turn.userText && html`<div class="chat-msg user">${turn.userText}</div>`}
    ${turn.steps.map((part, i) => html`<${Part} key=${i} part=${part} state=${part.state} live=${!turn.done && i === turn.steps.length - 1} />`)}`;
}

/** How a turn ended, when that needs saying: stopped, or an error. */
function TurnEnd({ turn }) {
  if (turn.failed || turn.stopped === "error") {
    return turn.error ? html`<div class="chat-error">${errorText(turn.error)}</div>` : null;
  }
  if (turn.stopped === "step_limit") return html`<div class="chat-note muted">${t("The assistant stopped after many steps; ask it to continue")}</div>`;
  if (turn.stopped === "cancelled") return html`<div class="chat-note muted">${t("Stopped")}</div>`;
  return null;
}

/** One assistant part: reasoning (its latest thought while live, else collapsed), text, or a tool call's card. */
function Part({ part, state, live = false }) {
  if (part.type === "reasoning") {
    // live, its summary lines are `thoughts`; stored, they are its text
    const lines = part.thoughts ?? String(part.text ?? "").split("\n").filter((line) => line.trim());
    if (live) return html`<div class="chat-thinking"><${Spinner} /><span>${lines.at(-1) || t("Thinking…")}</span></div>`;
    return lines.length
      ? html`<details class="chat-reasoning"><summary>${t("Thinking")}</summary><div>${lines.map((line, i) => html`<div key=${i}>${line}</div>`)}</div></details>`
      : null;
  }
  if (part.type === "text") return part.text ? html`<div class="chat-text">${part.text}</div>` : null;
  if (part.type === "tool-call") return html`<${ToolCard} name=${part.toolName} input=${part.input} state=${state} />`;
  return null;
}

const TOOL_STATES = {
  running: () => t("Running"),
  done: () => t("Done"),
  failed: () => t("Failed"),
};

/** A tool call: its readable name and whether it ran; open it for what the assistant passed. */
function ToolCard({ name, input, state }) {
  const args = input && typeof input === "object" && Object.keys(input).length ? JSON.stringify(input, null, 2) : "";
  const head = html`
    <span class="tool-mark">${state === "running" ? html`<${Spinner} />` : state === "failed" ? "✕" : "✓"}</span>
    <span class="tool-name">${String(name ?? "").replace(/[_-]+/g, " ")}</span>
    <span class="tool-state muted">${TOOL_STATES[state]?.()}</span>`;
  const cls = `chat-tool ${state}`;
  return args
    ? html`<details class=${cls} data-help="chat.tool"><summary>${head}</summary><pre class="mono">${args}</pre></details>`
    : html`<div class=${cls} data-help="chat.tool"><div class="tool-head">${head}</div></div>`;
}
