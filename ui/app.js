const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const esc = (value) =>
  String(value ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const when = (ts) => (ts ? String(ts).replace("T", " ").slice(0, 16) : "");
const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

const ICONS = {
  activity: '<path d="M8 2.2a5.8 5.8 0 1 1-5.6 4.3" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/><path d="M1.6 3.4l.9 3.2 3.1-1" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/><path d="M8 4.8V8l2.2 1.4" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>',
  fillsets: '<rect x="2.5" y="2.5" width="11" height="11" rx="2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5 6h6M5 8.5h6M5 11h3.5" stroke="currentColor" stroke-width="1.4"/>',
  documents: '<path d="M4 1.8h5l3 3v9.4H4z" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/><path d="M9 1.8v3h3" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  templates: '<rect x="2" y="3.5" width="8.5" height="10.5" rx="1.2" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5.5 3.5V2h8.5v10.5h-3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/>',
  profiles: '<circle cx="8" cy="5.5" r="2.8" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M2.8 14c.6-2.8 2.7-4.2 5.2-4.2s4.6 1.4 5.2 4.2" fill="none" stroke="currentColor" stroke-width="1.4"/>',
  person: '<circle cx="8" cy="5.2" r="3" fill="currentColor"/><path d="M2.5 14.5c.6-3 2.8-4.6 5.5-4.6s4.9 1.6 5.5 4.6z" fill="currentColor"/>',
  doc: '<path d="M3.5 1.5h5.5l3.5 3.5v9.5h-9z" fill="currentColor"/><path d="M5.5 8h5M5.5 10.5h5" stroke="var(--bg)" stroke-width="1.2"/>',
};

const PLACEHOLDERS = {
  date: "DD.MM.YYYY",
  egn: "10 digits",
  iban: "BG00 XXXX 0000 0000 0000 00",
  number: "1 500,00",
  email: "name@example.com",
};

const state = { status: null, page: "activity", search: "", processing: false, gateReason: "" };

// -- helpers ----------------------------------------------------------------------

function toast(message, ms = 2600) {
  const el = $("#toast");
  el.textContent = message;
  el.classList.remove("hidden");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => el.classList.add("hidden"), ms);
}

const isLocked = (e) => String(e).includes("locked");

async function call(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    if (isLocked(e)) await refreshStatus();
    else toast(String(e), 5000);
    throw e;
  }
}

/** fields: [{name, label, value, type}] -> resolves to {name: value}, or null when cancelled */
function dialog({ title, text = "", fields = [], ok = "OK", danger = false, cancel = "Cancel" }) {
  const modal = $("#modal");
  const form = $("#modal-form");
  form.innerHTML = `
    <h3>${esc(title)}</h3>
    ${text ? `<div class="muted" style="white-space: pre-line">${esc(text)}</div>` : ""}
    ${fields
      .map(
        (f) => `<label class="small muted">${esc(f.label)}</label>
        <input type="${f.type || "text"}" name="${esc(f.name)}" value="${esc(f.value || "")}" spellcheck="false">`
      )
      .join("")}
    <div class="modal-buttons">
      ${cancel ? `<button type="button" data-cancel>${esc(cancel)}</button>` : ""}
      <button type="submit" class="${danger ? "primary danger" : "primary"}">${esc(ok)}</button>
    </div>`;
  modal.classList.remove("hidden");
  (form.querySelector("input") || form.querySelector("button[type=submit]")).focus();
  return new Promise((resolve) => {
    const close = (value) => {
      modal.classList.add("hidden");
      form.onsubmit = null;
      form.onkeydown = null;
      resolve(value);
    };
    form.onsubmit = (e) => {
      e.preventDefault();
      close(Object.fromEntries(new FormData(form)));
    };
    const cancelButton = form.querySelector("[data-cancel]");
    if (cancelButton) cancelButton.onclick = () => close(null);
    form.onkeydown = (e) => e.key === "Escape" && close(null);
  });
}

const confirmDialog = (title, text, ok = "Delete") => dialog({ title, text, ok, danger: true }).then(Boolean);
const alertDialog = (title, text) => dialog({ title, text, ok: "OK", cancel: "" });

function matches(...values) {
  const q = state.search.trim().toLowerCase();
  return !q || values.some((v) => String(v ?? "").toLowerCase().includes(q));
}

/** A pop-up menu at (x, y). items: [{label, checked, action}] | "-" | {head}. */
function popup(x, y, items, minWidth = 0) {
  const menu = $("#menu");
  menu.innerHTML = "";
  for (const item of items) {
    const el = document.createElement("div");
    if (item === "-") el.className = "menu-sep";
    else if (item.head) {
      el.className = "menu-head";
      el.textContent = item.head;
    } else {
      el.className = `menu-item${item.checked ? " checked" : ""}`;
      el.textContent = item.label;
      el.title = item.label;
      el.onclick = () => {
        hidePopup();
        item.action();
      };
    }
    menu.append(el);
  }
  menu.style.minWidth = `${Math.max(260, minWidth)}px`;
  menu.classList.remove("hidden");
  const rect = menu.getBoundingClientRect();
  menu.style.left = `${Math.min(x, window.innerWidth - rect.width - 8)}px`;
  menu.style.top = `${y + rect.height > window.innerHeight - 8 ? Math.max(8, y - rect.height - 36) : y}px`;
}

function hidePopup() {
  $("#menu").classList.add("hidden");
}

document.addEventListener("mousedown", (e) => {
  if (!$("#menu").contains(e.target)) hidePopup();
});

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

// -- gate: setup / unlock / recovery ---------------------------------------------------

function showGate(mode) {
  $("#app").classList.add("hidden");
  $("#sheet").classList.add("hidden");
  $("#gate").classList.remove("hidden");
  $("#gate-error").textContent = "";
  const form = $("#gate-form");
  const title = $("#gate-title");
  const text = $("#gate-text");
  const status = state.status || {};
  const keyringLabel = status.biometric ? "Unlock with Touch ID" : "Unlock automatically with the keychain";

  if (mode === "setup") {
    title.textContent = "Create your vault";
    text.textContent = "Personal data is encrypted with this password. Without it or the recovery key the data cannot be read.";
    form.innerHTML = `
      <input type="password" name="password" placeholder="Master password (8+ characters)" autofocus>
      <input type="password" name="confirm" placeholder="Repeat password">
      <label class="check small"><input type="checkbox" name="keyring"> ${esc(keyringLabel)}</label>
      <button class="primary">Create vault</button>`;
    form.onsubmit = gateSubmit(async ({ password, confirm, keyring }) => {
      if (password !== confirm) throw "The passwords do not match.";
      const recovery = await invoke("setup", { password, useKeyring: keyring === "on" });
      showRecovery(recovery);
    });
  } else if (mode === "recovery") {
    title.textContent = "Reset password";
    text.textContent = "Enter the recovery key you saved when the vault was created.";
    form.innerHTML = `
      <input type="text" name="key" placeholder="XXXX-XXXX-…" autofocus class="mono" spellcheck="false">
      <input type="password" name="password" placeholder="New master password">
      <button class="primary">Unlock and set password</button>
      <button type="button" class="link" data-back>Back</button>`;
    form.querySelector("[data-back]").onclick = () => showGate("unlock");
    form.onsubmit = gateSubmit(async ({ key, password }) => {
      await invoke("unlock_recovery", { recoveryKey: key, newPassword: password });
      await enterApp();
    });
  } else {
    title.textContent = "Blankey is locked";
    text.textContent =
      state.gateReason ||
      (status.pending ? `${plural(status.pending, "request")} from Claude ${status.pending === 1 ? "is" : "are"} waiting.` : "Enter your master password.");
    const quick = status.keyring_enabled;
    form.innerHTML = `
      <input type="password" name="password" placeholder="Master password" autofocus>
      <button class="primary">Unlock</button>
      ${quick ? `<button type="button" data-quick>${status.biometric ? "Use Touch ID" : "Use keychain"}</button>` : ""}
      <button type="button" class="link" data-recovery>Forgot password?</button>`;
    form.querySelector("[data-recovery]").onclick = () => showGate("recovery");
    if (quick) form.querySelector("[data-quick]").onclick = () => quickUnlock();
    form.onsubmit = gateSubmit(async ({ password }) => {
      await invoke("unlock", { password });
      await enterApp();
    });
    if (quick) tryQuickOnce();
  }
  form.querySelector("input")?.focus();
}

/** Touch ID / keychain once per lock, and only while the window is in front (never from a hidden window). */
function tryQuickOnce() {
  if (showGate.triedQuick || !document.hasFocus() || $("#gate").classList.contains("hidden")) return;
  if (!state.status?.keyring_enabled || state.status?.unlocked) return;
  showGate.triedQuick = true;
  quickUnlock();
}

window.addEventListener("focus", () => tryQuickOnce());

async function quickUnlock() {
  if (await invoke("unlock_quick", { reason: state.gateReason ? "open Claude's request" : null })) await enterApp();
}

function gateSubmit(action) {
  return async (e) => {
    e.preventDefault();
    const form = e.target;
    const button = form.querySelector("button.primary");
    $("#gate-error").textContent = "";
    button.disabled = true;
    try {
      await action(Object.fromEntries(new FormData(form)));
    } catch (err) {
      $("#gate-error").textContent = String(err);
      form.querySelector("input[type=password]")?.select();
    } finally {
      button.disabled = false;
    }
  };
}

function showRecovery(recovery) {
  $("#gate-title").textContent = "Save your recovery key";
  $("#gate-text").textContent = "Store it somewhere safe. It is shown only once and lets you set a new password if you forget the current one.";
  const form = $("#gate-form");
  form.innerHTML = `
    <div class="recovery">${esc(recovery)}</div>
    <button type="button" data-copy>Copy</button>
    <label class="check small"><input type="checkbox" name="saved"> I saved it somewhere safe</label>
    <button class="primary" disabled>Continue</button>`;
  form.querySelector("[data-copy]").onclick = async () => {
    await invoke("copy_text", { text: recovery });
    toast("Recovery key copied");
  };
  const button = form.querySelector("button.primary");
  form.saved.onchange = () => (button.disabled = !form.saved.checked);
  form.onsubmit = (e) => {
    e.preventDefault();
    enterApp();
  };
}

// -- shell ------------------------------------------------------------------------------

const PAGES = {};
const SECTIONS = [
  ["Claude", ["activity"]],
  ["Library", ["fillsets", "documents", "templates"]],
  ["Vault", ["profiles"]],
];

async function refreshStatus() {
  state.status = await invoke("status");
  if (!state.status.initialized) showGate("setup");
  else if (!state.status.unlocked) showGate("unlock");
  else renderShellStatus();
  return state.status;
}

function renderShellStatus() {
  const s = state.status;
  $("#mcp-status").innerHTML = `<span class="dot ${s.mcp_running ? "up" : "down"}"></span>${
    s.mcp_running ? `MCP on port ${esc(new URL(s.mcp_url).port)}` : "MCP is down"
  }`;
  $("#mcp-status").title = s.mcp_url;
  renderNav();
}

async function enterApp() {
  const status = await refreshStatus();
  if (!status.unlocked) return;
  state.gateReason = "";
  showGate.triedQuick = false;
  $("#gate").classList.add("hidden");
  $("#app").classList.remove("hidden");
  if (status.pending && state.page !== "activity") state.page = "activity";
  await showPage(state.page);
  processRequests();
}

function renderNav() {
  const pending = state.status?.pending || 0;
  $("#nav").innerHTML = SECTIONS.map(
    ([section, pages]) =>
      `<div class="nav-section">${section}</div>` +
      pages
        .map((id) => {
          const badge = id === "activity" && pending ? `<span class="badge">${pending}</span>` : "";
          return `<div class="nav-item ${id === state.page ? "active" : ""}" data-page="${id}">
            <svg viewBox="0 0 16 16">${ICONS[id]}</svg>${PAGES[id].title}${badge}</div>`;
        })
        .join("")
  ).join("");
  $$(".nav-item", $("#nav")).forEach((el) => (el.onclick = () => showPage(el.dataset.page)));
}

async function showPage(id) {
  state.page = id;
  state.search = "";
  $("#search").value = "";
  renderNav();
  const page = PAGES[id];
  $("#page-title").textContent = page.title;
  $("#page-subtitle").textContent = "";
  $("#page-actions").innerHTML = "";
  $("#page").innerHTML = "";
  await page.render();
}

const rerender = () => PAGES[state.page].render();

function setSubtitle(text) {
  $("#page-subtitle").textContent = text;
}

function setActions(buttons) {
  const box = $("#page-actions");
  box.innerHTML = "";
  for (const [label, handler, cls = ""] of buttons) {
    const b = document.createElement("button");
    b.textContent = label;
    b.className = cls;
    b.onclick = handler;
    box.append(b);
  }
}

/** A selectable table. columns: [{title, cell(row) -> html, cls}] */
function table(rows, columns, { selected = [], onSelect, onOpen, multi = false, rowClass = () => "" }) {
  const html = `<table class="grid"><thead><tr>${columns.map((c) => `<th>${esc(c.title)}</th>`).join("")}</tr></thead>
    <tbody>${rows
      .map(
        (r) => `<tr data-id="${r.id}" class="${selected.includes(r.id) ? "selected" : ""} ${rowClass(r)}">${columns
          .map((c) => `<td class="${c.cls || ""}">${c.cell(r)}</td>`)
          .join("")}</tr>`
      )
      .join("")}</tbody></table>`;
  $("#page").innerHTML = html;
  let current = [...selected];
  $$("tbody tr", $("#page")).forEach((tr) => {
    const id = Number(tr.dataset.id);
    tr.onclick = (e) => {
      if (multi && (e.metaKey || e.ctrlKey)) current = current.includes(id) ? current.filter((x) => x !== id) : [...current, id];
      else current = [id];
      $$("tbody tr", $("#page")).forEach((row) => row.classList.toggle("selected", current.includes(Number(row.dataset.id))));
      onSelect?.(current);
    };
    tr.ondblclick = () => onOpen?.(id);
  });
}

// -- activity -----------------------------------------------------------------------------

PAGES.activity = {
  title: "Activity",
  selected: null,
  async render() {
    const rows = await call("list_activity");
    const pending = rows.filter((r) => r.status === "pending").length;
    setSubtitle(pending ? `${pending} waiting` : "Nothing waiting");
    setActions([["Open", () => this.open(this.selected)]]);
    const visible = rows.filter((r) => matches(r.label, r.details, r.outcome));
    this.rows = rows;
    if (!visible.length) {
      $("#page").innerHTML = `<div class="empty">Nothing from Claude yet.<br><span class="small">Requests for data and documents show up here.</span></div>`;
      return;
    }
    table(
      visible,
      [
        { title: "Request", cell: (r) => esc(r.label), cls: "nowrap" },
        { title: "Details", cell: (r) => esc(r.details) },
        { title: "Outcome", cell: (r) => `<span class="status ${esc(r.status)}">${esc(r.outcome)}</span>`, cls: "nowrap" },
        { title: "Time", cell: (r) => `<span class="muted">${esc(when(r.time))}</span>`, cls: "nowrap" },
      ],
      {
        selected: [this.selected],
        onSelect: ([id]) => (this.selected = id),
        onOpen: (id) => this.open(id),
        rowClass: (r) => (r.status === "pending" ? "pending" : ""),
      }
    );
  },
  async open(id) {
    const row = this.rows?.find((r) => r.id === id);
    if (!row) return toast("Select a request");
    if (row.status === "pending") return processRequests();
    if (row.kind === "fill" && row.fill_set_id) return openFill({ fill_set_id: row.fill_set_id });
    if (row.document_ids.length) return call("open_documents", { ids: row.document_ids });
    toast("Nothing to open for this request");
  },
};

// -- fill sets ------------------------------------------------------------------------------

PAGES.fillsets = {
  title: "Fill sets",
  selected: null,
  async render() {
    const sets = await call("list_fill_sets");
    setSubtitle(plural(sets.length, "set"));
    const need = (fn) => () => (this.selected ? fn(this.selected) : toast("Select a fill set"));
    setActions([
      ["Open", need((id) => openFill({ fill_set_id: id }))],
      ["Generate", need((id) => this.generate(id))],
      ["Export", need((id) => this.export(id, false))],
      ["Export bundle", need((id) => this.export(id, true))],
      ["Import", () => this.import()],
      ["Delete", need((id) => this.remove(sets.find((s) => s.id === id))), "danger"],
    ]);
    const visible = sets.filter((s) => matches(s.name, ...s.templates, ...s.profiles));
    if (!visible.length) {
      $("#page").innerHTML = `<div class="empty">No fill sets yet. Ask Claude to fill a template.</div>`;
      return;
    }
    table(
      visible,
      [
        { title: "Name", cell: (s) => `<b>${esc(s.name)}</b>` },
        { title: "Templates", cell: (s) => esc(s.templates.join(", ")) },
        { title: "Profiles", cell: (s) => esc(s.profiles.join(", ")) },
        { title: "Updated", cell: (s) => `<span class="muted">${esc(when(s.updated_at))}</span>`, cls: "nowrap" },
      ],
      { selected: [this.selected], onSelect: ([id]) => (this.selected = id), onOpen: (id) => openFill({ fill_set_id: id }) }
    );
  },
  async generate(id) {
    document.body.classList.add("busy");
    try {
      const ids = await call("generate_fill_set", { id });
      toast(`${plural(ids.length, "document")} generated`);
    } finally {
      document.body.classList.remove("busy");
    }
  },
  async export(id, bundle) {
    const path = await call("export_fill_set", { id, bundle });
    if (path) toast(`Exported to ${path}`, 4000);
  },
  async import() {
    const id = await call("import_fill_set");
    if (id) {
      this.selected = id;
      toast("Fill set imported");
      this.render();
    }
  },
  async remove(set) {
    if (!(await confirmDialog(`Delete the fill set "${set.name}"?`, "Generated documents are kept."))) return;
    await call("delete_fill_set", { id: set.id });
    this.selected = null;
    this.render();
  },
};

// -- documents ----------------------------------------------------------------------------------

PAGES.documents = {
  title: "Documents",
  selected: [],
  async render() {
    const docs = await call("list_documents");
    this.docs = docs;
    setSubtitle("Encrypted in the vault");
    const need = (fn) => () => (this.selected.length ? fn(this.selected) : toast("Select a document"));
    setActions([
      ["Open", need((ids) => call("open_documents", { ids }))],
      ["Export", need(async ([id]) => {
        const path = await call("export_document", { id });
        if (path) toast(`Saved to ${path}`, 4000);
      })],
      ["Delete", need((ids) => this.remove(ids)), "danger"],
    ]);
    const visible = docs.filter((d) => matches(d.title, d.template_id, d.filename));
    if (!visible.length) {
      $("#page").innerHTML = `<div class="empty">No documents yet. Documents are generated from fill sets.</div>`;
      return;
    }
    table(
      visible,
      [
        { title: "Title", cell: (d) => esc(d.title) },
        { title: "Template", cell: (d) => `<span class="mono">${esc(d.template_id)}</span>` },
        { title: "Kind", cell: (d) => (d.filename.endsWith(".pdf") ? "PDF" : "Word"), cls: "nowrap" },
        { title: "Pages", cell: (d) => (d.pages || "-"), cls: "nowrap" },
        { title: "Created", cell: (d) => `<span class="muted">${esc(when(d.created_at))}</span>`, cls: "nowrap" },
      ],
      {
        selected: this.selected,
        multi: true,
        onSelect: (ids) => (this.selected = ids),
        onOpen: (id) => call("open_documents", { ids: [id] }),
      }
    );
  },
  async remove(ids) {
    const docs = this.docs.filter((d) => ids.includes(d.id));
    const text = docs.length === 1 ? `Delete "${docs[0].title}"?` : `Delete ${docs.length} documents?`;
    if (!(await confirmDialog(text, "They are removed from the vault. Copies you exported stay where you saved them."))) return;
    await call("delete_documents", { ids });
    this.selected = [];
    this.render();
  },
};

// -- templates -----------------------------------------------------------------------------------

const KIND_NAMES = { docx: "Word", pdf_form: "PDF", typst: "Typst" };

PAGES.templates = {
  title: "Templates",
  selected: null,
  async render() {
    const templates = await call("list_templates");
    this.templates = templates;
    setSubtitle(plural(templates.length, "template"));
    const need = (fn) => () => (this.selected ? fn(this.selected) : toast("Select a template"));
    setActions([
      ["Fill", need((id) => openFill({ templates: [id] }))],
      ["Copy ID", need((id) => this.copyId(id))],
      ["Show in Finder", () => call("show_template_folder", { id: this.selected })],
      ["Delete", need((id) => this.remove(id)), "danger"],
    ]);
    const visible = templates.filter((t) => matches(t.name, t.id, t.description));
    if (!templates.length) {
      $("#page").innerHTML = `<div class="empty">No templates yet. Ask Claude to make one from a document.<br>
        <span class="small mono selectable">${esc(state.status.data_dir)}/templates</span></div>`;
      return;
    }
    $("#page").innerHTML = `<div class="split"><div class="list">${visible
      .map((t) => {
        const meta = [KIND_NAMES[t.kind] || t.kind, plural(t.blanks, "blank")];
        if (t.used_by.length) meta.push(plural(t.used_by.length, "fill set"));
        return `<div class="card" data-id="${esc(t.id)}"><div class="title">${esc(t.name)}</div>
          <div class="muted small">${esc(meta.join("  ·  "))}</div>
          ${t.description ? `<div class="desc muted">${esc(t.description)}</div>` : ""}</div>`;
      })
      .join("")}</div><div class="detail" id="tpl-detail"></div></div>`;
    $$(".card", $("#page")).forEach((el) => {
      el.onclick = () => this.select(el.dataset.id);
      el.ondblclick = () => openFill({ templates: [el.dataset.id] });
    });
    const first = visible.some((t) => t.id === this.selected) ? this.selected : visible[0]?.id;
    if (first) this.select(first);
  },
  async select(id) {
    this.selected = id;
    $$(".card", $("#page")).forEach((el) => el.classList.toggle("active", el.dataset.id === id));
    const t = this.templates.find((x) => x.id === id);
    const data = await call("template_preview", { id });
    if (this.selected !== id) return;
    // built off-screen and swapped in once the page images are decoded, so the old preview stays until then
    const detail = document.createElement("div");
    const info = [
      `<span class="mono selectable">${esc(t.id)}</span>`,
      plural(data.blanks, "blank"),
      `roles: ${esc(Object.keys(data.roles).join(", ") || "-")}`,
    ];
    if (t.used_by.length) info.push(`used by: ${esc(t.used_by.join(", "))}`);
    detail.innerHTML = `
      <div class="detail-head"><div style="flex:1;min-width:0"><h3>${esc(t.name)}</h3>
      <div class="muted small">${info.join(" &nbsp;·&nbsp; ")}</div></div>
      <button data-copy>Copy ID</button></div>
      <div class="preview-area" id="tpl-preview" style="margin-top:12px"></div>
      <h4>Bindings</h4>
      <table class="kv selectable"><tbody>${Object.entries(data.fields)
        .map(
          ([name, expr]) => `<tr><td><span class="mono">${esc(name)}</span>${
            data.labels[name] ? `<div class="muted small">${esc(data.labels[name])}</div>` : ""
          }</td><td class="mono">${esc(expr) || '<span class="muted">(asked at fill time)</span>'}</td></tr>`
        )
        .join("")}</tbody></table>
      ${data.examples.length ? `<h4>Examples</h4>${data.examples.map((e) => `<span class="pill">${esc(e)}</span>`).join("")}` : ""}`;
    detail.querySelector("[data-copy]").onclick = () => this.copyId(id);
    renderPreview(detail.querySelector("#tpl-preview"), data.preview, { interactive: false });
    await Promise.all([...detail.querySelectorAll("img")].map((img) => img.decode().catch(() => {})));
    if (this.selected !== id) return;
    $("#tpl-detail").replaceChildren(...detail.childNodes);
  },
  async copyId(id) {
    await call("copy_text", { text: id });
    toast(`Copied ${id}`);
  },
  async remove(id) {
    const t = this.templates.find((x) => x.id === id);
    let text = "Its folder and examples are removed. Generated documents are kept.";
    if (t.used_by.length) text += ` Fill sets using it will no longer open: ${t.used_by.join(", ")}.`;
    if (!(await confirmDialog(`Delete the template "${t.name}"?`, text))) return;
    await call("delete_template", { id });
    this.selected = null;
    this.render();
  },
};

/** Renders a preview (html or page images with field boxes) into `container`. */
function renderPreview(container, preview, { interactive = true, onPick } = {}) {
  if (!preview) return;
  if (preview.type === "error") {
    container.innerHTML = `<div class="preview-error">No preview: ${esc(preview.message)}</div>`;
    return;
  }
  if (preview.type === "html") {
    container.innerHTML = `<div class="zoom"><div class="paper">${preview.html}</div></div>`;
    if (onPick) $$(".blank", container).forEach((el) => (el.onclick = () => onPick(el.dataset.var)));
    enableZoom(container);
    return;
  }
  const pages = preview.pages
    .map((page) => {
      const boxes = Object.entries(page.boxes)
        .flatMap(([name, rects]) =>
          rects.map(([x0, y0, x1, y1]) => {
            const style = `left:${(x0 / page.width) * 100}%;top:${((page.height - y1) / page.height) * 100}%;width:${
              ((x1 - x0) / page.width) * 100
            }%;height:${((y1 - y0) / page.height) * 100}%`;
            return `<div class="box" data-var="${esc(name)}" style="${style}" title="${esc(name)}"></div>`;
          })
        )
        .join("");
      return `<div class="pdf-page ${interactive ? "" : "static"}" style="aspect-ratio:${page.width}/${page.height}">
        <img src="${page.image}" alt="">${boxes}</div>`;
    })
    .join("");
  container.innerHTML = `<div class="zoom">${pages}</div>`;
  if (onPick) $$(".box", container).forEach((el) => (el.onclick = () => onPick(el.dataset.var)));
  enableZoom(container);
}

const ZOOM_MIN = 0.5;
const ZOOM_MAX = 4;

/** Pinch (trackpad or ctrl+wheel) zooms the preview around the pointer; the level survives re-renders. */
function enableZoom(container) {
  const apply = () => {
    const content = container.querySelector(".zoom");
    if (!content) return;
    content.style.setProperty("--zoom", container.zoom);
    content.style.width = `${container.zoom * 100}%`;
  };
  if (!container.zoom) container.zoom = 1;
  apply();
  if (container.zoomBound) return;
  container.zoomBound = true;
  const zoomTo = (level, x, y) => {
    const old = container.zoom;
    const next = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, level));
    if (next === old) return;
    const rect = container.getBoundingClientRect();
    const px = x - rect.left;
    const py = y - rect.top;
    const contentX = (container.scrollLeft + px) / old;
    const contentY = (container.scrollTop + py) / old;
    container.zoom = next;
    apply();
    container.scrollLeft = contentX * next - px;
    container.scrollTop = contentY * next - py;
  };
  container.addEventListener(
    "wheel",
    (e) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      zoomTo(container.zoom * Math.exp(-e.deltaY * 0.01), e.clientX, e.clientY);
    },
    { passive: false }
  );
  // WebKit sends pinches as gesture events instead of ctrl+wheel
  let start = 1;
  container.addEventListener("gesturestart", (e) => {
    e.preventDefault();
    start = container.zoom;
  });
  container.addEventListener("gesturechange", (e) => {
    e.preventDefault();
    zoomTo(start * e.scale, e.clientX, e.clientY);
  });
  container.addEventListener("dblclick", (e) => {
    if (e.target.closest(".box, .blank")) return;
    zoomTo(1, e.clientX, e.clientY);
  });
}

function highlightPreview(container, name, scroll) {
  let first = null;
  $$("[data-var]", container).forEach((el) => {
    const on = el.dataset.var === name;
    el.classList.toggle("active", on);
    if (on && !first) first = el;
  });
  if (first && scroll) first.scrollIntoView({ block: "center", behavior: "smooth" });
}

// -- profiles ---------------------------------------------------------------------------------------

PAGES.profiles = {
  title: "Profiles",
  selected: null,
  reveal: false,
  dirty: false,
  async render() {
    const profiles = await call("list_profiles");
    this.profiles = profiles;
    setSubtitle(`${plural(profiles.length, "profile")}, values encrypted per field`);
    setActions([
      ["New profile", () => this.create()],
      ["Rename", () => this.rename()],
      ["Delete profile", () => this.remove(), "danger"],
    ]);
    const visible = profiles.filter((p) => matches(p.name, p.kind));
    $("#page").innerHTML = `<div class="split"><div class="list">${
      visible
        .map(
          (p) => `<div class="list-item" data-id="${p.id}"><div class="title">${esc(p.name)}</div>
            <div class="muted small">${esc(p.kind || "profile")} · #${p.id}</div></div>`
        )
        .join("") || '<div class="empty">No profiles</div>'
    }</div><div class="detail" id="profile-detail"><div class="empty">Create a profile to store personal data.</div></div></div>`;
    $$(".list-item", $("#page")).forEach((el) => (el.onclick = () => this.select(Number(el.dataset.id))));
    if (visible.length) this.select(visible.some((p) => p.id === this.selected) ? this.selected : visible[0].id);
  },

  filter() {
    $$(".list-item", $("#page")).forEach((el) => {
      const p = this.profiles.find((x) => x.id === Number(el.dataset.id));
      el.classList.toggle("hidden", !matches(p?.name, p?.kind));
    });
  },

  async create() {
    const values = await dialog({
      title: "New profile",
      fields: [
        { name: "name", label: "Name" },
        { name: "kind", label: "Kind (person, company, loan…)", value: "person" },
      ],
      ok: "Create",
    });
    if (!values?.name?.trim()) return;
    this.selected = await call("create_profile", values);
    this.render();
  },

  current() {
    return this.profiles?.find((x) => x.id === this.selected);
  },

  async rename() {
    const p = this.current();
    if (!p) return toast("Select a profile");
    const values = await dialog({
      title: "Rename profile",
      fields: [
        { name: "name", label: "Name", value: p.name },
        { name: "kind", label: "Kind", value: p.kind },
      ],
      ok: "Save",
    });
    if (!values) return;
    await call("update_profile", { id: p.id, name: values.name.trim() || p.name, kind: values.kind });
    this.render();
  },

  async remove() {
    const p = this.current();
    if (!p) return toast("Select a profile");
    if (!(await confirmDialog(`Delete the profile "${p.name}"?`, "All its fields are removed from the vault."))) return;
    await call("delete_profile", { id: p.id });
    this.selected = null;
    this.render();
  },

  async select(id) {
    if (this.dirty && id !== this.selected) {
      const discard = await confirmDialog("Discard unsaved changes?", "The fields of this profile were edited.", "Discard");
      if (!discard) return;
    }
    this.selected = id;
    this.removed = [];
    this.dirty = false;
    $$(".list-item", $("#page")).forEach((el) => el.classList.toggle("active", Number(el.dataset.id) === id));
    const profile = this.current();
    const fields = await call("profile_fields", { id });
    const detail = $("#profile-detail");
    detail.innerHTML = `
      <h3>${esc(profile.name)}</h3>
      <div class="muted small">Claude sees keys, labels, types and value lengths. Never values.</div>
      <table class="fields"><thead><tr><th>Key</th><th>Label</th><th>Type</th><th>Value</th><th></th></tr></thead>
      <tbody id="field-rows"></tbody></table>
      <div class="detail-actions">
        <button id="add-field">Add field</button>
        <label class="check-row small muted"><input type="checkbox" id="reveal" ${this.reveal ? "checked" : ""}> Show values</label>
        <span class="spacer"></span>
        <span id="dirty" class="muted small"></span>
        <button id="save-fields" class="primary">Save</button>
      </div>
      <div id="field-problems" class="problems"></div>`;
    const body = $("#field-rows");
    fields.forEach((f) => body.append(this.row(f)));
    if (!fields.length) body.append(this.row({ key: "", label: "", type: "text", value: "" }));
    $("#add-field").onclick = () => {
      const tr = this.row({ key: "", label: "", type: "text", value: "" });
      body.append(tr);
      tr.querySelector("input").focus();
    };
    $("#reveal").onchange = (e) => {
      this.reveal = e.target.checked;
      $$("input.value", body).forEach((i) => i.classList.toggle("masked", !this.reveal));
    };
    $("#save-fields").onclick = () => this.save(id);
    body.oninput = () => this.markDirty();
    body.onchange = () => this.markDirty();
  },

  row(f) {
    const tr = document.createElement("tr");
    tr.dataset.original = f.key;
    const types = state.status.field_types.map((t) => `<option ${t === f.type ? "selected" : ""}>${t}</option>`).join("");
    tr.innerHTML = `
      <td style="width:26%"><input type="text" class="key mono" value="${esc(f.key)}" placeholder="address.city" spellcheck="false"></td>
      <td style="width:22%"><input type="text" class="label" value="${esc(f.label)}"></td>
      <td style="width:12%"><select class="type">${types}</select></td>
      <td><input type="text" class="value ${this.reveal ? "" : "masked"}" value="${esc(f.value ?? "")}"
        placeholder="${esc(PLACEHOLDERS[f.type] || "")}" spellcheck="false"></td>
      <td style="width:1%"><button class="remove ghost" title="Remove field">✕</button></td>`;
    tr.querySelector(".type").addEventListener("change", (e) => {
      tr.querySelector(".value").placeholder = PLACEHOLDERS[e.target.value] || "";
    });
    tr.querySelector(".remove").onclick = () => {
      if (tr.dataset.original) this.removed.push(tr.dataset.original);
      tr.remove();
      this.markDirty();
    };
    return tr;
  },

  markDirty() {
    this.dirty = true;
    $("#dirty").textContent = "Unsaved changes";
  },

  async save(id) {
    const rows = $$("#field-rows tr");
    const fields = rows.map((tr) => ({
      key: tr.querySelector(".key").value.trim(),
      label: tr.querySelector(".label").value.trim(),
      type: tr.querySelector(".type").value,
      value: tr.querySelector(".value").value,
    }));
    const removed = [...this.removed];
    rows.forEach((tr, i) => {
      const original = tr.dataset.original;
      if (original && original !== fields[i].key) removed.push(original);
    });
    const problems = await call("save_fields", { id, fields: fields.filter((f) => f.key), removed });
    $("#field-problems").textContent = problems.join("\n");
    if (problems.length) {
      $("#dirty").textContent = "Fix the problems below";
      return;
    }
    this.dirty = false;
    toast("Saved");
    this.select(id);
  },
};

// -- requests from Claude -----------------------------------------------------------------------------

/** Opens the waiting requests one after another; asks to unlock first when the vault is locked. */
async function processRequests() {
  if (state.processing) return;
  state.processing = true;
  try {
    for (;;) {
      let detail;
      try {
        detail = await invoke("next_request");
      } catch (e) {
        if (isLocked(e)) {
          state.gateReason = "Claude sent a request. Unlock the vault to continue.";
          await refreshStatus();
        } else toast(String(e), 5000);
        return;
      }
      if (!detail) return;
      const { request } = detail;
      await invoke("set_busy", { busy: true });
      try {
        if (request.kind === "profile_input") await openProfileInput(detail);
        else if (request.kind === "fill") {
          const p = request.payload;
          await openFill({
            templates: p.templates || [],
            profiles: p.profiles || {},
            name: p.name || "",
            values: p.values || {},
            paths: p.paths || {},
            fill_set_id: p.fill_set_id ?? null,
            request_id: request.id,
          });
        } else {
          await invoke("cancel_request", { id: request.id });
        }
      } finally {
        await invoke("set_busy", { busy: false });
      }
      const still = await invoke("next_request").catch(() => null);
      if (still?.request.id === request.id) return;
    }
  } finally {
    state.processing = false;
    await refreshAfterRequests();
  }
}

async function refreshAfterRequests() {
  await refreshStatus();
  if (state.status.unlocked && !$("#app").classList.contains("hidden")) rerender();
}

function sheetHead(icon, title, reason) {
  return `<div class="head-text"><div class="head-icon"><svg viewBox="0 0 16 16">${ICONS[icon]}</svg></div>
    <div style="min-width:0"><h2>${esc(title)}</h2>${reason ? `<div class="reason">${esc(reason)}</div>` : ""}</div></div>`;
}

function showSheet(html, resolve) {
  state.sheetResolve = resolve;
  const sheet = $("#sheet");
  sheet.innerHTML = `<div class="sheet-drag" data-tauri-drag-region></div>${html}`;
  sheet.classList.remove("hidden");
  return sheet;
}

function closeSheet() {
  const sheet = $("#sheet");
  sheet.classList.add("hidden");
  sheet.innerHTML = "";
  const resolve = state.sheetResolve;
  state.sheetResolve = null;
  state.sheetKeys && document.removeEventListener("keydown", state.sheetKeys);
  state.sheetKeys = null;
  resolve?.();
}

function openProfileInput(detail) {
  return new Promise((resolve) => showProfileInput(detail, resolve));
}

function showProfileInput({ request, profile_name, existing }, resolve) {
  const payload = request.payload;
  const specs = payload.fields || [];
  const target = profile_name || "a new profile";
  const sheet = showSheet(`
    <div class="sheet-head">${sheetHead("person", `${plural(specs.length, "field")} for ${target}`, payload.reason)}</div>
    <div class="sheet-body"><form class="input-form" id="input-form" autocomplete="off">
      ${
        payload.profile_id == null
          ? `<label>New profile</label><input type="text" name="__profile" value="${esc(payload.new_profile?.name || "")}">`
          : ""
      }
      ${specs
        .map((spec, i) => {
          const label = (spec.label || spec.key) + (spec.required ? " *" : "");
          const current = existing[spec.key] || "";
          if (spec.type === "bool") {
            const on = ["1", "true", "да", "yes", "x"].includes(current.trim().toLowerCase());
            return `<label>${esc(label)}</label><label class="check-row"><input type="checkbox" name="f${i}" ${on ? "checked" : ""}>
              <span class="muted small">${esc(spec.hint || "")}</span></label>`;
          }
          return `<label title="${esc(spec.key)}">${esc(label)}</label><input type="text" name="f${i}" value="${esc(current)}"
            placeholder="${esc(spec.hint || PLACEHOLDERS[spec.type] || "")}" spellcheck="false">`;
        })
        .join("")}
    </form><div id="input-problems" class="problems" style="max-width:640px;margin:0 auto;padding:0 24px"></div></div>
    <div class="sheet-foot"><span class="note">Values stay in Blankey. Claude only sees the field names and lengths.</span>
      <button data-cancel>Cancel</button><button class="primary" data-save>Save</button></div>`, resolve);
  sheet.querySelector("input")?.focus();
  {
    const done = () => closeSheet();
    sheet.querySelector("[data-cancel]").onclick = async () => {
      await invoke("cancel_request", { id: request.id });
      done();
    };
    const save = async () => {
      const form = $("#input-form");
      const values = {};
      specs.forEach((spec, i) => {
        const input = form.elements[`f${i}`];
        values[spec.key] = input.type === "checkbox" ? (input.checked ? "yes" : "") : input.value;
      });
      const name = form.elements.__profile?.value ?? null;
      const problems = await call("resolve_profile_input", { id: request.id, profileName: name, values });
      $("#input-problems").textContent = problems.join("\n");
      if (!problems.length) {
        toast("Saved");
        done();
      }
    };
    sheet.querySelector("[data-save]").onclick = save;
    $("#input-form").onsubmit = (e) => {
      e.preventDefault();
      save();
    };
  }
}

// -- fill form --------------------------------------------------------------------------------------

const keyOf = (binding) => JSON.stringify(binding, Object.keys(binding).sort());

async function openFill(args) {
  let session;
  try {
    session = await call("fill_open", {
      args: { templates: [], profiles: {}, name: "", values: {}, paths: {}, fill_set_id: null, request_id: null, ...args },
    });
  } catch {
    if (args.request_id) await invoke("cancel_request", { id: args.request_id }).catch(() => {});
    return;
  }
  const standalone = !args.request_id;
  if (standalone) await invoke("set_busy", { busy: true });
  try {
    await new FillForm(session).run();
  } finally {
    if (standalone) await invoke("set_busy", { busy: false });
    await refreshAfterRequests();
  }
}

class FillForm {
  constructor(session) {
    this.s = session;
    this.bindings = { ...session.bindings };
    this.profiles = { ...session.profiles };
    this.fillSetId = session.fill_set_id;
    this.saved = false;
    this.active = session.vars[0]?.name || null;
    this.evaluation = { sources: {}, rows: {}, empty: [] };
    this.previews = {};
    this.tab = session.templates[0]?.id;
    this.previewSeq = 0;
    this.evalSeq = 0;
  }

  templateIds() {
    return this.s.templates.map((t) => t.id);
  }

  run() {
    return new Promise((resolve) => this.show(resolve));
  }

  show(resolve) {
    const s = this.s;
    const count = s.templates.length;
    const names = s.templates.map((t) => t.name).join(" · ");
    const title = s.request_id ? `Claude wants to fill ${plural(count, "document")}` : "Fill documents";
    const explanation = `${names}. Check each blank and pick where its value comes from.`;
    const roleRows = Object.entries(s.roles)
      .map(
        ([role, description]) => `<label title="${esc(description)}">${esc(role)}</label>
        <select data-role="${esc(role)}"><option value="">Choose a profile…</option>${s.all_profiles
          .map(
            (p) => `<option value="${p.id}" ${this.profiles[role] === p.id ? "selected" : ""}>${esc(p.name)}${p.kind ? `  (${esc(p.kind)})` : ""}</option>`
          )
          .join("")}</select>`
      )
      .join("");
    const sheet = showSheet(`
      <div class="sheet-head">${sheetHead("doc", title, s.reason ? `${s.reason}. ${explanation}` : explanation)}
        <div class="sheet-form"><label>Fill set</label><input type="text" id="fill-name" value="${esc(s.name)}" spellcheck="false">${roleRows}</div>
      </div>
      <div class="sheet-summary" id="fill-summary"></div>
      <div class="sheet-body"><div class="fill-split">
        <div class="fill-rows" id="fill-rows"></div>
        <div class="fill-preview"><div class="tabs ${count > 1 ? "" : "hidden"}" id="fill-tabs">${s.templates
          .map((t) => `<div class="tab" data-tab="${esc(t.id)}">${esc(t.name)}</div>`)
          .join("")}</div><div class="preview-area" id="fill-preview"><div class="preview-error">Loading preview…</div></div></div>
      </div></div>
      <div class="sheet-foot"><span class="note">Values stay in Blankey. Claude only learns which source each field uses.</span>
        <button data-cancel>Cancel</button><button data-save>Save</button><button class="primary" data-generate>Save and Generate</button></div>`, resolve);
    $$("[data-role]", sheet).forEach((select) => {
      select.onchange = () => {
        if (select.value) this.profiles[select.dataset.role] = Number(select.value);
        else delete this.profiles[select.dataset.role];
        this.refresh();
      };
    });
    $$("[data-tab]", sheet).forEach((el) => (el.onclick = () => this.showTab(el.dataset.tab, false)));
    this.renderRows();
    this.refresh();
    sheet.querySelector("[data-cancel]").onclick = () => this.cancel();
    sheet.querySelector("[data-save]").onclick = () => this.save(false);
    sheet.querySelector("[data-generate]").onclick = () => this.save(true);
    state.sheetKeys = (e) => this.onKey(e);
    document.addEventListener("keydown", state.sheetKeys);
  }

  rolesReady() {
    return Object.keys(this.s.roles).every((role) => this.profiles[role]);
  }

  async refresh() {
    const seq = ++this.evalSeq;
    const args = { templates: this.templateIds(), profiles: this.profiles, bindings: this.bindings };
    try {
      const evaluation = await invoke("fill_evaluate", { args });
      if (seq !== this.evalSeq) return;
      this.evaluation = evaluation;
    } catch (e) {
      toast(String(e), 5000);
    }
    this.renderRows();
    this.schedulePreview();
  }

  schedulePreview() {
    clearTimeout(this.previewTimer);
    this.previewTimer = setTimeout(() => this.loadPreview(), 250);
  }

  async loadPreview() {
    const seq = ++this.previewSeq;
    try {
      const previews = await invoke("fill_preview", {
        args: { templates: this.templateIds(), profiles: this.profiles, bindings: this.bindings },
      });
      if (seq !== this.previewSeq) return;
      this.previews = previews;
      this.showTab(this.tab, false);
    } catch (e) {
      $("#fill-preview").innerHTML = `<div class="preview-error">No preview: ${esc(e)}</div>`;
    }
  }

  showTab(id, scroll) {
    this.tab = id;
    $$("[data-tab]").forEach((el) => el.classList.toggle("active", el.dataset.tab === id));
    const container = $("#fill-preview");
    const keep = container.scrollTop;
    renderPreview(container, this.previews[id], { onPick: (name) => this.select(name, false) });
    if (!scroll) container.scrollTop = keep;
    if (this.active) highlightPreview(container, this.active, scroll);
  }

  /** Shows the variable in the current document if it is there, else in the first one that has it. */
  reveal(name) {
    const has = (id) => this.previews[id]?.names?.includes(name);
    if (!has(this.tab)) {
      const other = this.templateIds().find(has);
      if (other) return this.showTab(other, true);
    }
    highlightPreview($("#fill-preview"), name, true);
  }

  describe(v) {
    const binding = this.bindings[v.name] || { default: true };
    const row = this.evaluation.rows[v.name] || { value: "", empty: true };
    let kind, path;
    if ("path" in binding) [kind, path] = ["Vault", binding.path];
    else if ("value" in binding) [kind, path] = [binding.value === this.s.claude_values[v.name] ? "Claude" : "Value", ""];
    else {
      const expressions = [...new Set(Object.values(v.defaults))];
      kind = "Default";
      path = (expressions.length === 1 ? expressions[0] : "per template").replace(/\{\{|\}\}/g, "").trim();
    }
    if (!row.empty) return { kind, path, text: row.value, empty: false };
    if (kind === "Vault") return { kind, path, text: "(empty in profile)", empty: true };
    return { kind: "Empty", path, text: "Choose a source…", empty: true };
  }

  renderRows() {
    const box = $("#fill-rows");
    if (!box) return;
    const keep = box.scrollTop;
    box.innerHTML = this.s.vars
      .map((v) => {
        const d = this.describe(v);
        const tip = v.contexts.map(([t, c]) => `${t}: ${c}`).join("\n\n");
        return `<div class="fill-row ${d.empty ? "is-empty" : ""} ${v.name === this.active ? "active" : ""}" data-var="${esc(v.name)}">
          <div class="fill-field" title="${esc(tip)}"><div class="label">${esc(v.label || v.name)}</div><div class="var">${esc(v.name)}</div></div>
          <div class="source" tabindex="0"><span class="chip ${d.kind}">${d.kind}</span>${
            d.path ? `<span class="path">${esc(d.path)}</span>` : ""
          }<span class="val">${esc(d.text)}</span><span class="caret">⌄</span></div></div>`;
      })
      .join("");
    box.scrollTop = keep;
    $$(".fill-row", box).forEach((row) => {
      const name = row.dataset.var;
      row.querySelector(".fill-field").onclick = () => this.select(name, true);
      const source = row.querySelector(".source");
      source.onclick = () => {
        this.select(name, true);
        this.sourceMenu(name, source);
      };
    });
    const total = this.s.vars.length;
    const empty = this.evaluation.empty.length;
    const missing = Object.keys(this.s.roles).filter((r) => !this.profiles[r]);
    let summary = empty ? `${plural(total, "field")} · ${empty} empty` : `${plural(total, "field")} · all filled`;
    if (missing.length) summary += ` · choose profiles for: ${missing.join(", ")}`;
    $("#fill-summary").textContent = summary;
  }

  select(name, fromList) {
    this.active = name;
    $$(".fill-row").forEach((row) => row.classList.toggle("active", row.dataset.var === name));
    if (!fromList) $(`.fill-row[data-var="${CSS.escape(name)}"]`)?.scrollIntoView({ block: "nearest" });
    this.reveal(name);
  }

  setBinding(name, binding) {
    this.bindings[name] = binding;
    this.refresh();
  }

  sourceMenu(name, anchor) {
    const v = this.s.vars.find((x) => x.name === name);
    const current = keyOf(this.bindings[name] || { default: true });
    const option = (label, binding) => ({ label, checked: keyOf(binding) === current, action: () => this.setBinding(name, binding) });
    const items = [{ label: "Type a value…", action: () => this.startTyping(name) }, "-"];
    const first = [];
    const expressions = [...new Set(Object.values(v.defaults))];
    if (expressions.length) {
      const expression = expressions.length === 1 ? expressions[0] : "per template";
      const value = this.evaluation.rows[name]?.default_value ?? "";
      const shown = value || "(empty)";
      first.push(option(expression ? `Template default  ${expression}  —  ${shown}` : `Template default  —  ${shown}`, { default: true }));
    }
    const claude = this.s.claude_values[name];
    const typed = this.bindings[name]?.value;
    if (claude) first.push(option(`Claude  —  ${claude}`, { value: claude }));
    if (typed && typed !== claude) first.push(option(`Typed  —  ${typed}`, { value: typed }));
    items.push(...first);
    for (const [role, entries] of Object.entries(this.evaluation.sources)) {
      if (!entries.length) continue;
      if (items[items.length - 1] !== "-") items.push("-");
      items.push({ head: role });
      for (const entry of entries) items.push(option(`${entry.path}  —  ${entry.value || "(empty)"}`, { path: entry.path }));
    }
    if (!Object.keys(this.evaluation.sources).length && Object.keys(this.s.roles).length) {
      items.push("-", { head: "Choose profiles above to pick vault fields" });
    }
    const rect = anchor.getBoundingClientRect();
    popup(rect.left, rect.bottom + 2, items, rect.width);
  }

  startTyping(name) {
    const row = $(`.fill-row[data-var="${CSS.escape(name)}"]`);
    const source = row.querySelector(".source");
    const current = this.bindings[name]?.value ?? "";
    source.innerHTML = `<input type="text" value="${esc(current)}" placeholder="Type a value" spellcheck="false">`;
    source.onclick = null;
    const input = source.querySelector("input");
    input.focus();
    input.select();
    let done = false;
    const finish = (commit) => {
      if (done) return;
      done = true;
      if (commit) this.setBinding(name, { value: input.value.trim() });
      else this.renderRows();
    };
    input.onkeydown = (e) => {
      e.stopPropagation();
      if (e.key === "Enter") finish(true);
      if (e.key === "Escape") finish(false);
    };
    input.onblur = () => finish(true);
  }

  onKey(e) {
    if (!$("#modal").classList.contains("hidden")) return;
    if (!$("#menu").classList.contains("hidden")) {
      if (e.key === "Escape") hidePopup();
      return;
    }
    if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
    const names = this.s.vars.map((v) => v.name);
    const index = names.indexOf(this.active);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = names[Math.max(0, Math.min(names.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) this.select(next, false);
    } else if ((e.key === "Enter" || e.key === " ") && this.active) {
      e.preventDefault();
      const source = $(`.fill-row[data-var="${CSS.escape(this.active)}"] .source`);
      if (source) this.sourceMenu(this.active, source);
    } else if (e.key === "Escape") {
      this.cancel();
    }
  }

  async save(generate) {
    const name = $("#fill-name").value.trim();
    const buttons = $$("#sheet .sheet-foot button");
    buttons.forEach((b) => (b.disabled = true));
    document.body.classList.add("busy");
    try {
      const saved = await invoke("fill_save", {
        args: {
          request_id: this.s.request_id,
          fill_set_id: this.fillSetId,
          name,
          templates: this.templateIds(),
          profiles: this.profiles,
          bindings: this.bindings,
          generate,
        },
      });
      this.fillSetId = saved.fill_set_id;
      this.saved = true;
      if (generate) {
        toast(`${plural(saved.document_ids.length, "document")} generated`);
        this.close();
      } else toast("Fill set saved");
    } catch (e) {
      await alertDialog("Blankey", String(e));
    } finally {
      buttons.forEach((b) => (b.disabled = false));
      document.body.classList.remove("busy");
    }
  }

  async cancel() {
    if (this.s.request_id) {
      await invoke("fill_close", {
        args: {
          request_id: this.s.request_id,
          fill_set_id: this.saved ? this.fillSetId : null,
          templates: this.templateIds(),
          profiles: this.profiles,
          bindings: this.bindings,
        },
      }).catch((e) => toast(String(e)));
    }
    this.close();
  }

  close() {
    clearTimeout(this.previewTimer);
    hidePopup();
    closeSheet();
  }
}

// -- boot ---------------------------------------------------------------------------------------

$("#search").oninput = (e) => {
  state.search = e.target.value;
  const page = PAGES[state.page];
  if (page.filter) page.filter();
  else rerender();
};
$("#lock-btn").onclick = () => invoke("lock");

listen("vault-locked", async () => {
  showGate.triedQuick = false;
  closeSheet();
  hidePopup();
  $("#modal").classList.add("hidden");
  await refreshStatus();
});
listen("vault-unlocked", () => enterApp());
listen("request-created", async () => {
  await refreshStatus();
  if (state.status.unlocked) {
    renderNav();
    if (state.page === "activity") rerender();
    processRequests();
  } else {
    state.gateReason = "Claude sent a request. Unlock the vault to continue.";
    showGate("unlock");
  }
});
listen("requests-changed", () => refreshStatus().then(renderNav));

(async () => {
  const status = await refreshStatus();
  if (status.unlocked) enterApp();
})();
