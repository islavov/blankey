import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call, invoke } from "../api.js";
import { html, plural } from "../lib.js";
import { Preview } from "../preview.js";
import { refreshAfterRequests } from "../requests.js";
import { alertDialog, isDialogOpen, toast } from "../ui/feedback.js";
import { hidePopup, isMenuOpen, popup } from "../ui/menu.js";
import { Select } from "../ui/select.js";
import { openSheet, SheetHead } from "../ui/sheet.js";
import { FillRow } from "./rows.js";
import { sourceItems } from "./source.js";

const PREVIEW_DELAY = 250;

/** Opens the fill form for templates, a saved fill set or a request from the assistant; resolves when it closes. */
export async function openFill(args) {
  let session;
  try {
    session = await call("fill_open", {
      args: { templates: [], profiles: {}, name: "", values: {}, paths: {}, fill_set_id: null, request_id: null, ...args },
    });
  } catch {
    if (args.request_id) await invoke("cancel_request", { id: args.request_id }).catch(() => {});
    return;
  }
  // a request already holds off auto-lock while it is processed
  const standalone = !args.request_id;
  if (standalone) await invoke("set_busy", { busy: true });
  try {
    await openSheet((close) => html`<${FillForm} session=${session} close=${close} />`);
  } finally {
    if (standalone) await invoke("set_busy", { busy: false });
    await refreshAfterRequests();
  }
}

function FillForm({ session: s, close }) {
  const templateIds = s.templates.map((t) => t.id);
  const [name, setName] = useState(s.name);
  const [profiles, setProfiles] = useState({ ...s.profiles });
  const [bindings, setBindings] = useState({ ...s.bindings });
  const [fillSetId, setFillSetId] = useState(s.fill_set_id);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [active, setActive] = useState(s.vars[0]?.name || null);
  const [typing, setTyping] = useState(null);
  const [evaluation, setEvaluation] = useState({ sources: {}, rows: {}, empty: [] });
  const [previews, setPreviews] = useState(null);
  const [previewError, setPreviewError] = useState("");
  const [tab, setTab] = useState(templateIds[0]);
  const [revealSeq, setRevealSeq] = useState(0);
  const args = { templates: templateIds, profiles, bindings };

  useEffect(() => {
    let live = true;
    invoke("fill_evaluate", { args })
      .then((result) => live && setEvaluation(result))
      .catch((e) => toast(String(e), 5000));
    return () => {
      live = false;
    };
  }, [profiles, bindings]);

  useEffect(() => {
    let live = true;
    const timer = setTimeout(() => {
      invoke("fill_preview", { args })
        .then((result) => {
          if (!live) return;
          setPreviews(result);
          setPreviewError("");
        })
        .catch((e) => live && setPreviewError(`No preview: ${e}`));
    }, PREVIEW_DELAY);
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [profiles, bindings]);

  /** Selects a blank and shows it in the preview: in the current document if it is there, else the first that has it. */
  const select = (field) => {
    setActive(field);
    const has = (id) => previews?.[id]?.names?.includes(field);
    if (!has(tab)) {
      const other = templateIds.find(has);
      if (other) setTab(other);
    }
    setRevealSeq((n) => n + 1);
  };

  const bind = (field, binding) => setBindings({ ...bindings, [field]: binding });

  const sourceMenu = (field, anchor) => {
    const v = s.vars.find((x) => x.name === field);
    const items = sourceItems({
      v,
      binding: bindings[field],
      evaluation,
      suggested: s.suggested_values[field],
      hasRoles: Object.keys(s.roles).length > 0,
      choose: (binding) => bind(field, binding),
      type: () => setTyping(field),
    });
    const rect = anchor.getBoundingClientRect();
    popup(rect.left, rect.bottom + 2, items, rect.width);
  };

  const save = async (generate) => {
    setBusy(true);
    document.body.classList.add("busy");
    try {
      const result = await invoke("fill_save", {
        args: { request_id: s.request_id, fill_set_id: fillSetId, name: name.trim(), ...args, generate },
      });
      setFillSetId(result.fill_set_id);
      setSaved(true);
      if (generate) {
        toast(`${plural(result.document_ids.length, "document")} generated`);
        finish();
      } else toast("Fill set saved");
    } catch (e) {
      await alertDialog("Blankey", String(e));
    } finally {
      setBusy(false);
      document.body.classList.remove("busy");
    }
  };

  const cancel = async () => {
    if (s.request_id) {
      await invoke("fill_close", {
        args: { request_id: s.request_id, fill_set_id: saved ? fillSetId : null, ...args },
      }).catch((e) => toast(String(e)));
    }
    finish();
  };

  const finish = () => {
    hidePopup();
    close();
  };

  const onKey = (e) => {
    if (isDialogOpen()) return;
    if (isMenuOpen()) {
      if (e.key === "Escape") hidePopup();
      return;
    }
    if (e.target.tagName === "INPUT" || e.target.tagName === "SELECT") return;
    const names = s.vars.map((v) => v.name);
    const index = names.indexOf(active);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = names[Math.max(0, Math.min(names.length - 1, index + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) select(next);
    } else if ((e.key === "Enter" || e.key === " ") && active) {
      e.preventDefault();
      const source = document.querySelector(`.fill-row[data-var="${CSS.escape(active)}"] .source`);
      if (source) sourceMenu(active, source);
    } else if (e.key === "Escape") {
      cancel();
    }
  };
  useDocumentKeys(onKey);

  const chooseProfile = (role, value) => {
    const next = { ...profiles };
    if (value) next[role] = Number(value);
    else delete next[role];
    setProfiles(next);
  };

  const missing = Object.keys(s.roles).filter((role) => !profiles[role]);
  const total = s.vars.length;
  const empty = evaluation.empty.length;
  let summary = empty ? `${plural(total, "field")} · ${empty} empty` : `${plural(total, "field")} · all filled`;
  if (missing.length) summary += ` · choose profiles for: ${missing.join(", ")}`;

  const title = s.request_id ? `Your AI assistant wants to fill in ${plural(s.templates.length, "document")}` : "Fill documents";
  const explanation = `${s.templates.map((t) => t.name).join(" · ")}. Check each blank and pick where its value comes from.`;

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="doc" title=${title} reason=${s.reason ? `${s.reason}. ${explanation}` : explanation} />
      <div class="sheet-form">
        <label>Fill set</label>
        <input type="text" value=${name} spellcheck="false" onInput=${(e) => setName(e.target.value)} />
        ${Object.entries(s.roles).map(
          ([role, description]) => html`
            <label title=${description}>${role}</label>
            <${Select} value=${profiles[role] ?? ""} onChange=${(value) => chooseProfile(role, value)} options=${[
              ["", "Choose a profile…"],
              ...s.all_profiles.map((p) => [p.id, `${p.name}${p.kind ? `  (${p.kind})` : ""}`]),
            ]} />`
        )}
      </div>
    </div>
    <div class="sheet-summary">${summary}</div>
    <div class="sheet-body">
      <div class="fill-split">
        <div class="fill-rows">
          ${s.vars.map(
            (v) => html`<${FillRow}
              key=${v.name}
              v=${v}
              binding=${bindings[v.name]}
              row=${evaluation.rows[v.name]}
              suggested=${s.suggested_values[v.name]}
              active=${v.name === active}
              typing=${v.name === typing}
              onSelect=${() => select(v.name)}
              onMenu=${(anchor) => {
                select(v.name);
                sourceMenu(v.name, anchor);
              }}
              onTyped=${(value) => {
                setTyping(null);
                if (value !== null) bind(v.name, { value });
              }} />`
          )}
        </div>
        <div class="fill-preview">
          <div class=${`tabs ${s.templates.length > 1 ? "" : "hidden"}`}>
            ${s.templates.map(
              (t) => html`<div class=${`tab ${t.id === tab ? "active" : ""}`} onClick=${() => setTab(t.id)}>${t.name}</div>`
            )}
          </div>
          ${previewError
            ? html`<div class="preview-area"><div class="preview-error">${previewError}</div></div>`
            : html`<${Preview} preview=${previews?.[tab]} placeholder="Loading preview…" active=${active}
                revealSeq=${revealSeq} onPick=${select} />`}
        </div>
      </div>
    </div>
    <div class="sheet-foot">
      <span class="note">Your data stays in Blankey. The AI assistant only learns where each value comes from, never the value.</span>
      <button disabled=${busy} onClick=${cancel}>Cancel</button>
      <button disabled=${busy} onClick=${() => save(false)}>Save</button>
      <button class="primary" disabled=${busy} onClick=${() => save(true)}>Save and Generate</button>
    </div>`;
}

/** A document keydown listener that always calls the latest handler. */
function useDocumentKeys(handler) {
  const latest = useRef(handler);
  latest.current = handler;
  useEffect(() => {
    const listener = (e) => latest.current(e);
    document.addEventListener("keydown", listener);
    return () => document.removeEventListener("keydown", listener);
  }, []);
}
