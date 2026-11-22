// What the manual answers in place of the fake backend, which fills one fixed pair of templates for one fixed person:
// the fill of the manual's own case, the sale contract of 12 Oak Street with a seller and a buyer, and requests that
// name the profile they are for.

const TEMPLATE = { id: "oak-sale-contract", name: "Sale contract, 12 Oak Street", kind: "docx" };
const ROLES = { seller: "Who sells the apartment", buyer: "Who buys it" };
const VARS = [
  { name: "seller_name", label: "Seller", from: "seller.name" },
  { name: "seller_egn", label: "Seller's EGN", from: "seller.egn" },
  { name: "seller_address", label: "Seller's address", from: "seller.address" },
  { name: "buyer_name", label: "Buyer", from: "buyer.name" },
  { name: "buyer_egn", label: "Buyer's EGN", from: "buyer.egn" },
  { name: "price", label: "Price" },
  { name: "deed_date", label: "Date of the deed" },
];
const SUGGESTED = { price: "185 000 EUR", deed_date: "14 November 2026" };

/** The value of a profile's field, "" when it has none. */
const field = (db, profileId, key) => (db.fields[profileId] || []).find((f) => f.key === key)?.value ?? "";

/** The value at "role.key" with the roles' profiles as chosen in the form. */
const atPath = (db, profiles, path) => {
  const [role, ...key] = path.split(".");
  return profiles[role] ? field(db, Number(profiles[role]), key.join(".")) : "";
};

/** What `fill_evaluate` answers: each blank's value under these bindings, and the fields each role offers. */
function evaluate(db, { bindings = {}, profiles = {} }) {
  const rows = {};
  for (const v of VARS) {
    const fallback = v.from ? atPath(db, profiles, v.from) : "";
    const binding = bindings[v.name];
    let value = fallback;
    if (binding?.path) value = binding.profile ? field(db, Number(binding.profile), binding.path) : atPath(db, profiles, binding.path);
    else if (binding && "value" in binding) value = binding.value;
    rows[v.name] = { value, empty: !value, default_value: fallback };
  }
  const sources = {};
  for (const role of Object.keys(ROLES)) {
    if (profiles[role]) sources[role] = (db.fields[Number(profiles[role])] || []).map((f) => ({ path: `${role}.${f.key}`, value: f.value }));
  }
  return { sources, rows, empty: VARS.filter((v) => rows[v.name].empty).map((v) => v.name), used_roles: Object.keys(ROLES) };
}

/** The contract as the preview shows it: each blank with its value, or its name while it is empty. */
function contract(rows) {
  const blank = (name) => {
    const row = rows[name];
    return `<span class="blank${row.empty ? " is-empty" : ""}" data-var="${name}">${row.empty ? name : row.value}</span>`;
  };
  return {
    type: "html",
    names: VARS.map((v) => v.name),
    html: `
      <h3 style="text-align:center">CONTRACT FOR THE SALE OF AN APARTMENT</h3>
      <p>Today, ${blank("deed_date")}, in Springfield, this contract was signed between:</p>
      <p>1. ${blank("seller_name")}, EGN ${blank("seller_egn")}, with address ${blank("seller_address")}, called the SELLER, and</p>
      <p>2. ${blank("buyer_name")}, EGN ${blank("buyer_egn")}, called the BUYER.</p>
      <p>I. The SELLER sells to the BUYER the apartment at 12 Oak Street, Springfield, with an area of 78 square metres, together with its cellar and the matching share of the common parts of the building.</p>
      <p>II. The price is ${blank("price")}, which the BUYER pays to the SELLER by bank transfer on the day of the deed.</p>
      <p>III. The SELLER declares that the apartment has no mortgages or other burdens and that nobody else has rights over it.</p>
      <p>IV. The costs of the transfer are shared equally between the two parties.</p>
      <p style="margin-top:2em">SELLER: ....................&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;BUYER: ....................</p>`,
  };
}

/** The commands the manual answers in place of the fake backend's, as `command(args, backend)`. */
export const commands = {
  next_request: (args, { db }) => {
    const request = db.requests[0];
    if (!request) return null;
    return { request, profile_name: db.profiles.find((p) => p.id === request.payload.profile_id)?.name ?? "", existing: {} };
  },
  fill_open: ({ args }, { db }) => ({
    request_id: args.request_id,
    reason: db.requests.find((r) => r.id === args.request_id)?.payload.reason || "",
    fill_set_id: args.fill_set_id,
    name: "Oak Street sale",
    templates: [TEMPLATE],
    roles: ROLES,
    profiles: { seller: 3, buyer: 4 },
    all_profiles: db.profiles.filter((p) => p.global !== false || db.caseLinks.some((l) => l.case_id === 1 && l.profile_id === p.id)),
    case_id: 1,
    vars: VARS.map((v) => ({ name: v.name, label: v.label, defaults: v.from ? { [TEMPLATE.id]: `{{ ${v.from} }}` } : {}, contexts: [] })),
    bindings: {},
    suggested_values: SUGGESTED,
    ask: {},
  }),
  fill_evaluate: ({ args }, { db }) => evaluate(db, args),
  fill_preview: ({ args }, { db }) => ({ [TEMPLATE.id]: contract(evaluate(db, args).rows) }),
  fill_save: ({ args }, { db }) => {
    db.requests = db.requests.filter((r) => r.id !== args.request_id);
    const id = Math.max(0, ...db.docs.map((d) => d.id)) + 1;
    db.docs.push({ id, title: `${TEMPLATE.name} (${args.name})`, template_id: TEMPLATE.id, filename: "oak-sale-contract.docx", pages: 0, created_at: "2026-11-03T10:00:00", case_id: 1 });
    db.lastFill = evaluate(db, args).rows;
    return { fill_set_id: 2, document_ids: args.generate ? [id] : [] };
  },
  document_preview: ({ id }, { db }) => {
    const { title, filename } = db.docs.find((d) => d.id === id);
    const filled = contract(db.lastFill || evaluate(db, { profiles: { seller: 3, buyer: 4 } }).rows);
    return { title, filename, preview: { ...filled, html: filled.html.replace(/<span class="blank[^>]*>([^<]*)<\/span>/g, "$1") } };
  },
};
