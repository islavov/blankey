// Where each blank's value comes from: the template default, a typed value, a value the assistant suggested, or a
// vault path.
// Bindings: {default: true} | {value: "..."} | {path: "role.key"}.

export const keyOf = (binding) => JSON.stringify(binding, Object.keys(binding).sort());

const defaultExpression = (v) => {
  const expressions = [...new Set(Object.values(v.defaults))];
  return { any: expressions.length > 0, text: expressions.length === 1 ? expressions[0] : "per template" };
};

/** What a fill row shows: the source kind (chip), its path or expression, and the value or a hint. */
export function describe(v, binding = { default: true }, row = { value: "", empty: true }, suggested) {
  let kind, path;
  if ("path" in binding) [kind, path] = ["Vault", binding.path];
  else if ("value" in binding) [kind, path] = [binding.value === suggested ? "Suggested" : "Value", ""];
  else [kind, path] = ["Default", defaultExpression(v).text.replace(/\{\{|\}\}/g, "").trim()];
  if (!row.empty) return { kind, path, text: row.value, empty: false };
  if (kind === "Vault") return { kind, path, text: "(empty in profile)", empty: true };
  return { kind: "Empty", path, text: "Choose a source…", empty: true };
}

/** Items for the source pop-up of one blank; `choose(binding)` and `type()` apply the pick. */
export function sourceItems({ v, binding, evaluation, suggested, hasRoles, choose, type }) {
  const current = keyOf(binding || { default: true });
  const option = (label, pick) => ({ label, checked: keyOf(pick) === current, action: () => choose(pick) });
  const items = [{ label: "Type a value…", action: type }, "-"];

  const expression = defaultExpression(v);
  if (expression.any) {
    const shown = evaluation.rows[v.name]?.default_value || "(empty)";
    items.push(option(expression.text ? `Template default  ${expression.text}  —  ${shown}` : `Template default  —  ${shown}`, { default: true }));
  }
  const typed = binding?.value;
  if (suggested) items.push(option(`Suggested by the assistant  —  ${suggested}`, { value: suggested }));
  if (typed && typed !== suggested) items.push(option(`Typed  —  ${typed}`, { value: typed }));

  for (const [role, entries] of Object.entries(evaluation.sources)) {
    if (!entries.length) continue;
    if (items[items.length - 1] !== "-") items.push("-");
    items.push({ head: role });
    for (const entry of entries) items.push(option(`${entry.path}  —  ${entry.value || "(empty)"}`, { path: entry.path }));
  }
  if (!Object.keys(evaluation.sources).length && hasRoles) {
    items.push("-", { head: "Choose profiles above to pick vault fields" });
  }
  return items;
}
