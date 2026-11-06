// The case chat. A turn runs in the backend and streams `chat-event`s; the store below keeps what each running turn
// has streamed so far, per case, whichever case or page is open, so coming back shows the progress. The stored thread
// comes from `case_chat`, fetched by the Chat tab when it opens and again when a turn is done.

import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "../vendor/preact-htm.js";
import { call, invoke } from "./api.js";
import { language, number, t } from "./i18n.js";
import { html } from "./lib.js";

// -- store -------------------------------------------------------------------------------
// live turn: {turnId, userText, sentAt, steps: [part], status, done, stopped, error, failed, endedAt, ms}; steps are
// shaped like the stored assistant parts (text, tool-call with a `state` and, once it ran, its `ms`), except
// reasoning, which keeps its summary lines as `thoughts`

const running = new Set();
const live = new Map();
// case id -> turns done so far; the open chat re-fetches the thread when it grows
const ended = new Map();
// case id -> the composer's text, kept while another case is open
const drafts = new Map();
// case id -> tool calls and turns ended, which may have added to the case's lists
const worked = new Map();
let workChanges = 0;
// the case whose Chat tab is shown: Escape stops its turn
let openCase = null;
const listeners = new Set();
let changes = 0;
let runningChanges = 0;

/** Tells the subscribed components; `runningToo` when a turn started or ended. */
function changed(runningToo = false) {
  changes++;
  if (runningToo) runningChanges++;
  listeners.forEach((listener) => listener());
}

/** A turn that just started; `userText` and `sentAt` are null when this window did not see it start. */
const newTurn = (turnId, userText, sentAt = null) => ({
  turnId, userText, sentAt, steps: [], status: null, done: false, stopped: null, error: null, failed: false, endedAt: 0, ms: null,
});

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
  if (event === "tool_done" || event === "done" || event === "error") {
    worked.set(caseId, (worked.get(caseId) ?? 0) + 1);
    workChanges++;
  }
  if (event === "started") {
    running.add(caseId);
    live.set(caseId, newTurn(turnId, data.text ?? "", data.sent_at ?? null));
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
    if (step) Object.assign(step, { state: data.is_error ? "failed" : "done", ms: data.ms ?? null });
    turn.status = null;
  } else if (event === "status" && data.state === "thinking") {
    // the step's live line shows it, not the status line
    think(turn);
    turn.status = null;
  } else if (event === "status") {
    turn.status = data.state;
  } else if (event === "done") {
    Object.assign(turn, { done: true, status: null, stopped: data.stopped ?? null, error: data.error ?? null, ms: data.timing?.ms ?? null, endedAt: end(caseId) });
  } else if (event === "error") {
    running.delete(caseId);
    Object.assign(turn, { failed: true, status: null, error: { code: data.code, message: data.message }, ms: data.ms ?? null });
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

/** How many of the case's tool calls and turns ended, to reload what they may have added; re-renders only then. */
export function useWorked(caseId) {
  useStamp(() => workChanges);
  return worked.get(caseId) ?? 0;
}

/** Escape with nothing else to close: stops the turn of the chat that is shown. False when none runs. */
export function stopOpenChat() {
  if (openCase == null || !running.has(openCase)) return false;
  call("stop_chat", { caseId: openCase }).catch(() => {});
  return true;
}

/** The cases with a turn in progress; re-renders only when a turn starts or ends. */
export function useRunning() {
  useStamp(() => runningChanges);
  return running;
}

// -- the Chat tab -------------------------------------------------------------------------

/** The sign of a turn in progress, in the case list, the Chat tab and the line that says what the assistant does. */
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

/** The time of day a message was sent, with its date when that is not today. */
function clock(iso) {
  const at = new Date(iso);
  const time = { hour: "2-digit", minute: "2-digit", second: "2-digit" };
  return at.toDateString() === new Date().toDateString()
    ? at.toLocaleTimeString(language(), time)
    : at.toLocaleString(language(), { day: "numeric", month: "short", ...time });
}

/** A length of time: tenths of a second under ten seconds, whole seconds under a minute, then minutes and seconds. */
function duration(ms) {
  if (ms < 10000) return t("{n} s", { n: number(ms / 1000, 1) });
  const total = Math.round(ms / 1000);
  return total < 60 ? t("{n} s", { n: total }) : t("{m} min {s} s", { m: Math.floor(total / 60), s: total % 60 });
}

const sum = (values) => values.reduce((total, ms) => total + (ms ?? 0), 0);

/** A turn's timing, `{sentAt, ms, tools}`, from what its stored user message keeps. */
const storedTiming = (timing) => ({ sentAt: timing?.sent_at ?? null, ms: timing?.ms ?? null, tools: sum(Object.values(timing?.tools ?? {})) });

/** When the user sent the message; once the turn ended, how long it took, and how much of that the model and the tools took. */
function TurnTime({ timing }) {
  if (!timing.sentAt) return null;
  const parts = [clock(timing.sentAt)];
  if (timing.ms != null) {
    parts.push(duration(timing.ms));
    if (timing.tools > 0) {
      parts.push(t("model {time}", { time: duration(Math.max(0, timing.ms - timing.tools)) }), t("tools {time}", { time: duration(timing.tools) }));
    }
  }
  return html`<div class="chat-time">${parts.join(" · ")}</div>`;
}

/** The parts of a stored message; its content can also be a plain string. */
const partsOf = (message) => (typeof message.content === "string" ? [{ type: "text", text: message.content }] : message.content || []);

/** A reasoning part's progress lines: live, its `thoughts`; stored, the lines of its text. */
const linesOf = (part) => part.thoughts ?? String(part.text ?? "").split("\n").filter((line) => line.trim());

/** The stored thread as turns: each user message with the progress lines and the texts of the steps that answered it. */
function turnsOf(messages) {
  const turns = [];
  for (const { seq, message } of messages) {
    if (message.role === "user" || !turns.length) turns.push({ seq, user: message.role === "user" ? message : null, lines: [], texts: [] });
    if (message.role !== "assistant") continue;
    const turn = turns.at(-1);
    for (const part of partsOf(message)) {
      if (part.type === "reasoning") turn.lines.push(...linesOf(part));
      else if (part.type === "text" && part.text) turn.texts.push(part.text);
    }
  }
  return turns;
}

/** What the assistant did in a turn, collapsed: the progress lines it recorded. */
function Progress({ lines }) {
  if (!lines.length) return null;
  return html`<details class="chat-reasoning"><summary>${t("Progress")}</summary><div>${lines.map((line, i) => html`<div key=${i}>${line}</div>`)}</div></details>`;
}

/** The Chat tab of a case: the stored thread, the turn streaming now, and the composer. */
export function CaseChat({ caseId }) {
  const chat = useChat(caseId);
  const [thread, setThread] = useState(null);
  const box = useRef();
  // follows new content while the user is at the bottom
  const stick = useRef(true);

  useEffect(() => {
    openCase = caseId;
    return () => {
      if (openCase === caseId) openCase = null;
    };
  }, [caseId]);

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
    return html`
      ${thread.summary &&
      html`<details class="chat-note"><summary>${t("Earlier messages are summarized")}</summary><div>${thread.summary}</div></details>`}
      ${turnsOf(thread.messages).map((turn) => html`<${StoredTurn} key=${turn.seq} turn=${turn} />`)}`;
  }, [thread, language()]);

  const { turn } = chat;
  // a done turn shows from the stored thread once that is fetched again; until then, as it streamed
  const stored = turn?.done && thread?.at >= turn.endedAt;
  const empty = !thread?.messages.length && !turn && !chat.running;

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
    } else if (e.key === "Escape" && !chat.running) {
      // while a turn runs, Escape goes on to the window: a sheet closes first, else the turn stops
      e.preventDefault();
      e.target.blur();
    }
  };

  return html`
    <div class="chat-thread" ref=${box} onScroll=${onScroll}>
      ${thread &&
      html`
        ${storedView}
        ${turn && !stored && !turn.failed && html`<${LiveTurn} turn=${turn} running=${chat.running} />`}
        ${chat.running && !turn && html`<div class="chat-status"><${Spinner} /> ${t("The assistant is working")}</div>`}
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

/** The user's message and its timing, then what the assistant did, collapsed, and what it answered. */
function StoredTurn({ turn }) {
  const text = turn.user ? partsOf(turn.user).filter((p) => p.type === "text").map((p) => p.text).join("\n") : "";
  return html`
    ${text && html`<div class="chat-msg user">${text}</div><${TurnTime} timing=${storedTiming(turn.user.timing)} />`}
    <${Progress} lines=${turn.lines} />
    ${turn.texts.map((text, i) => html`<div key=${i} class="chat-text">${text}</div>`)}`;
}

/**
 * The turn as it streams: the user's message, then what the assistant wrote; while it works, one line at the bottom
 * says what it is doing (its latest progress line, or that it waits or summarizes), and once done its progress collapses.
 */
function LiveTurn({ turn, running }) {
  const lines = turn.steps.flatMap((step) => (step.type === "reasoning" ? linesOf(step) : []));
  const texts = turn.steps.filter((step) => step.type === "text" && step.text);
  const thinking = turn.steps.at(-1)?.type === "reasoning";
  const now = STATUS[turn.status]?.() || lines.at(-1) || (thinking ? t("Thinking…") : t("The assistant is working"));
  return html`
    ${turn.userText && html`<div class="chat-msg user">${turn.userText}</div>`}
    <${TurnTime} timing=${{ sentAt: turn.sentAt, ms: turn.ms, tools: sum(turn.steps.map((step) => step.ms)) }} />
    ${!running && html`<${Progress} lines=${lines} />`}
    ${texts.map((step, i) => html`<div key=${i} class="chat-text">${step.text}</div>`)}
    ${running && html`<div class="chat-thinking"><${Spinner} /><span>${now}</span></div>`}`;
}

/** How a turn ended, when that needs saying: stopped, or an error and how long the turn ran until then. */
function TurnEnd({ turn }) {
  if (turn.failed || turn.stopped === "error") {
    const after = turn.failed && turn.ms != null ? html` <span class="muted">${t("after {time}", { time: duration(turn.ms) })}</span>` : null;
    return turn.error ? html`<div class="chat-error">${errorText(turn.error)}${after}</div>` : null;
  }
  if (turn.stopped === "step_limit") return html`<div class="chat-note muted">${t("The assistant stopped after many steps; ask it to continue")}</div>`;
  if (turn.stopped === "cancelled") return html`<div class="chat-note muted">${t("Stopped")}</div>`;
  return null;
}
