import { readFileSync } from "node:fs";

// A fake Tauri backend: the commands the UI invokes, over an in-memory vault. Tests read `calls` and change `db`.

const PAGE_IMAGE = "data:image/png;base64,iVBORw0KGgo=";

export const pagesPreview = {
  type: "pages",
  names: ["name", "egn"],
  pages: [{ image: PAGE_IMAGE, width: 612, height: 792, boxes: { name: [[150, 670, 400, 700]], egn: [[150, 570, 300, 600]] } }],
};

export const htmlPreview = {
  type: "html",
  names: ["who", "amount"],
  html: '<p>Between <span class="blank" data-var="who">who</span> and the bank: <span class="blank empty" data-var="amount">amount</span></p>',
};

export const reviewText = "Tenant Jane Example, card 77-ABC, rent 30 days. Signed: Jane Example";
// what detection finds: one name, at both its places
const detected = [{ id: "pii:1", value: "Jane Example" }];

export function seed() {
  return {
    initialized: true,
    unlocked: true,
    keyring: true,
    password: "pw",
    profiles: [
      { id: 1, name: "John", kind: "person" },
      { id: 2, name: "Acme", kind: "company" },
    ],
    fields: {
      1: [
        { key: "name", label: "Full name", type: "text", value: "John Watson" },
        { key: "egn", label: "EGN", type: "egn", value: "" },
      ],
      2: [],
    },
    sets: [{ id: 1, name: "acme", templates: ["nzok"], profiles: ["John"], updated_at: "2026-10-10T10:00:00" }],
    docs: [
      { id: 1, title: "NZOK (acme)", template_id: "nzok", filename: "nzok.pdf", pages: 2, created_at: "2026-10-10T10:00:00" },
      { id: 2, title: "Loan (acme)", template_id: "loan", filename: "loan.docx", pages: 0, created_at: "2026-10-10T11:00:00" },
    ],
    templates: [
      { id: "nzok", name: "NZOK request", kind: "pdf_form", blanks: 2, used_by: ["acme"], description: "Data subject request" },
      { id: "loan", name: "Loan contract", kind: "docx", blanks: 2, used_by: [], description: "" },
    ],
    roots: [{ id: 1, path: "/Users/test/Forms", files: 2, added_at: "2026-10-18T09:00:00+00:00" }],
    libraryFiles: [
      { id: 4, root_id: 1, rel_path: "Leases/Lease agreement.docx", kind: "docx", title: "", category: "lease", tags: ["leases"], readiness: "blanks", fields: 0, blanks: 6, pages: null, missing: false },
      { id: 5, root_id: 1, rel_path: "Application.pdf", kind: "pdf", title: "Application for a permit", category: null, tags: [], readiness: "ready", fields: 4, blanks: 0, pages: 1, missing: false },
    ],
    requests: [],
    links: [
      {
        link_id: "XchaBj2",
        url: "https://relay.blankey.io/s/XchaBj2#k4Fp9QzR2mWx7aLc",
        title: "Tenant details",
        profile_id: 1,
        new_profile: null,
        created_at: "2026-10-17T09:00:00+00:00",
        expires_at: "2026-10-24T09:00:00Z",
        max_submissions: 3,
        state: "open",
        submissions: 2,
        pending: 1,
        last_received: "2026-10-17T12:00:00+00:00",
        fields: ["Full name", "Email"],
        profile_name: "John",
      },
    ],
    linkSubmissions: [
      { sid: "s-2", link_id: "XchaBj2", request_id: 12, size: 4096, received_at: "2026-10-17T12:00:00+00:00", status: "pending", handled_at: null, profile_id: null, profile_name: null },
      { sid: "s-1", link_id: "XchaBj2", request_id: 11, size: 2048, received_at: "2026-10-16T10:00:00+00:00", status: "saved", handled_at: "2026-10-16T10:05:00+00:00", profile_id: 1, profile_name: "John" },
    ],
    collected: [],
    submissions: {},
    previewDelay: 0,
    telemetry: true,
    update: { current: "0.2.5", auto: true, state: "idle" },
    relay: {
      url: "https://relay.blankey.io",
      official: true,
      signed_in: true,
      account: "you@example.com",
      state: "connected",
      message: "",
      account_url: "https://relay.blankey.io/account",
      mcp_url: "https://relay.blankey.io/mcp",
      grants: [
        { grant_id: "g-1", relay_url: "https://relay.blankey.io", client_name: "Claude", redirect_host: "claude.ai", accepted_at: "2026-10-11T09:30:00+00:00", on_relay: true },
      ],
    },
  };
}

export const fillRequest = (id = 7) => ({
  id,
  kind: "fill",
  payload: { reason: "Fill the NZOK form", templates: ["nzok", "loan"], profiles: { applicant: 1 } },
});

export const profileInputRequest = (id = 8) => ({
  id,
  kind: "profile_input",
  payload: {
    reason: "Need contact details",
    profile_id: 1,
    fields: [
      { key: "email", label: "Email", type: "email", required: true },
      { key: "vip", label: "VIP", type: "bool", hint: "Priority customer" },
    ],
  },
});

export const submissionRequest = (id = 12) => ({ id, kind: "link_submission", payload: { link_id: "XchaBj2", sid: "s-1", reason: "Submission for Tenant details" } });

export const submission = (id = 12, sid = "s-2", status = "pending") => ({
  sid,
  request_id: id,
  status,
  received_at: "2026-10-17T12:00:00+00:00",
  handled_at: status === "saved" ? "2026-10-17T12:05:00+00:00" : null,
  profile_id: status === "saved" ? 1 : null,
  link: { link_id: "XchaBj2", title: "Tenant details", profile_id: null, new_profile: "New tenant" },
  fields: [
    { key: "name", label: "Full name", type: "text", hint: "", required: true },
    { key: "pets", label: "Pets", type: "bool", hint: "", required: false },
  ],
  values: { name: "Jane Roe", pets: "yes" },
  files: [
    { index: 0, name: "id-card.png", type: "image/png", size: 2048, thumbnail: "data:image/png;base64,iVBORw0KGgo=" },
    { index: 1, name: "payslip.pdf", type: "application/pdf", size: 3 * 1024 * 1024, thumbnail: null },
  ],
  submitted_at: "2026-10-17T12:00:00.000Z",
});

export const linkRequest = (id = 14) => ({
  id,
  kind: "collection_link",
  payload: {
    reason: "Collect the tenant's contact details",
    title: "Contact details",
    profile_id: 1,
    fields: [
      { key: "name", label: "Name", type: "text", required: true },
      { key: "phone", label: "Phone", type: "text" },
    ],
    max_files: 0,
    expires_days: 30,
    max_submissions: 1,
  },
});

export const fileRequest = (id = 10) => ({ id, kind: "file", payload: { kind: "pdf", reason: "The blank NZOK form" } });
export const libraryRequest = (id = 13) => ({
  id,
  kind: "file",
  payload: { kind: "docx", reason: "Make a lease template", source: "lib:4", path: "Leases/Lease agreement.docx" },
});

const fillSession = (db, args) => ({
  request_id: args.request_id,
  reason: args.request_id ? "Fill the NZOK form" : "",
  fill_set_id: args.fill_set_id,
  name: "acme",
  templates: [
    { id: "nzok", name: "NZOK request", kind: "pdf_form" },
    { id: "loan", name: "Loan contract", kind: "docx" },
  ],
  roles: { applicant: "The person" },
  profiles: { applicant: 1 },
  all_profiles: db.profiles,
  vars: [
    { name: "name", label: "Name", defaults: { nzok: "{{ applicant.name }}" }, contexts: [["nzok", "Name: ___"]] },
    { name: "egn", label: "EGN", defaults: { nzok: "{{ applicant.egn }}" }, contexts: [] },
    { name: "who", label: "Borrower", defaults: { loan: "{{ applicant.name }}" }, contexts: [] },
    { name: "amount", label: "Amount", defaults: {}, contexts: [] },
  ],
  bindings: Object.fromEntries(Object.keys(args.ask || {}).map((name) => [name, { value: "" }])),
  suggested_values: { amount: "34 000" },
  ask: args.ask || {},
});

function evaluate({ bindings, profiles }) {
  const value = (name, fallback) => {
    const b = bindings[name];
    if (b?.path) return b.profile ? `from ${b.profile}:${b.path}` : `from ${b.path}`;
    return b?.value ?? fallback;
  };
  const row = (name, fallback) => {
    const v = value(name, fallback);
    return { value: v, empty: !v, default_value: fallback };
  };
  const rows = { name: row("name", "John Watson"), egn: row("egn", ""), who: row("who", "John Watson"), amount: row("amount", "") };
  const sources = profiles.applicant
    ? { applicant: [{ path: "applicant.name", value: "John Watson" }, { path: "applicant.egn", value: "" }] }
    : {};
  for (const b of Object.values(bindings)) {
    if (b?.profile) sources[`profile:${b.profile}`] = [{ path: b.path, value: `from ${b.profile}:${b.path}`, profile: b.profile }];
  }
  const readsRole = Object.keys(rows).some((k) => !bindings[k] || bindings[k].default || (bindings[k].path && !bindings[k].profile));
  return { sources, rows, empty: Object.keys(rows).filter((k) => rows[k].empty), used_roles: readsRole ? ["applicant"] : [] };
}

export function createBackend(db = seed()) {
  const calls = [];
  const listeners = {};
  const emit = (name, payload) => (listeners[name] || []).forEach((listener) => listener({ payload }));
  const removeRequest = (id) => (db.requests = db.requests.filter((r) => r.id !== id));

  const handlers = {
    status: () => ({
      initialized: db.initialized,
      unlocked: db.unlocked,
      pending: db.requests.length,
      keyring_enabled: db.keyring,
      biometric: true,
      data_dir: "/data",
      mcp_url: "http://127.0.0.1:8765/mcp",
      mcp_running: true,
      field_types: ["text", "date", "egn", "email", "bool"],
      language: db.language || "en",
      languages: { en: "English", bg: "Български" },
    }),
    setup: ({ password }) => {
      db.initialized = true;
      db.unlocked = true;
      db.password = password;
      return "ABCD-EFGH-IJKL";
    },
    unlock: ({ password }) => {
      if (password !== db.password) throw "Wrong password or key";
      db.unlocked = true;
    },
    unlock_quick: () => false,
    unlock_recovery: () => {
      db.unlocked = true;
    },
    lock: () => {
      db.unlocked = false;
      emit("vault-locked");
    },
    touch: () => null,
    set_busy: () => null,
    copy_text: () => null,
    list_activity: () => [
      ...db.requests.map((r) => ({ id: r.id, label: "Fill", details: "NZOK", outcome: "waiting", status: "pending", time: "2026-10-10T10:00", kind: r.kind, document_ids: [], fill_set_id: null })),
      { id: 6, label: "Fill", details: "acme", outcome: "generated", status: "done", time: "2026-10-09T10:00", kind: "fill", document_ids: [1], fill_set_id: 1 },
    ],
    list_fill_sets: () => db.sets,
    generate_fill_set: () => [5, 6],
    export_fill_set: () => "/tmp/acme.yaml",
    import_fill_set: () => null,
    delete_fill_set: ({ id }) => {
      db.sets = db.sets.filter((s) => s.id !== id);
    },
    list_documents: () => db.docs,
    list_collected_files: () => db.collected,
    open_collected_files: () => null,
    export_collected_file: () => "/tmp/payslip.pdf",
    delete_collected_files: ({ ids }) => {
      db.collected = db.collected.filter((f) => !ids.includes(f.id));
    },
    list_links: () => db.links,
    create_link: ({ draft, requestId }) => {
      if (!draft.title.trim()) throw "Give the link a title";
      const link = { link_id: "Ab3dE9z", url: "https://relay.blankey.io/s/Ab3dE9z#qqj_9wO1CyKX9Pbh", title: draft.title };
      db.links.unshift({ ...link, state: "open", submissions: 0, max_submissions: draft.max_submissions, fields: draft.fields.map((f) => f.label), expires_at: "2026-10-24T09:00:00Z" });
      if (requestId) removeRequest(requestId);
      return link;
    },
    delete_link: ({ linkId }) => {
      db.links = db.links.filter((l) => l.link_id !== linkId);
    },
    revoke_link: ({ linkId }) => {
      db.links.find((l) => l.link_id === linkId).state = "revoked";
    },
    list_submissions: ({ linkId }) => db.linkSubmissions.filter((s) => s.link_id === linkId),
    open_submission: ({ id, sid }) => {
      const found = sid ? db.submissions[sid] : Object.values(db.submissions).find((s) => s.request_id === id);
      if (!found) throw "This submission could not be opened, so it was dropped.";
      return found;
    },
    resolve_submission: ({ sid, values }) => {
      if (!values.name) return [{ label: "Full name", message: "required" }];
      removeRequest(db.submissions[sid].request_id);
      return [];
    },
    discard_submission: ({ sid }) => {
      removeRequest(db.submissions[sid].request_id);
    },
    open_submission_file: () => null,
    export_submission_file: () => "/tmp/id-card.png",
    open_documents: () => null,
    export_document: () => "/tmp/doc.pdf",
    delete_documents: ({ ids }) => {
      db.docs = db.docs.filter((d) => !ids.includes(d.id));
    },
    list_templates: () => db.templates,
    library_roots: () => db.roots,
    library_files: () => db.libraryFiles.filter((f) => db.roots.some((r) => r.id === f.root_id)),
    add_library_root: () => {
      db.roots.push({ id: 2, path: "/Users/test/Models", files: 0, added_at: "2026-10-20T09:00:00+00:00" });
      return { id: 2, report: { added: 0, updated: 0, moved: 0, missing: 0 } };
    },
    remove_library_root: ({ id }) => {
      db.roots = db.roots.filter((r) => r.id !== id);
    },
    rescan_library: () => null,
    show_library_file: () => null,
    open_library_file: () => null,
    library_preview: ({ id }) => (id === 5 ? pagesPreview : htmlPreview),
    pii_review: ({ source }) => review(db, source),
    pii_correct: ({ source, correction }) => {
      const state = piiState(db, source);
      const { action, texts = [], entities = [], start, end } = correction;
      const values = [...texts, ...entities.map((id) => [...detected, ...userHidden(state)].find((m) => m.id === id).value)];
      if (start !== undefined) values.push(reviewText.slice(start, end));
      for (const value of values) {
        if (action === "show") {
          state.hide = state.hide.filter((v) => v !== value);
          if (!state.show.includes(value)) state.show.push(value);
        } else if (action === "hide") {
          state.show = state.show.filter((v) => v !== value);
          state.ignore = state.ignore.filter((v) => v !== value);
          if (!detected.some((d) => d.value === value) && !state.hide.includes(value)) state.hide.push(value);
        } else if (action === "forget" || action === "remove") {
          const added = state.hide.includes(value);
          state.hide = state.hide.filter((v) => v !== value);
          state.show = state.show.filter((v) => v !== value);
          if (action === "remove" && !added) state.ignore.push(value);
        }
      }
      return review(db, source);
    },
    show_template_folder: () => null,
    delete_template: ({ id }) => {
      db.templates = db.templates.filter((t) => t.id !== id);
    },
    template_preview: ({ id }) =>
      delayed(db.previewDelay, {
        roles: { applicant: "person" },
        fields: { name: "{{ applicant.name }}", egn: "" },
        labels: { name: "Name" },
        examples: ["ex1"],
        blanks: 2,
        preview: id === "loan" ? htmlPreview : pagesPreview,
      }),
    list_profiles: () => db.profiles,
    profile_fields: ({ id }) => db.fields[id],
    save_fields: ({ id, fields }) => {
      const problems = fields.filter((f) => f.key.includes(" ")).map((f) => ({ label: f.key, message: "use dotted words without spaces" }));
      if (!problems.length) db.fields[id] = fields;
      return problems;
    },
    create_profile: ({ name, kind }) => {
      const id = Math.max(...db.profiles.map((p) => p.id)) + 1;
      db.profiles.push({ id, name, kind });
      db.fields[id] = [];
      return id;
    },
    update_profile: ({ id, name, kind }) => Object.assign(db.profiles.find((p) => p.id === id), { name, kind }),
    delete_profile: ({ id }) => {
      db.profiles = db.profiles.filter((p) => p.id !== id);
    },
    next_request: () => {
      if (!db.unlocked) throw "Vault is locked";
      const request = db.requests[0];
      return request ? { request, profile_name: "John", existing: { email: "old@example.com" } } : null;
    },
    cancel_request: ({ id }) => removeRequest(id),
    resolve_profile_input: ({ id, values }) => {
      if (!values.email) return [{ label: "Email", message: "required" }];
      removeRequest(id);
      return [];
    },
    pick_request_file: () => {
      if (db.pickedFile === null) return null;
      const name = db.pickedFile || "form.pdf";
      return { path: `/Users/test/${name}`, name };
    },
    share_upload: ({ id }) => {
      removeRequest(id);
      return { outcome: "uploaded", file_id: "file:0f8fad5b-d9cb-469f-a165-70867728950e", kind: "pdf", size: 1024 };
    },
    share_library_request: ({ id }) => {
      removeRequest(id);
      return { outcome: "shared", file_id: "lib:4", kind: "docx", size: 2048 };
    },
    fill_open: ({ args }) => fillSession(db, args),
    fill_evaluate: ({ args }) => evaluate(args),
    fill_preview: () => ({ nzok: pagesPreview, loan: htmlPreview }),
    fill_profile_entries: ({ profileId }) => [{ path: "name", value: `profile ${profileId} name`, profile: profileId }],
    fill_save: ({ args }) => {
      if (!args.name) throw "Enter a fill set name";
      if (args.request_id) removeRequest(args.request_id);
      return { fill_set_id: 9, document_ids: args.generate ? [3, 4] : [] };
    },
    fill_close: ({ args }) => removeRequest(args.request_id),
    catalog: ({ language }) => (language === "en" ? {} : JSON.parse(readFileSync(new URL(`../i18n/${language}.json`, import.meta.url), "utf8"))),
    focused: () => null,
    log_error: () => null,
    telemetry_status: () => db.telemetry,
    telemetry_set: ({ enabled }) => {
      db.telemetry = enabled;
    },
    update_status: () => db.update,
    update_check: () => {
      db.update = { ...db.update, state: "ready", version: "0.3.0" };
      return db.update;
    },
    update_install: () => null,
    update_auto_set: ({ enabled }) => {
      db.update.auto = enabled;
    },
    relay_status: () => db.relay,
    relay_summary: () => ({ signed_in: db.relay.signed_in, state: db.relay.state, account: db.relay.account }),
    relay_login: () => {
      db.relay.state = "signing_in";
    },
    relay_logout: () => {
      Object.assign(db.relay, { signed_in: false, state: "signed_out", account: null });
    },
    relay_revoke: ({ grantId }) => {
      db.relay.grants = db.relay.grants.filter((g) => g.grant_id !== grantId);
    },
    relay_set_url: ({ url }) => {
      const next = url ?? "https://relay.blankey.io";
      if (!next.startsWith("https://")) throw `${next} must use https://`;
      Object.assign(db.relay, { url: next, official: !url, signed_in: false, state: "signing_in", account: null, grants: [] });
    },
    relay_open_account: () => null,
  };

  const tauri = {
    core: {
      invoke: async (cmd, args = {}) => {
        calls.push([cmd, args]);
        const handler = handlers[cmd];
        if (!handler) throw new Error(`fake backend: no command ${cmd}`);
        return structuredClone(await handler(args));
      },
    },
    event: {
      listen: async (name, listener) => {
        (listeners[name] ||= []).push(listener);
      },
    },
  };
  const called = (cmd) => calls.filter(([c]) => c === cmd).map(([, args]) => args);
  return { db, calls, called, emit, tauri };
}

const delayed = (ms, value) => (ms ? new Promise((resolve) => setTimeout(() => resolve(value), ms)) : value);

const count = (value) => reviewText.split(value).length - 1;
const userHidden = (state) => state.hide.map((value, i) => ({ id: `pii:${i + 2}`, value }));

function piiState(db, source) {
  db.pii ??= {};
  db.pii[source] ??= { hide: [], show: [], ignore: [] };
  return db.pii[source];
}

/** Marks every place of the members in the text, numbered per member, as the backend renders the preview. */
function reviewHtml(members) {
  const marks = [];
  for (const m of members) {
    const at = [];
    for (let i = reviewText.indexOf(m.value); i >= 0; i = reviewText.indexOf(m.value, i + 1)) at.push(i);
    at.forEach((start, n) => {
      if (marks.some((x) => x.start === start)) return;
      const attrs = m.id.startsWith("shown:") ? `class="pii shown" data-v="${m.id.slice(6)}"` : `class="pii" data-e="${m.id}"`;
      marks.push({ start, end: start + m.value.length, html: `<mark ${attrs} data-n="${n}">${m.value}</mark>` });
    });
  }
  marks.sort((a, b) => a.start - b.start);
  let html = "";
  let at = 0;
  for (const m of marks) {
    html += reviewText.slice(at, m.start) + m.html;
    at = m.end;
  }
  return `<p><span data-b="0" data-o="0">${html + reviewText.slice(at)}</span></p>`;
}

/** One row per value in document order, with its members, as the backend sends them. */
function review(db, source) {
  const state = piiState(db, source);
  const members = [
    ...detected.filter((d) => !state.show.includes(d.value) && !state.ignore.includes(d.value)).map((d) => ({ ...d, occurrences: count(d.value), hidden: true, user: false })),
    ...userHidden(state).map((m) => ({ ...m, occurrences: count(m.value), hidden: true, user: true })),
    ...state.show.map((value, i) => {
      const found = detected.find((d) => d.value === value);
      return { id: `shown:${i}`, value, occurrences: count(value), hidden: false, user: !found };
    }),
  ];
  const rows = new Map();
  for (const m of [...members].sort((a, b) => reviewText.indexOf(a.value) - reviewText.indexOf(b.value))) {
    const key = m.value.toLowerCase();
    rows.set(key, [...(rows.get(key) || []), m]);
  }
  const items = [...rows].map(([key, ms]) => ({
    key,
    value: ms[0].value,
    hidden: ms.some((m) => m.hidden),
    user: ms.some((m) => m.user),
    occurrences: ms.reduce((n, m) => n + m.occurrences, 0),
    ids: ms.map((m) => m.id),
    members: ms,
  }));
  return { preview: { type: "html", names: [], html: reviewHtml(members) }, items };
}
