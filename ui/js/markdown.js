// The Markdown the assistant writes, as elements: paragraphs, headings, lists, code blocks and rules; in a line, bold,
// italic and code. Anything else stays as written, and so does a mark that is not closed yet while the text streams.

import { html } from "./lib.js";

const FENCE = /^\s*(`{3,}|~{3,})/;
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)(?:\s+#+)?\s*$/;
const RULE = /^\s{0,3}([-*_])(?:\s*\1){2,}\s*$/;
const ITEM = /^(\s*)(?:[-*+]|(\d{1,9})[.)])\s+(.*)$/;
const WORD = /[\p{L}\p{N}_]/u;

// an escaped mark, code, bold italic, bold (** or __), italic (* or _)
const INLINE =
  /\\([\\`*_{}[\]()#+\-.!>~|])|(`+)(?!`)([^]*?[^`])\2(?!`)|\*\*\*(?=\S)(.*?\S)\*\*\*|\*\*(?=\S)(.*?\S)\*\*|__(?=\S)(.*?\S)__(?![\p{L}\p{N}_])|\*(?=[^\s*])(.*?\S)\*|_(?=[^\s_])(.*?\S)_(?![\p{L}\p{N}_])/gsu;

/** The text of a line or a paragraph with its bold, italic and code as elements. */
function inline(text) {
  const out = [];
  const marks = new RegExp(INLINE);
  let last = 0;
  let m;
  while ((m = marks.exec(text))) {
    const [all, escaped, , code, boldItalic, bold, underBold, italic, underItalic] = m;
    // an underscore inside a word is part of it, as in a field key
    if ((underBold ?? underItalic) != null && m.index > 0 && WORD.test(text[m.index - 1])) {
      marks.lastIndex = m.index + 1;
      continue;
    }
    if (m.index > last) out.push(text.slice(last, m.index));
    if (escaped != null) out.push(escaped);
    else if (code != null) out.push(html`<code>${code}</code>`);
    else if (boldItalic != null) out.push(html`<strong><em>${inline(boldItalic)}</em></strong>`);
    else if ((bold ?? underBold) != null) out.push(html`<strong>${inline(bold ?? underBold)}</strong>`);
    else out.push(html`<em>${inline(italic ?? underItalic)}</em>`);
    last = m.index + all.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** List items as lists: deeper items go into the item above them, and a change between numbers and bullets starts a new list. */
function lists(items) {
  const base = items[0].indent;
  const groups = [];
  for (const item of items) {
    const group = groups.at(-1);
    if (item.indent > base && group) group.entries.at(-1).children.push(item);
    else if (group && group.ordered === item.ordered) group.entries.push({ ...item, children: [] });
    else groups.push({ ordered: item.ordered, start: item.start, entries: [{ ...item, children: [] }] });
  }
  return groups.map(({ ordered, start, entries }) => {
    const rows = entries.map((entry) => html`<li>${inline(entry.text)}${entry.children.length ? lists(entry.children) : null}</li>`);
    return ordered ? html`<ol start=${start}>${rows}</ol>` : html`<ul>${rows}</ul>`;
  });
}

/** True when the line starts a block of its own, which ends the paragraph before it. */
const startsBlock = (line) => FENCE.test(line) || HEADING.test(line) || RULE.test(line) || ITEM.test(line);

/** The blocks of a Markdown text, as elements. */
function blocks(lines) {
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const fence = line.match(FENCE);
    if (fence) {
      // a fence that is not closed, as while it streams, runs to the end
      const code = [];
      for (i++; i < lines.length && !(lines[i].trim().startsWith(fence[1]) && /^(`+|~+)$/.test(lines[i].trim())); i++) code.push(lines[i]);
      i++;
      out.push(html`<pre><code>${code.join("\n")}</code></pre>`);
      continue;
    }
    const heading = line.match(HEADING);
    if (heading) {
      out.push(html`<${`h${Math.min(heading[1].length + 2, 6)}`}>${inline(heading[2])}<//>`);
      i++;
      continue;
    }
    if (RULE.test(line)) {
      out.push(html`<hr />`);
      i++;
      continue;
    }
    if (ITEM.test(line)) {
      const items = [];
      for (; i < lines.length; i++) {
        const item = !RULE.test(lines[i]) && lines[i].match(ITEM);
        if (item) items.push({ indent: item[1].length, ordered: item[2] != null, start: Number(item[2] ?? 1), text: item[3] });
        else if (!lines[i].trim()) {
          // a blank line between two items keeps the list going
          const next = lines.slice(i + 1).find((l) => l.trim());
          if (!next || RULE.test(next) || !ITEM.test(next)) break;
        } else if (/^\s/.test(lines[i]) && !FENCE.test(lines[i])) items.at(-1).text += `\n${lines[i].trim()}`;
        else break;
      }
      out.push(...lists(items));
      continue;
    }
    const paragraph = [line];
    for (i++; i < lines.length && lines[i].trim() && !startsBlock(lines[i]); i++) paragraph.push(lines[i]);
    out.push(html`<p>${inline(paragraph.join("\n"))}</p>`);
  }
  return out;
}

/** A text the assistant wrote, with its Markdown shown as formatting. */
export const Markdown = ({ text }) => html`<div class="chat-text">${blocks(String(text).split("\n"))}</div>`;
