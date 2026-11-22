// The built-in AI assistant: the chat of a case, a turn as it runs, and what it asks to be approved in Blankey.

const OAK = ".case-item[data-case='1']";
const QUESTION = "Fill the sale contract for the deed on 14 November.";

/** A chat event of the Oak Street case's running turn, as the backend sends them. */
const chatEvent = (app, event, data) =>
  app.backend((backend, fixtures, [event, data]) => backend.emit("chat-event", { case_id: 1, turn_id: "turn-1", event, data }), [event, data]);

/** Puts a request of the assistant in front of the user, as when it arrives over MCP. */
const request = (app, kind, payload) =>
  app.backend(
    (backend, fixtures, request) => {
      backend.db.requests.push(request);
      backend.emit("request-created");
    },
    { id: 40 + kind.length, kind, payload },
  );

export default {
  id: "assistant",
  start: "assistant",
  async run(app) {
    await app.click(".case-item[data-case='2']");
    await app.tab("AI assistant");
    await app.shot("start");

    // a document dropped on the chat, as the window gets it from the desktop
    await app.backend((backend) => {
      const box = document.querySelector(".chat-composer textarea").getBoundingClientRect();
      backend.emit("tauri://drag-drop", { paths: ["/Users/demo/Documents/lease-draft.docx"], position: { x: box.x + 20, y: box.y + 10 } });
    });
    await app.wait(".chat-composer .chat-file");
    await app.fill(".chat-composer textarea", await app.t("Create a template from this document"));
    await app.shot("files");
    await app.fill(".chat-composer textarea", "");

    await app.click(OAK);
    await app.tab("AI assistant");
    await app.click(".chat-reasoning summary");
    await app.shot("thread");

    await app.fill(".chat-composer textarea", QUESTION);
    await app.press("Enter");
    await chatEvent(app, "started", { text: QUESTION });
    await chatEvent(app, "thinking", { text: "Checking which of the buyer's fields are still empty" });
    await chatEvent(app, "text_delta", { text: "Peter Sample's EGN is still missing, so I am asking you for it in Blankey." });
    await chatEvent(app, "status", { state: "waiting" });
    await app.shot("turn");

    await request(app, "profile_input", {
      reason: "The sale contract needs the buyer's EGN",
      profile_id: 4,
      fields: [{ key: "egn", label: "EGN", type: "egn", required: true }],
    });
    await app.shot("request-profile");
    await app.press("Escape");

    await request(app, "collection_link", {
      reason: "Ask the buyer for his details himself",
      title: "Buyer's details, 12 Oak Street",
      profile_id: null,
      new_profile: "Peter Sample",
      fields: [
        { key: "egn", label: "EGN", type: "egn", required: true },
        { key: "phone", label: "Phone", type: "text" },
      ],
      max_files: 1,
      expires_days: 30,
      max_submissions: 1,
    });
    await app.shot("request-link");
    await app.press("Escape");

    await request(app, "fill", { reason: "Fill the sale contract for 12 Oak Street", templates: ["oak-sale-contract"], case_id: 1, profiles: { seller: 3, buyer: 4 } });
    await app.shot("request-fill");
    await app.press("Escape");

    await chatEvent(app, "done", {});
    await app.goTo("Activity");
    await app.shot("activity");

    await app.backend((backend) => {
      backend.db.relay.chat = false;
      backend.emit("relay-status");
    });
    await app.goTo("Cases");
    await app.click(OAK);
    await app.shot("copy-link", { mark: ".copy-case" });
  },
};
