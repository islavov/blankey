// Checks the UI catalogs (ui/i18n/<code>.json) against the text the code translates: every t()/tn() literal in ui/js
// and every t/tf/tn/mark literal in src-tauri/src (tests excluded). Fails on missing or unused entries, and on
// plural entries that are not {one, other}. `--missing` prints the missing keys of each catalog as JSON.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const STRING = String.raw`"(?:[^"\\]|\\.)*"`;
const ARG = String.raw`&?[\w.:]+(?:\(\))?`;

function files(dir, extension) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path, extension);
    return path.endsWith(extension) ? [path] : [];
  });
}

const literal = (text) => JSON.parse(text);

/** key -> "text" | "plural" */
const used = new Map();
const use = (key, kind, where) => {
  if (used.has(key) && used.get(key).kind !== kind) throw new Error(`${where}: "${key}" is used both as text and as a plural`);
  used.set(key, { kind, where });
};

for (const path of [...files(join(ROOT, "ui/js"), ".js"), join(ROOT, "ui/index.html")]) {
  const source = readFileSync(path, "utf8");
  for (const m of source.matchAll(new RegExp(String.raw`\bt\(\s*(${STRING})`, "g"))) use(literal(m[1]), "text", path);
  for (const m of source.matchAll(new RegExp(String.raw`\btn\(\s*[^,]+?,\s*(${STRING})\s*,\s*(${STRING})`, "g"))) {
    use(literal(m[1]), "plural", path);
  }
}

for (const path of files(join(ROOT, "src-tauri/src"), ".rs")) {
  const source = readFileSync(path, "utf8").split("#[cfg(test)]")[0];
  for (const m of source.matchAll(new RegExp(String.raw`\b(?:t|tf)\(\s*${ARG}\s*,\s*(${STRING})`, "g"))) use(literal(m[1]), "text", path);
  for (const m of source.matchAll(new RegExp(String.raw`\btn\(\s*${ARG}\s*,\s*[^,]+,\s*(${STRING})\s*,\s*(${STRING})`, "g"))) {
    use(literal(m[1]), "plural", path);
  }
  for (const m of source.matchAll(new RegExp(String.raw`\bmark\(\s*(${STRING})`, "g"))) use(literal(m[1]), "text", path);
}

const languages = JSON.parse(readFileSync(join(ROOT, "ui/i18n/languages.json"), "utf8"));
let problems = 0;
const missingOut = {};
for (const code of Object.keys(languages).filter((code) => code !== "en")) {
  const catalog = JSON.parse(readFileSync(join(ROOT, `ui/i18n/${code}.json`), "utf8"));
  const missing = [...used.keys()].filter((key) => !(key in catalog));
  const unused = Object.keys(catalog).filter((key) => !used.has(key));
  const wrong = Object.entries(catalog).filter(([key, value]) => {
    const kind = used.get(key)?.kind;
    if (kind === "plural") return !(value && typeof value.one === "string" && typeof value.other === "string");
    return kind === "text" && typeof value !== "string";
  });
  missingOut[code] = missing;
  for (const key of missing) console.error(`${code}: missing "${key}" (${used.get(key).where.replace(ROOT, "")})`);
  for (const key of unused) console.error(`${code}: unused "${key}"`);
  for (const [key] of wrong) console.error(`${code}: "${key}" needs ${used.get(key).kind === "plural" ? "{one, other}" : "a string"}`);
  problems += missing.length + unused.length + wrong.length;
}
if (process.argv.includes("--missing")) console.log(JSON.stringify(missingOut, null, 2));
console.error(`i18n: ${used.size} texts, ${problems} problems`);
process.exit(problems ? 1 : 0);
