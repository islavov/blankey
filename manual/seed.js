// The manual's invented data: where a scene starts, on top of the test seed and the preview's demo cases.

import { seed } from "/ui/tests/backend.js";
import { demo } from "/ui/tests/preview/demo.js";

const signedOut = () => ({ ...seed().relay, signed_in: false, state: "signed_out", account: null, chat: false, grants: [] });

const egn = (data, profileId, value) => (data.fields[profileId].find((f) => f.key === "egn").value = value);

/** The demo cases with the seller's EGN known; the buyer's too unless it is `missing`, as the case's chat has it. */
function cases({ missing } = {}) {
  const data = demo();
  // the test seed's global template, named for the manual's cases
  Object.assign(data.templates.find((tpl) => tpl.id === "nzok"), { name: "Lease contract", description: "" });
  egn(data, 3, "0000000000");
  if (!missing) egn(data, 4, "1111111111");
  return data;
}

/** The states a scene can start in, by the scene's `start`. */
export const starts = {
  // a first launch: no vault, nobody signed in
  first: () => ({ initialized: false, unlocked: false, chats: {}, relay: signedOut() }),
  // an open vault with the demo cases, nobody signed in
  "signed-out": () => ({ ...cases(), relay: signedOut() }),
  // an open vault with the demo cases, signed in, with the built-in assistant
  cases: () => cases(),
  // the same while the assistant still waits for the buyer's EGN
  assistant: () => cases({ missing: true }),
};

/** The specimen shown for a page image of this width: the ID card for card-sized pages, the form page otherwise. */
export const specimen = (width) => (width === 856 || width === undefined ? "/manual/assets/id-card.svg" : "/manual/assets/form-page.svg");
