import { useEffect, useRef, useState } from "../../vendor/preact-htm.js";
import { call, invoke } from "../api.js";
import { t, tn } from "../i18n.js";
import { html } from "../lib.js";
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
      args: {
        templates: [],
        profiles: {},
        name: "",
        values: {},
        paths: {},
        ask: {},
        fill_set_id: null,
        copy_fill_set_id: null,
        request_id: null,
        ...args,
      },
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
  const templateIds = s.templates.map((tpl) => tpl.id);
  const [name, setName] = useState(s.name);
  const [profiles, setProfiles] = useState({ ...s.profiles });
  const [bindings, setBindings] = useState({ ...s.bindings });
  const [fillSetId, setFillSetId] = useState(s.fill_set_id);
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  // blanks the assistant asked the user to type, in form order; the first one still empty opens ready to type
  const asked = s.vars.map((v) => v.name).filter((name) => name in s.ask);
  const nextAsked = (after, current) =>
    asked.find((name, i) => i > asked.indexOf(after) && !current[name]?.value) ?? null;
  const firstAsked = asked.find((name) => !s.bindings[name]?.value) ?? null;
  const [active, setActive] = useState(firstAsked || s.vars[0]?.name || null);
  const [typing, setTyping] = useState(firstAsked);
  const [evaluation, setEvaluation] = useState({ sources: {}, rows: {}, empty: [], used_roles: Object.keys(s.roles) });
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
        .catch((e) => live && setPreviewError(t("No preview: {error}", { error: e })));
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

  const profileName = (id) => s.all_profiles.find((p) => p.id === id)?.name ?? `#${id}`;

  const at = (anchor, items) => {
    const rect = anchor.getBoundingClientRect();
    popup(rect.left, rect.bottom + 2, items, rect.width);
  };

  /** Any profile's fields, not only the ones chosen for the roles: pick the profile, then the field. */
  const otherProfileMenu = (field, anchor) =>
    at(
      anchor,
      s.all_profiles.map((p) => ({
        label: `${p.name}${p.kind ? `  (${p.kind})` : ""}`,
        action: async () => {
          const entries = await invoke("fill_profile_entries", { profileId: p.id }).catch((e) => {
            toast(String(e), 5000);
            return null;
          });
          if (!entries) return;
          const items = entries.length
            ? entries.map((entry) => ({
                label: `${entry.path}  —  ${entry.value || t("(empty)")}`,
                action: () => bind(field, { path: entry.path, profile: p.id }),
              }))
            : [{ head: t("This profile has no fields") }];
          at(anchor, [{ head: p.name }, ...items]);
        },
      }))
    );

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
      otherProfile: s.all_profiles.length ? () => otherProfileMenu(field, anchor) : null,
      profileName,
    });
    at(anchor, items);
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
        toast(tn(result.document_ids.length, "{n} document generated", "{n} documents generated"));
        finish();
      } else toast(t("Fill set saved"));
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

  const missing = evaluation.used_roles.filter((role) => !profiles[role]);
  const total = s.vars.length;
  const empty = evaluation.empty.length;
  const fields = tn(total, "{n} field", "{n} fields");
  let summary = empty ? t("{fields} · {n} empty", { fields, n: empty }) : t("{fields} · all filled", { fields });
  if (missing.length) summary += ` · ${t("choose profiles for: {roles}", { roles: missing.join(", ") })}`;

  const title = s.request_id
    ? tn(s.templates.length, "Your AI assistant wants to fill in {n} document", "Your AI assistant wants to fill in {n} documents")
    : t("Fill documents");
  const explanation = `${s.templates.map((tpl) => tpl.name).join(" · ")}. ${t("Check each blank and pick where its value comes from.")}`;

  return html`
    <div class="sheet-head">
      <${SheetHead} icon="doc" title=${title} reason=${s.reason ? `${s.reason}. ${explanation}` : explanation} />
      <div class="sheet-form">
        <label>${t("Fill set")}</label>
        <input type="text" value=${name} spellcheck="false" onInput=${(e) => setName(e.target.value)} />
        ${Object.entries(s.roles).map(
          ([role, description]) => html`
            <label title=${description}>${role}</label>
            <${Select} value=${profiles[role] ?? ""} onChange=${(value) => chooseProfile(role, value)} options=${[
              ["", evaluation.used_roles.includes(role) ? t("Choose a profile…") : t("Not needed")],
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
              ask=${s.ask[v.name]}
              profileName=${profileName}
              active=${v.name === active}
              typing=${v.name === typing}
              onSelect=${() => select(v.name)}
              onMenu=${(anchor) => {
                select(v.name);
                sourceMenu(v.name, anchor);
              }}
              onTyped=${(value, enter) => {
                const next = value !== null ? { ...bindings, [v.name]: { value } } : bindings;
                if (value !== null) setBindings(next);
                const following = enter && v.name in s.ask ? nextAsked(v.name, next) : null;
                setTyping(following);
                if (following) select(following);
              }} />`
          )}
        </div>
        <div class="fill-preview">
          <div class=${`tabs ${s.templates.length > 1 ? "" : "hidden"}`}>
            ${s.templates.map(
              (tpl) => html`<div class=${`tab ${tpl.id === tab ? "active" : ""}`} onClick=${() => setTab(tpl.id)}>${tpl.name}</div>`
            )}
          </div>
          ${previewError
            ? html`<div class="preview-area"><div class="preview-error">${previewError}</div></div>`
            : html`<${Preview} preview=${previews?.[tab]} placeholder=${t("Loading preview…")} active=${active}
                revealSeq=${revealSeq} onPick=${select} />`}
        </div>
      </div>
    </div>
    <div class="sheet-foot">
      <span class="note">${t("Your data stays in Blankey. The AI assistant only learns where each value comes from, never the value.")}</span>
      <button disabled=${busy} onClick=${cancel}>${t("Cancel")}</button>
      <button disabled=${busy} onClick=${() => save(false)}>${t("Save")}</button>
      <button class="primary" disabled=${busy} onClick=${() => save(true)}>${t("Save and Generate")}</button>
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
