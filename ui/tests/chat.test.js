import assert from "node:assert/strict";
import { before, test } from "node:test";
import { stored } from "./backend.js";
import { $, $$, byText, click, goTo, key, settle, startApp, submit, text, type } from "./setup.js";

let backend;
let helpText;
before(async () => {
  backend = await startApp({
    cases: [
      { id: 1, title: "Sale, 12 Oak Street", notes: "", created_at: "2026-10-28T09:00:00", updated_at: "2026-11-02T10:00:00" },
      { id: 2, title: "Lease, Main Road", notes: "", created_at: "2026-10-20T09:00:00", updated_at: "2026-10-21T10:00:00" },
    ],
    chatRunning: [2],
  });
  ({ helpText } = await import(new URL("../js/help.js", import.meta.url).href));
});

const chips = () => $$(".case-tabs .chip").map((el) => el.textContent.replace(/\d+/g, "").trim());
const tab = (label) => click($$(".case-tabs .chip").find((el) => el.textContent.trim().startsWith(label)));
const caseItem = (title) => byText(".split.cases .list-item .title", title).parentElement;
const spinning = () => $$(".case-item").filter((el) => $(".spinner", el)).map((el) => text(".title", el));
const composer = () => $(".chat-composer textarea");
const button = (label) => byText(".chat-composer button", label);
const userTexts = () => $$(".chat-msg.user").map((el) => el.textContent);
const lastNote = () => $$(".chat-note").at(-1)?.textContent ?? null;
const toolCards = () => $$(".chat-tool").map((el) => [text(".tool-name", el), ["running", "done", "failed"].find((s) => el.classList.contains(s))]);
const thoughts = (details) => $$("div > div", details).map((el) => el.textContent);

/** A chat-event from the backend, as Rust emits it. */
async function chatEvent(event, data, caseId = 1, turnId = "turn-1") {
  backend.emit("chat-event", { case_id: caseId, turn_id: turnId, event, data });
  await settle();
}

/** Types into the composer and presses Enter; the turn id the backend gave. */
async function send(message) {
  await type(composer(), message);
  await key("Enter", composer());
  return `turn-${backend.called("send_chat_message").length}`;
}

test("at startup, chat_running puts a spinner on the cases with a turn in progress", async () => {
  assert.ok(backend.called("chat_running").length > 0);
  assert.deepEqual(spinning(), ["Lease, Main Road"]);
  assert.equal($(".case-item .spinner").getAttribute("aria-label"), "The assistant is working");
  await chatEvent("done", { stopped: null }, 2, "turn-0");
  assert.deepEqual(spinning(), []);
});

test("the Chat tab shows, first, once the relay offers the assistant", async () => {
  assert.ok(!chips().includes("Chat"));
  backend.db.relay.chat = true;
  backend.emit("relay-status");
  await settle();
  assert.equal(chips()[0], "Chat");
  assert.equal($(".case-tabs .chip").getAttribute("data-help"), "case.chat");
  assert.ok(helpText("case.chat"));
  assert.ok(text(".case-tabs .chip.active").startsWith("Files"), "the default tab stays");
});

test("the stored thread: messages, collapsed reasoning, tool cards done or failed", async () => {
  await tab("Chat");
  assert.ok($(".case-tab.chat-tab"));
  assert.deepEqual(backend.called("case_chat").at(-1), { caseId: 1 });
  assert.deepEqual(userTexts(), ["Who sells and who buys?"]);
  const reasoning = $(".chat-reasoning");
  assert.equal(reasoning.open, false);
  assert.equal(text("summary", reasoning), "Thinking");
  assert.deepEqual(thoughts(reasoning), ["The case has two profiles; their roles say it."]);
  assert.deepEqual(toolCards(), [["find profiles", "done"], ["get template", "failed"]]);
  assert.equal(text(".tool-state", $$(".chat-tool")[1]), "Failed");
  assert.equal($(".chat-text").textContent, "Mary Example sells, John buys.\nThere is no deed template yet.");
  assert.equal($(".chat-note"), null, "no summary yet");
  assert.equal(button("Send").disabled, true, "nothing to send");

  const keys = $$(".case-tab [data-help]").map((el) => el.getAttribute("data-help"));
  assert.ok(keys.includes("chat.message") && keys.includes("chat.send") && keys.includes("chat.tool"));
  assert.deepEqual(keys.filter((k) => !helpText(k)), []);
});

test("Enter sends the message and clears the composer; Shift+Enter does not send", async () => {
  await type(composer(), "Fill the sale contract");
  await key("Enter", composer(), { shiftKey: true });
  assert.equal(backend.called("send_chat_message").length, 0);
  assert.equal(button("Send").disabled, false);
  await key("Enter", composer());
  assert.deepEqual(backend.called("send_chat_message").at(-1), { caseId: 1, text: "Fill the sale contract" });
  assert.equal(composer().value, "");
});

test("a turn streams in: the user's text at once, the thinking line, text, tool cards and the status", async () => {
  await chatEvent("started", { text: "Fill the sale contract" });
  assert.deepEqual(spinning(), ["Sale, 12 Oak Street"]);
  assert.deepEqual(userTexts(), ["Who sells and who buys?", "Fill the sale contract"]);
  assert.equal(text(".chat-status"), "The assistant is working");
  assert.ok(button("Stop"));
  assert.ok($(".case-tabs .chip.active .spinner"), "the Chat tab spins too");

  const reasoningShown = $$(".chat-reasoning").length;
  await chatEvent("status", { state: "thinking" });
  assert.equal(text(".chat-thinking"), "Thinking…");
  assert.ok($(".chat-thinking .spinner"));
  assert.equal($(".chat-status"), null, "the thinking line is the status");
  assert.equal($$(".chat-reasoning").length, reasoningShown, "nothing to open yet");

  await chatEvent("thinking", { text: "Checking which parties the template needs" });
  assert.equal(text(".chat-thinking"), "Checking which parties the template needs");
  await chatEvent("thinking", { text: "Looking up both parties' profiles" });
  assert.deepEqual($$(".chat-thinking").map((el) => el.textContent), ["Looking up both parties' profiles"], "a thought replaces the last");

  await chatEvent("text_delta", { text: "Filling the contract" });
  assert.equal($(".chat-thinking"), null, "the answer ends the thinking line");
  const thinking = $$(".chat-reasoning").at(-1);
  assert.equal(thinking.open, false);
  assert.equal(text("summary", thinking), "Thinking");
  assert.deepEqual(thoughts(thinking), ["Checking which parties the template needs", "Looking up both parties' profiles"]);
  assert.equal($$(".chat-text").at(-1).textContent, "Filling the contract");
  await chatEvent("thinking", { text: "Filling the contract for both" });
  assert.equal($(".chat-thinking"), null, "a late thought joins the list");
  assert.deepEqual(thoughts($$(".chat-reasoning").at(-1)), ["Checking which parties the template needs", "Looking up both parties' profiles", "Filling the contract for both"]);

  await chatEvent("tool_call", { id: "c9", name: "request_fill", input: { templates: ["oak-sale"] } });
  const card = $$(".chat-tool").at(-1);
  assert.deepEqual(toolCards().at(-1), ["request fill", "running"]);
  assert.ok($(".spinner", card));
  assert.equal($(".chat-status"), null, "the running card has its own spinner");

  await chatEvent("status", { state: "waiting" });
  assert.equal(text(".chat-status"), "Waiting for you in Blankey");
  await chatEvent("tool_done", { id: "c9", is_error: false });
  assert.deepEqual(toolCards().at(-1), ["request fill", "done"]);
  assert.equal(text(".chat-status"), "The assistant is working");
  await chatEvent("status", { state: "compacting" });
  assert.equal(text(".chat-status"), "Summarizing earlier messages");
});

test("the turn goes on while another case or page is open; coming back shows it", async () => {
  backend.db.chats[2] = { summary: "The lease starts on 1 December.", messages: [stored(1, "user", [{ type: "text", text: "When does it start?" }])] };
  await click(caseItem("Lease, Main Road"));
  assert.ok(text(".case-tabs .chip.active").startsWith("Chat"));
  assert.deepEqual(spinning(), ["Sale, 12 Oak Street"], "only the running case spins");
  assert.equal(text(".chat-note summary"), "Earlier messages are summarized");
  assert.deepEqual(userTexts(), ["When does it start?"]);
  assert.ok(button("Send"), "this case has no turn running");

  await chatEvent("text_delta", { text: "Both parties are in the case." });
  await goTo("Documents");
  await chatEvent("tool_call", { id: "c10", name: "wait_request", input: {} });
  await goTo("Cases");
  await click(caseItem("Sale, 12 Oak Street"));
  assert.deepEqual(userTexts(), ["Who sells and who buys?", "Fill the sale contract"]);
  assert.deepEqual($$(".chat-text").slice(-2).map((el) => el.textContent), ["Filling the contract", "Both parties are in the case."], "text after a tool call is a new part");
  assert.deepEqual(toolCards().slice(-2), [["request fill", "done"], ["wait request", "running"]]);
  assert.ok(button("Stop"));

  backend.emit("cases-changed");
  await settle();
  assert.deepEqual(toolCards().slice(-2), [["request fill", "done"], ["wait request", "running"]], "a reload keeps the live turn");
});

test("Stop asks the backend to stop the case's turn", async () => {
  await click(button("Stop"));
  assert.deepEqual(backend.called("stop_chat").at(-1), { caseId: 1 });
});

test("done fetches the stored thread again and the spinner goes", async () => {
  backend.db.chats[1].messages.push(
    stored(5, "user", [{ type: "text", text: "Fill the sale contract" }]),
    stored(6, "assistant", [
      { type: "reasoning", text: "Checking which parties the template needs\nLooking up both parties' profiles" },
      { type: "text", text: "Filling the contract" },
      { type: "tool-call", toolCallId: "c9", toolName: "request_fill", input: { templates: ["oak-sale"] } },
      { type: "text", text: "Both parties are in the case." },
      { type: "tool-call", toolCallId: "c10", toolName: "wait_request", input: {} },
    ]),
    stored(7, "tool", [
      { type: "tool-result", toolCallId: "c9", toolName: "request_fill", output: { type: "json", value: { request_id: 4 } } },
      { type: "tool-result", toolCallId: "c10", toolName: "wait_request", output: { type: "error-json", value: { cancelled: true } } },
    ])
  );
  const fetched = backend.called("case_chat").length;
  await chatEvent("done", { stopped: "cancelled" });
  assert.equal(backend.called("case_chat").length, fetched + 1);
  assert.deepEqual(spinning(), []);
  assert.equal($(".chat-status"), null);
  assert.deepEqual(userTexts(), ["Who sells and who buys?", "Fill the sale contract"], "the turn shows once, as stored");
  assert.deepEqual(toolCards().slice(-2), [["request fill", "done"], ["wait request", "failed"]]);
  const reasoning = $$(".chat-reasoning").at(-1);
  assert.equal(reasoning.open, false);
  reasoning.open = true;
  assert.deepEqual(thoughts(reasoning), ["Checking which parties the template needs", "Looking up both parties' profiles"], "one line per thought");
  assert.equal(lastNote(), "Stopped");
  assert.ok(button("Send"));
});

test("each message shows when it was sent, how long its turn took, and each tool card how long it ran", async () => {
  const sentAt = new Date().toISOString();
  const clock = new Date(sentAt).toLocaleTimeString("en", { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const turn = await send("Check the deed");
  await chatEvent("started", { text: "Check the deed", sent_at: sentAt }, 1, turn);
  assert.equal($$(".chat-time").at(-1).textContent, clock, "the time it was sent, until it ends");
  await chatEvent("tool_call", { id: "c20", name: "inspect_docx", input: {} }, 1, turn);
  assert.equal(text(".tool-state", $$(".chat-tool").at(-1)), "Running");
  await chatEvent("tool_done", { id: "c20", is_error: false, ms: 1200 }, 1, turn);
  assert.equal(text(".tool-state", $$(".chat-tool").at(-1)), "Done · 1.2 s");

  const timing = { sent_at: sentAt, ms: 38200, tools: { c20: 1200 } };
  backend.db.chats[1].messages.push(
    { ...stored(8, "user", [{ type: "text", text: "Check the deed" }]), message: { role: "user", content: [{ type: "text", text: "Check the deed" }], timing } },
    stored(9, "assistant", [{ type: "tool-call", toolCallId: "c20", toolName: "inspect_docx", input: {} }, { type: "text", text: "It has three blanks." }]),
    stored(10, "tool", [{ type: "tool-result", toolCallId: "c20", toolName: "inspect_docx", output: { type: "json", value: {} } }])
  );
  await chatEvent("done", { stopped: null, timing }, 1, turn);
  assert.equal($$(".chat-time").at(-1).textContent, `${clock} · 38 s · model 37 s · tools 1.2 s`, "as stored");
  assert.equal(text(".tool-state", $$(".chat-tool").at(-1)), "Done · 1.2 s");
  assert.equal($$(".chat-time").length, 1, "turns stored before timing show none");
});

test("a turn stopped at the step limit says how to go on", async () => {
  const turn = await send("Go on");
  await chatEvent("started", { text: "Go on" }, 1, turn);
  assert.equal(lastNote(), null, "the previous turn's note goes");
  await chatEvent("done", { stopped: "step_limit" }, 1, turn);
  assert.equal(lastNote(), "The assistant stopped after many steps; ask it to continue");
});

test("an error says what went wrong and puts the text back in the composer", async () => {
  let turn = await send("Draft the power of attorney");
  assert.equal(composer().value, "");
  await chatEvent("started", { text: "Draft the power of attorney" }, 1, turn);
  await chatEvent("error", { code: "chat_budget", message: "The month's budget is used up", ms: 3200 }, 1, turn);
  assert.equal(text(".chat-error"), "This month's assistant budget is used up after 3.2 s");
  assert.equal(composer().value, "Draft the power of attorney");
  assert.ok(!userTexts().includes("Draft the power of attorney"), "nothing of it was stored");
  assert.deepEqual(spinning(), []);
  assert.ok(button("Send"));

  turn = await send(composer().value);
  await chatEvent("started", { text: "Draft the power of attorney" }, 1, turn);
  assert.equal($(".chat-error"), null);
  await chatEvent("error", { code: "turn_exists", message: "This turn was already started" }, 1, turn);
  assert.equal(text(".chat-error"), "This turn was already started", "an unknown code shows the message");
  assert.equal(composer().value, "Draft the power of attorney");
});

test("after a lock and unlock, chat_running says again which cases run", async () => {
  backend.db.chatRunning = [2];
  await click($("#lock-btn"));
  assert.equal(text(".gate h1"), "Blankey is locked");
  await type($(".gate input[name=password]"), "pw");
  await submit($(".gate form"));
  assert.ok($(".app"));
  assert.deepEqual(spinning(), ["Lease, Main Road"]);
  assert.equal(composer().value, "", "the draft went with the lock");
  await chatEvent("done", { stopped: "cancelled" }, 2, "turn-9");
  assert.deepEqual(spinning(), []);
});

test("the Chat tab goes when the relay stops offering the assistant", async () => {
  backend.db.relay.chat = false;
  backend.emit("relay-status");
  await settle();
  assert.ok(!chips().includes("Chat"));
  assert.ok(text(".case-tabs .chip.active").startsWith("Files"));
  assert.equal($(".chat-composer"), null);
});
