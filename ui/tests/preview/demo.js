// Invented data for looking at the UI in a browser: three cases with their profiles, files and documents, on top of
// the test seed.

export function demo() {
  return {
    profiles: [
      { id: 1, name: "John", kind: "person" },
      { id: 2, name: "Acme", kind: "company" },
      { id: 3, name: "Maria Example", kind: "person", global: false },
      { id: 4, name: "Peter Sample", kind: "person", global: false },
      { id: 5, name: "Oak Street Holdings", kind: "company", global: false },
    ],
    fields: {
      1: [
        { key: "name", label: "Full name", type: "text", value: "John Watson" },
        { key: "egn", label: "EGN", type: "egn", value: "" },
      ],
      2: [],
      3: [
        { key: "name", label: "Full name", type: "text", value: "Maria Example" },
        { key: "egn", label: "EGN", type: "egn", value: "7501020018" },
        { key: "address.city", label: "City", type: "text", value: "Springfield" },
      ],
      4: [
        { key: "name", label: "Full name", type: "text", value: "Peter Sample" },
        { key: "egn", label: "EGN", type: "egn", value: "" },
      ],
      5: [{ key: "name", label: "Company name", type: "text", value: "Oak Street Holdings Ltd" }],
    },
    cases: [
      {
        id: 1,
        title: "Sale of apartment, 12 Oak Street",
        notes: "Seller Maria Example, buyer Peter Sample. The mortgage was released in October; the deed is signed on 14 November.",
        created_at: "2026-10-28T09:00:00",
        updated_at: "2026-11-02T15:10:00",
      },
      { id: 2, title: "Lease, shop on Main Road", notes: "", created_at: "2026-10-25T10:00:00", updated_at: "2026-10-30T11:20:00" },
      { id: 3, title: "Power of attorney, car sale", notes: "Peter Sample authorises his brother to sell the car.", created_at: "2026-10-18T14:00:00", updated_at: "2026-10-20T16:45:00" },
    ],
    caseLinks: [
      { case_id: 1, profile_id: 3, role: "seller" },
      { case_id: 1, profile_id: 4, role: "buyer" },
      { case_id: 2, profile_id: 5, role: "landlord" },
      { case_id: 2, profile_id: 1, role: "tenant" },
      { case_id: 3, profile_id: 4, role: "principal" },
    ],
    collected: [
      file(11, 1, "maria-id-front.jpg", "image/jpeg", 1.8, "Maria Example, seller, ID card front", "2026-10-28T09:12:00"),
      file(12, 1, "maria-id-back.jpg", "image/jpeg", 1.7, "Maria Example, seller, ID card back", "2026-10-28T09:12:00"),
      file(13, 1, "title-deed-2019.pdf", "application/pdf", 0.9, "Title deed of the apartment, 2019", "2026-10-28T09:13:00"),
      file(14, 1, "sale-contract-draft.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", 0.1, "Draft sale contract from the buyer's lawyer", "2026-10-29T10:02:00"),
      file(15, 1, "peter-id.png", "image/png", 2.4, "Peter Sample, buyer, ID card", "2026-10-29T10:03:00"),
      file(16, 2, "lease-model.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", 0.06, "Our model lease for shops", "2026-10-25T10:05:00"),
      file(17, 3, "car-registration.pdf", "application/pdf", 0.4, "Registration certificate of the car, Peter Sample", "2026-10-18T14:10:00"),
    ],
    templates: [
      { id: "nzok", name: "NZOK request", kind: "pdf_form", blanks: 2, used_by: ["acme"], description: "Data subject request" },
      { id: "loan", name: "Loan contract", kind: "docx", blanks: 2, used_by: [], description: "" },
      { id: "oak-sale-contract", name: "Sale contract, 12 Oak Street", kind: "docx", blanks: 9, used_by: ["Oak Street sale"], description: "From the buyer's lawyer's draft", case_id: 1 },
    ],
    sets: [
      { id: 1, name: "acme", templates: ["nzok"], profiles: ["John"], updated_at: "2026-10-10T10:00:00" },
      { id: 2, name: "Oak Street sale", templates: ["oak-sale-contract"], profiles: ["Maria Example", "Peter Sample"], updated_at: "2026-11-02T15:10:00", case_id: 1 },
    ],
    docs: [
      { id: 1, title: "NZOK (acme)", template_id: "nzok", filename: "nzok.pdf", pages: 2, created_at: "2026-10-10T10:00:00" },
      { id: 2, title: "Loan (acme)", template_id: "loan", filename: "loan.docx", pages: 0, created_at: "2026-10-10T11:00:00" },
      { id: 3, title: "Sale contract, 12 Oak Street (Oak Street sale)", template_id: "oak-sale-contract", filename: "oak-sale-contract.docx", pages: 0, created_at: "2026-11-02T15:10:00", case_id: 1 },
    ],
    activity: [
      { id: 21, label: "Fill", details: "Oak Street sale", outcome: "generated", status: "done", time: "2026-11-02T15:10", kind: "fill", document_ids: [3], fill_set_id: 2, case_id: 1 },
      { id: 20, label: "Profile input", details: "Peter Sample: 2 fields", outcome: "saved", status: "done", time: "2026-10-29T10:20", kind: "profile_input", document_ids: [], fill_set_id: null, case_id: 1 },
      { id: 6, label: "Fill", details: "acme", outcome: "generated", status: "done", time: "2026-10-09T10:00", kind: "fill", document_ids: [1], fill_set_id: 1 },
    ],
  };
}

const file = (id, caseId, name, type, mb, description, created_at) => ({
  id,
  case_id: caseId,
  profile_id: null,
  link_id: "",
  name,
  type,
  size: Math.round(mb * 1024 * 1024),
  description,
  created_at,
  link_title: "",
  profile_name: null,
});
