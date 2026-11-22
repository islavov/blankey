// A language's chapters (Markdown) and its screenshots as one HTML document, printed to PDF by the browser.

import { readdir, readFile, writeFile } from "node:fs/promises";
import { basename, dirname, join, relative } from "node:path";
import { pathToFileURL } from "node:url";
import { Marked } from "marked";

const escape = (text) => text.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** The chapters of a language folder, in file name order: `[{ file, markdown }]`. */
export async function chapters(dir) {
  const files = (await readdir(dir)).filter((name) => name.endsWith(".md")).sort();
  return Promise.all(files.map(async (file) => ({ file, markdown: await readFile(join(dir, file), "utf8") })));
}

/** The width each screenshot had on screen, in CSS pixels by shot id: half the PNG's width, as they are taken at 2x. */
async function screenWidths(shotsDir) {
  const files = (await readdir(shotsDir)).filter((name) => name.endsWith(".png"));
  const entries = files.map(async (name) => [basename(name, ".png"), (await readFile(join(shotsDir, name))).readUInt32BE(16) / 2]);
  return Object.fromEntries(await Promise.all(entries));
}

/**
 * One chapter as HTML. Headings get ids (`## Title {#id}` sets one, for links between chapters) and are collected
 * into `headings`; `![caption](shot:id)` becomes a figure with that language's screenshot, never wider than it was on screen (`widths`), images on adjacent lines
 * a row of figures, and a `>` quote a warning.
 */
function chapterHtml({ file, markdown }, number, shots, widths, headings) {
  let count = 0;
  const marked = new Marked({
    renderer: {
      heading({ tokens, depth }) {
        let html = this.parser.parseInline(tokens);
        const named = html.match(/\s*\{#([\w-]+)\}\s*$/);
        if (named) html = html.slice(0, named.index);
        const id = named ? named[1] : `${basename(file, ".md")}-${++count}`;
        const label = depth === 1 ? `${number}. ${html}` : html;
        if (depth <= 2) headings.push({ depth, id, html: label });
        return `<h${depth} id="${id}">${label}</h${depth}>\n`;
      },
      image({ href, text }) {
        if (!href.startsWith("shot:")) throw new Error(`${file}: only shot: images are supported, not ${href}`);
        const id = href.slice(5);
        return `<figure><img src="${shots}/${id}.png" style="--screen: ${widths[id]}px" alt="${escape(text)}"><figcaption>${escape(text)}</figcaption></figure>`;
      },
      paragraph({ tokens }) {
        // screenshots alone in a paragraph are figures, which cannot sit inside <p>; several of them share a row
        const html = this.parser.parseInline(tokens);
        if (!/^(<figure>.*?<\/figure>\s*)+$/s.test(html)) return `<p>${html}</p>\n`;
        return html.match(/<figure>/g).length > 1 ? `<div class="figures">${html}</div>\n` : `${html}\n`;
      },
    },
  });
  return `<section class="chapter">${marked.parse(markdown)}</section>`;
}

/** Writes the language's manual as HTML next to its screenshots and prints it to `path`. */
export async function render(browser, { language, chapters, dir, shotsDir, version, path, stylesheet, icon }) {
  const words = JSON.parse(await readFile(join(dir, "manual.json"), "utf8"));
  const headings = [];
  const shots = relative(dirname(path), shotsDir).split("\\").join("/");
  const widths = await screenWidths(shotsDir);
  const body = chapters.map((chapter, index) => chapterHtml(chapter, index + 1, shots, widths, headings)).join("\n");
  const contents = headings.map(({ depth, id, html }) => `<li class="d${depth}"><a href="#${id}">${html}</a></li>`).join("\n");
  const html = `<!doctype html>
<html lang="${language}">
<head>
<meta charset="utf-8">
<title>Blankey: ${escape(words.title)}</title>
<style>${await readFile(stylesheet, "utf8")}</style>
</head>
<body>
<section class="cover">
  <img src="data:image/png;base64,${(await readFile(icon)).toString("base64")}" alt="">
  <h1>Blankey</h1>
  <p class="subtitle">${escape(words.title)}</p>
  <p class="version">${escape(words.version.replace("{version}", version))}</p>
</section>
<nav class="contents">
  <h1>${escape(words.contents)}</h1>
  <ol>
${contents}
  </ol>
</nav>
${body}
</body>
</html>
`;
  const htmlPath = path.replace(/\.pdf$/, ".html");
  await writeFile(htmlPath, html);
  const page = await browser.newPage();
  await page.goto(pathToFileURL(htmlPath).href);
  await page.evaluate(() => Promise.all([...document.images].map((image) => image.decode())));
  await page.pdf({
    path,
    format: "A4",
    printBackground: true,
    outline: true,
    tagged: true,
    displayHeaderFooter: true,
    headerTemplate: "<span></span>",
    footerTemplate: `<div style="width:100%;font:9px -apple-system,system-ui,sans-serif;color:#888;padding:0 18mm;display:flex;justify-content:space-between"><span>Blankey ${escape(version)}</span><span class="pageNumber"></span></div>`,
    margin: { top: "18mm", bottom: "20mm", left: "18mm", right: "18mm" },
  });
  await page.close();
}
