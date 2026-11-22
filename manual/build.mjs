// Builds the user manual: drives the real UI over the fake backend for the screenshots (manual/scenes), then lays
// them out with each language's text (manual/languages/<code>) into dist/manual/Blankey-manual-<code>.pdf.
//
//   node manual/build.mjs [--lang en,bg] [--scene cases] [--shots | --pdf] [--headed]

import { readdir, readFile, rm } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { chromium } from "playwright";
import { App } from "./lib/app.mjs";
import { check } from "./lib/check.mjs";
import { chapters, render } from "./lib/render.mjs";
import { serve } from "./lib/server.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const root = dirname(here);
const out = join(root, "dist", "manual");
const SCENES = ["workspace", "online", "cases", "assistant"];

const { values: flags } = parseArgs({
  options: { lang: { type: "string" }, scene: { type: "string" }, shots: { type: "boolean" }, pdf: { type: "boolean" }, headed: { type: "boolean" } },
});

/** The languages to build: the folders of manual/languages, each of which the UI must have too. */
async function languages() {
  const folders = (await readdir(join(here, "languages"), { withFileTypes: true })).filter((entry) => entry.isDirectory()).map((entry) => entry.name);
  const ui = Object.keys(JSON.parse(await readFile(join(root, "ui", "i18n", "languages.json"), "utf8")));
  const wanted = flags.lang ? flags.lang.split(",") : folders.sort((a, b) => (a === "en" ? -1 : b === "en" ? 1 : a.localeCompare(b)));
  for (const code of wanted) {
    if (!folders.includes(code)) throw new Error(`manual/languages/${code} does not exist`);
    if (!ui.includes(code)) throw new Error(`"${code}" is not a language of the UI (ui/i18n/languages.json)`);
  }
  return wanted;
}

/** Runs the scenes in one language; the ids of the screenshots taken. */
async function shoot(browser, origin, language, scenes) {
  const shotsDir = join(out, "shots", language);
  const taken = [];
  for (const name of scenes) {
    const { default: scene } = await import(`./scenes/${name}.mjs`);
    const app = await App.open(browser, { origin, language, shotsDir, scene });
    try {
      await scene.run(app);
    } catch (error) {
      await app.page.screenshot({ path: join(out, `failed-${language}-${scene.id}.png`) }).catch(() => {});
      throw new Error(`scene "${scene.id}" (${language}), after shot "${app.step}": ${error.message}`, { cause: error });
    }
    taken.push(...app.shots);
    await app.close();
    console.log(`${language}: ${scene.id}, ${app.shots.length} screenshots`);
  }
  return taken;
}

const codes = await languages();
const scenes = flags.scene ? flags.scene.split(",") : SCENES;
const partial = Boolean(flags.scene || flags.shots || flags.pdf);
const text = Object.fromEntries(await Promise.all(codes.map(async (code) => [code, await chapters(join(here, "languages", code))])));
const version = JSON.parse(await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8")).version;

const server = await serve(root);
const browser = await chromium.launch({ headless: !flags.headed });
try {
  const taken = {};
  if (!flags.pdf) {
    for (const code of codes) {
      if (!flags.scene) await rm(join(out, "shots", code), { recursive: true, force: true });
      taken[code] = await shoot(browser, server.origin, code, scenes);
    }
  }
  if (!partial) check(text, taken);
  if (!flags.shots && !flags.scene) {
    for (const code of codes) {
      const path = join(out, `Blankey-manual-${code}.pdf`);
      await render(browser, { language: code, chapters: text[code], dir: join(here, "languages", code), shotsDir: join(out, "shots", code), version, path, stylesheet: join(here, "manual.css"), icon: join(root, "ui", "icon.png") });
      console.log(path);
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
} finally {
  await browser.close();
  await server.close();
}
