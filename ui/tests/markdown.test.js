import assert from "node:assert/strict";
import { before, test } from "node:test";
import { GlobalRegistrator } from "@happy-dom/global-registrator";

let shown;
before(async () => {
  GlobalRegistrator.register({ url: "http://localhost/" });
  const { h, render } = await import(new URL("../vendor/preact-htm.js", import.meta.url).href);
  const { Markdown } = await import(new URL("../js/markdown.js", import.meta.url).href);
  /** The HTML a Markdown text is shown as, inside its .chat-text. */
  shown = (text) => {
    const root = document.createElement("div");
    render(h(Markdown, { text }), root);
    return root.firstChild.innerHTML;
  };
});

test("plain text stays as written, a blank line starts a new paragraph", () => {
  assert.equal(shown("Mary sells, John buys.\nNo deed yet."), "<p>Mary sells, John buys.\nNo deed yet.</p>");
  assert.equal(shown("First.\n\nSecond."), "<p>First.</p><p>Second.</p>");
  assert.equal(shown("<b>not html</b> & co"), "<p>&lt;b&gt;not html&lt;/b&gt; &amp; co</p>");
});

test("bold, italic and code in a line", () => {
  assert.equal(shown("The **seller** signs *first*, then `buyer.name`."), "<p>The <strong>seller</strong> signs <em>first</em>, then <code>buyer.name</code>.</p>");
  assert.equal(shown("__bold__ and _italic_ and ***both***"), "<p><strong>bold</strong> and <em>italic</em> and <strong><em>both</em></strong></p>");
  assert.equal(shown("**bold with *italic* inside**"), "<p><strong>bold with <em>italic</em> inside</strong></p>");
  assert.equal(shown("**Продавач:** име"), "<p><strong>Продавач:</strong> име</p>");
});

test("marks that are not formatting stay as written", () => {
  assert.equal(shown("seller.first_name and buyer.first_name"), "<p>seller.first_name and buyer.first_name</p>");
  assert.equal(shown("име_на_поле и _курсив_"), "<p>име_на_поле и <em>курсив</em></p>");
  assert.equal(shown("2 * 3 * 4"), "<p>2 * 3 * 4</p>");
  assert.equal(shown("`**not bold**`"), "<p><code>**not bold**</code></p>");
  assert.equal(shown("\\*not italic\\*"), "<p>*not italic*</p>");
  assert.equal(shown("The **sel"), "<p>The **sel</p>", "a mark not closed yet, while the text streams");
});

test("headings, rules and code blocks", () => {
  assert.equal(shown("# Summary\nText\n## Next **steps**"), "<h3>Summary</h3><p>Text</p><h4>Next <strong>steps</strong></h4>");
  assert.equal(shown("#hashtag"), "<p>#hashtag</p>");
  assert.equal(shown("Above\n\n---\n\nBelow"), "<p>Above</p><hr><p>Below</p>");
  assert.equal(shown("```jinja\n{{ seller.name }}\n**raw**\n```\nAfter"), "<pre><code>{{ seller.name }}\n**raw**</code></pre><p>After</p>");
  assert.equal(shown("```\nstill streaming"), "<pre><code>still streaming</code></pre>");
});

test("lists: bullets, numbers, nesting and items apart", () => {
  assert.equal(shown("Parties:\n- **Seller**: Mary\n- Buyer: John"), "<p>Parties:</p><ul><li><strong>Seller</strong>: Mary</li><li>Buyer: John</li></ul>");
  assert.equal(shown("1. Make a template\n2. Fill it"), '<ol start="1"><li>Make a template</li><li>Fill it</li></ol>');
  assert.equal(shown("1. First\n\n2. Second\n\nDone."), '<ol start="1"><li>First</li><li>Second</li></ol><p>Done.</p>');
  assert.equal(shown("- Seller\n  - name\n  - address\n- Buyer"), "<ul><li>Seller<ul><li>name</li><li>address</li></ul></li><li>Buyer</li></ul>");
  assert.equal(shown("1. Seller\n- name\n2. Buyer"), '<ol start="1"><li>Seller</li></ol><ul><li>name</li></ul><ol start="2"><li>Buyer</li></ol>');
  assert.equal(shown("- a long item\n  that goes on"), "<ul><li>a long item\nthat goes on</li></ul>");
  assert.equal(shown("**Note** first"), "<p><strong>Note</strong> first</p>", "bold at the start of a line is not a bullet");
});
