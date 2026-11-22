// The Blankey Online account: signing in from the sidebar, what Settings shows once connected, web agents.

export default {
  id: "online",
  start: "signed-out",
  async run(app) {
    await app.shot("signed-out", { target: ".sidebar-foot", mark: "#relay-btn" });
    await app.click("#relay-btn");
    await app.shot("signing-in", { target: ".sidebar-foot", mark: "#relay-btn" });
    // the sign-in itself happens in the browser, on the Blankey Online page
    await app.backend((backend, fixtures) => {
      Object.assign(backend.db.relay, fixtures.seed().relay, { chat: true });
      backend.emit("relay-status");
    });
    await app.shot("connected", { target: ".sidebar-foot", mark: "#relay-btn" });
    await app.click("#relay-btn");
    await app.show("#relay-state");
    await app.shot("settings");
    await app.click({ text: "Revoke" });
    await app.shot("revoke", { target: ".modal-card" });
    await app.press("Escape");
  },
};
