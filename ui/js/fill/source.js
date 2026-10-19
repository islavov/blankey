// Where each blank's value comes from: the template default, a typed value, a value the assistant suggested, or a
// vault path.
// Bindings: {default: true} | {value: "..."} | {path: "role.key"} | {path: "key", profile: id}.

import { t } from "../i18n.js";

export const keyOf = (binding) => JSON.stringify(binding, Object.keys(binding).sort());

/** The chip text of a source kind; the kind itself stays English, it is also the chip's CSS class. */
export const kindLabel = (kind) =>
  ({ Vault: t("Vault"), Suggested: t("Suggested"), Value: t("Value"), Default: t("Default"), Empty: t("Empty"), Ask: t("To type") })[kind] ||
  kind;

/** The menu heading of a source group: a role, or the name of a profile read directly ("profile:ID"). */
const sourceHead = (group, profileName) => (group.startsWith("profile:") ? profileName(Number(group.slice(8))) : group);

const defaultExpression = (v) => {
  const expressions = [...new Set(Object.values(v.defaults))];
  return { any: expressions.length > 0, text: expressions.length === 1 ? expressions[0] : t("per template") };
};

/**
 * What a fill row shows: the source kind (chip), its path or expression, and the value or a hint.
 * `ask` is the assistant's hint when it asked the user to type this blank; `profileName(id)` names a profile.
 */
export function describe(v, binding = { default: true }, row = { value: "", empty: true }, suggested, ask, profileName = String) {
  let kind, path;
  if ("path" in binding) [kind, path] = ["Vault", "profile" in binding ? `${profileName(binding.profile)} › ${binding.path}` : binding.path];
  else if ("value" in binding) [kind, path] = [binding.value === suggested ? "Suggested" : "Value", ""];
  else [kind, path] = ["Default", defaultExpression(v).text.replace(/\{\{|\}\}/g, "").trim()];
  if (!row.empty) return { kind, path, text: row.value, empty: false };
  if (kind === "Vault") return { kind, path, text: t("(empty in profile)"), empty: true };
  if (ask !== undefined && "value" in binding) return { kind: "Ask", path: "", text: ask || t("Type a value…"), empty: true };
  return { kind: "Empty", path, text: t("Choose a source…"), empty: true };
}

/** Items for the source pop-up of one blank; `choose(binding)` and `type()` apply the pick. */
export function sourceItems({ v, binding, evaluation, suggested, hasRoles, choose, type, otherProfile, profileName = String }) {
  const current = keyOf(binding || { default: true });
  const option = (label, pick) => ({ label, checked: keyOf(pick) === current, action: () => choose(pick) });
  const items = [{ label: t("Type a value…"), action: type }, "-"];

  const expression = defaultExpression(v);
  if (expression.any) {
    const shown = evaluation.rows[v.name]?.default_value || t("(empty)");
    const label = expression.text
      ? t("Template default  {expression}  —  {value}", { expression: expression.text, value: shown })
      : t("Template default  —  {value}", { value: shown });
    items.push(option(label, { default: true }));
  }
  const typed = binding?.value;
  if (suggested) items.push(option(t("Suggested by the assistant  —  {value}", { value: suggested }), { value: suggested }));
  if (typed && typed !== suggested) items.push(option(t("Typed  —  {value}", { value: typed }), { value: typed }));

  for (const [group, entries] of Object.entries(evaluation.sources)) {
    if (!entries.length) continue;
    if (items[items.length - 1] !== "-") items.push("-");
    items.push({ head: sourceHead(group, profileName) });
    for (const entry of entries) {
      const pick = entry.profile ? { path: entry.path, profile: entry.profile } : { path: entry.path };
      items.push(option(`${entry.path}  —  ${entry.value || t("(empty)")}`, pick));
    }
  }
  if (!Object.keys(evaluation.sources).length && hasRoles) {
    items.push("-", { head: t("Choose profiles above to pick vault fields") });
  }
  if (otherProfile) items.push("-", { label: t("From another profile…"), action: otherProfile });
  return items;
}
