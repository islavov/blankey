// Every language of the manual has the same chapters with the same screenshots, and every screenshot is used.

const SHOT = /\]\(shot:([\w-]+)\)/g;

/** The screenshot ids a chapter's Markdown refers to, in order. */
export const shotsOf = (markdown) => [...markdown.matchAll(SHOT)].map((match) => match[1]);

/**
 * Throws with every problem found. `text` is `{ code: [{ file, markdown }] }`, `taken` is `{ code: [shot id] }`.
 */
export function check(text, taken) {
  const problems = [];
  const [reference, ...others] = Object.keys(text);
  const files = (code) => text[code].map((chapter) => chapter.file);
  for (const code of others) {
    for (const file of files(reference)) if (!files(code).includes(file)) problems.push(`${code}: chapter ${file} is missing (${reference} has it)`);
    for (const file of files(code)) if (!files(reference).includes(file)) problems.push(`${reference}: chapter ${file} is missing (${code} has it)`);
  }
  for (const code of Object.keys(text)) {
    const used = new Set();
    for (const { file, markdown } of text[code]) {
      const shots = shotsOf(markdown);
      shots.forEach((id) => used.add(id));
      for (const id of shots) if (!taken[code].includes(id)) problems.push(`${code}/${file}: no scene takes the screenshot "${id}"`);
      const same = text[reference].find((chapter) => chapter.file === file);
      if (same && code !== reference && shotsOf(same.markdown).join() !== shots.join()) problems.push(`${code}/${file}: its screenshots differ from ${reference}/${file}`);
    }
    for (const id of taken[code]) if (!used.has(id)) problems.push(`${code}: the screenshot "${id}" is in no chapter`);
  }
  if (problems.length) throw new Error(`The manual's languages do not match:\n  ${problems.join("\n  ")}`);
}
