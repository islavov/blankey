//! Filled .docx documents: turn them into templates ({{ var }} tags in runs), find their blanks, preview them as
//! HTML and render them. Edits are byte-range replacements on the XML, so everything else stays untouched.

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use roxmltree::{Document, Node};
use serde::Serialize;
use serde_json::{json, Value};

use super::bindings;

pub const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const BODY_PART: &str = "word/document.xml";
const BLANK: &str = "[____]";

pub static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z_]\w*)[^}]*\}\}").unwrap());
static VAR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_]\w*$").unwrap());
static ANY_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)\{\{.*?\}\}").unwrap());

#[derive(Serialize, Debug, Clone)]
pub struct ParagraphInfo {
    pub index: usize,
    /// "body" or "table T row R cell C"
    pub location: String,
    pub text: String,
    pub runs: Vec<String>,
    pub tags: Vec<String>,
    /// Byte range of each run in `text`; hyperlink text between runs is in no range.
    #[serde(skip)]
    pub run_spans: Vec<(usize, usize)>,
}

// -- reading the package -------------------------------------------------------------------------------

pub fn read_part(path: &Path, name: &str) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: not a .docx ({e})", path.display()))?;
    let mut entry = archive.by_name(name).map_err(|_| format!("{}: no {name}", path.display()))?;
    let mut text = String::new();
    entry.read_to_string(&mut text).map_err(|e| e.to_string())?;
    Ok(text)
}

/// Copies the package with some parts replaced.
fn write_package(source: &[u8], replaced: &HashMap<String, String>) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(source)).map_err(|e| e.to_string())?;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|e| e.to_string())?;
        match replaced.get(entry.name()) {
            Some(xml) => {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .last_modified_time(entry.last_modified().unwrap_or_default());
                let name = entry.name().to_string();
                drop(entry);
                writer.start_file(name, options).map_err(|e| e.to_string())?;
                writer.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
            }
            None => writer.raw_copy_file(entry).map_err(|e| e.to_string())?,
        }
    }
    Ok(writer.finish().map_err(|e| e.to_string())?.into_inner())
}

fn parse(xml: &str) -> Result<Document<'_>, String> {
    Document::parse_with_options(xml, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| format!("Invalid document XML: {e}"))
}

fn is_w(node: &Node, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(W)
}

fn children<'a, 'i>(node: Node<'a, 'i>, name: &'static str) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children().filter(move |c| is_w(c, name))
}

fn attr(node: &Node, name: &str) -> Option<String> {
    node.attributes().find(|a| a.name() == name && a.namespace() == Some(W)).map(|a| a.value().to_string())
}

/// Text of a run as python-docx reads it: w:t text, tabs as \t, breaks as \n.
fn run_text(run: Node) -> String {
    let mut text = String::new();
    for child in run.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(W)) {
        match child.tag_name().name() {
            "t" => text.push_str(child.text().unwrap_or("")),
            "tab" | "ptab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            "noBreakHyphen" => text.push('-'),
            _ => {}
        }
    }
    text
}

fn is_text_child(node: &Node) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(W)
        && matches!(node.tag_name().name(), "t" | "tab" | "ptab" | "br" | "cr" | "noBreakHyphen")
}

struct Run {
    node: roxmltree::NodeId,
    text: String,
    changed: bool,
}

enum Segment {
    Run(usize),
    Fixed(String),
}

struct Para {
    location: String,
    segments: Vec<Segment>,
    /// direct runs, as indexes into the run list
    runs: Vec<usize>,
}

struct Body {
    paragraphs: Vec<Para>,
    runs: Vec<Run>,
}

impl Body {
    fn text(&self, p: &Para) -> String {
        p.segments
            .iter()
            .map(|s| match s {
                Segment::Run(i) => self.runs[*i].text.as_str(),
                Segment::Fixed(t) => t.as_str(),
            })
            .collect()
    }
}

/// Where a run's content (everything after w:rPr) sits in the XML; None for an empty run.
fn content_range(run: Node, xml: &str) -> Option<Range<usize>> {
    run.first_child()?;
    let range = run.range();
    let element = &xml[range.clone()];
    let close = range.start + element.rfind("</")?;
    let start = match children(run, "rPr").next() {
        Some(rpr) => rpr.range().end,
        None => range.start + element.find('>')? + 1,
    };
    Some(start..close)
}

/// Paragraphs in python-docx order: body paragraphs, then the paragraphs of body tables cell by cell.
fn read_body(doc: &Document) -> Body {
    let mut body = Body { paragraphs: Vec::new(), runs: Vec::new() };
    let Some(root) = doc.root_element().children().find(|c| is_w(c, "body")) else { return body };
    let add = |p: Node, location: String, body: &mut Body| {
        let mut para = Para { location, segments: Vec::new(), runs: Vec::new() };
        for child in p.children() {
            if is_w(&child, "r") {
                let index = body.runs.len();
                body.runs.push(Run { node: child.id(), text: run_text(child), changed: false });
                para.runs.push(index);
                para.segments.push(Segment::Run(index));
            } else if is_w(&child, "hyperlink") {
                let text: String = children(child, "r").map(run_text).collect();
                para.segments.push(Segment::Fixed(text));
            }
        }
        body.paragraphs.push(para);
    };
    for p in children(root, "p") {
        add(p, "body".into(), &mut body);
    }
    for (t, table) in children(root, "tbl").enumerate() {
        for (r, row) in children(table, "tr").enumerate() {
            for (c, cell) in children(row, "tc").enumerate() {
                for p in children(cell, "p") {
                    add(p, format!("table {t} row {r} cell {c}"), &mut body);
                }
            }
        }
    }
    body
}

fn with_body<T>(path: &Path, f: impl FnOnce(&Body) -> T) -> Result<T, String> {
    let xml = read_part(path, BODY_PART)?;
    let doc = parse(&xml)?;
    Ok(f(&read_body(&doc)))
}

pub fn inspect(path: &Path) -> Result<Vec<ParagraphInfo>, String> {
    with_body(path, |body| {
        body.paragraphs
            .iter()
            .enumerate()
            .filter_map(|(index, p)| {
                let text = body.text(p);
                if text.trim().is_empty() {
                    return None;
                }
                let tags = TAG_RE.captures_iter(&text).map(|c| c[1].to_string()).collect();
                let runs = p.runs.iter().map(|&i| body.runs[i].text.clone()).collect();
                let mut run_spans = Vec::new();
                let mut at = 0;
                for segment in &p.segments {
                    let len = match segment {
                        Segment::Run(i) => body.runs[*i].text.len(),
                        Segment::Fixed(t) => t.len(),
                    };
                    if let Segment::Run(_) = segment {
                        run_spans.push((at, at + len));
                    }
                    at += len;
                }
                Some(ParagraphInfo { index, location: p.location.clone(), text, runs, tags, run_spans })
            })
            .collect()
    })
}

/// Tag names in document order, without duplicates.
pub fn variables(path: &Path) -> Result<Vec<String>, String> {
    with_body(path, |body| {
        let mut found: Vec<String> = Vec::new();
        for p in &body.paragraphs {
            for c in TAG_RE.captures_iter(&body.text(p)) {
                if !found.iter().any(|f| f == &c[1]) {
                    found.push(c[1].to_string());
                }
            }
        }
        found
    })
}

/// For each tag: the surrounding paragraph text per occurrence, the tag shown as [____] and other tags as
/// [name], trimmed to about `width` characters around the blank.
pub fn blank_contexts(path: &Path, width: usize) -> Result<IndexMap<String, Vec<String>>, String> {
    with_body(path, |body| {
        let mut contexts: IndexMap<String, Vec<String>> = IndexMap::new();
        let half = width.saturating_sub(BLANK.chars().count()) / 2;
        let others = |text: &str| TAG_RE.replace_all(text, "[$1]").into_owned();
        for p in &body.paragraphs {
            let text = body.text(p);
            for m in TAG_RE.captures_iter(&text) {
                let whole = m.get(0).unwrap();
                let mut before = others(&text[..whole.start()]);
                let mut after = others(&text[whole.end()..]);
                if before.chars().count() > half {
                    let skip = before.chars().count() - half;
                    before = format!("…{}", before.chars().skip(skip).collect::<String>());
                }
                if after.chars().count() > half {
                    after = format!("{}…", after.chars().take(half).collect::<String>());
                }
                let joined = format!("{before}{BLANK}{after}");
                let normalized = joined.split_whitespace().collect::<Vec<_>>().join(" ");
                contexts.entry(m[1].to_string()).or_default().push(normalized);
            }
        }
        contexts
    })
}

// -- writing -------------------------------------------------------------------------------------------

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Run content as python-docx writes it: text in w:t, tabs as w:tab, line breaks as w:br.
fn run_xml(prefix: &str, text: &str) -> String {
    let mut out = String::new();
    let mut chunk = String::new();
    let flush = |chunk: &mut String, out: &mut String| {
        if !chunk.is_empty() {
            out.push_str(&format!("<{prefix}t xml:space=\"preserve\">{}</{prefix}t>", escape(chunk)));
            chunk.clear();
        }
    };
    for ch in text.chars() {
        match ch {
            '\t' => {
                flush(&mut chunk, &mut out);
                out.push_str(&format!("<{prefix}tab/>"));
            }
            '\n' | '\r' => {
                flush(&mut chunk, &mut out);
                out.push_str(&format!("<{prefix}br/>"));
            }
            _ => chunk.push(ch),
        }
    }
    flush(&mut chunk, &mut out);
    out
}

fn apply_edits(xml: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut out = xml.to_string();
    for (range, text) in edits {
        out.replace_range(range, &text);
    }
    out
}

/// The "w:" in "<w:r>", whatever the document calls the namespace.
fn w_prefix(xml: &str, doc: &Document) -> String {
    let root = doc.root_element();
    root.namespaces()
        .find(|ns| ns.uri() == W)
        .and_then(|ns| ns.name())
        .map(|name| format!("{name}:"))
        .unwrap_or_else(|| if xml.contains("<w:") { "w:".into() } else { String::new() })
}

/// Replaces text spans with {{ var }} tags. Each replacement: {find, var, occurrence?} with occurrence 1-based
/// over the whole document (omit to replace all). A span must sit inside one run. Returns a report line per
/// replacement; fails listing every replacement that did not apply.
pub fn tokenize(source: &Path, target: &Path, replacements: &[Value]) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    let xml = read_part(source, BODY_PART)?;
    let doc = parse(&xml)?;
    let mut body = read_body(&doc);
    let original: Vec<String> = body.paragraphs.iter().map(|p| body.text(p)).collect();
    let original = original.join("\n");
    let (mut report, mut errors) = (Vec::new(), Vec::new());
    for item in replacements {
        let find = item.get("find").and_then(Value::as_str).unwrap_or("");
        let var = item.get("var").and_then(Value::as_str).unwrap_or("");
        let occurrence = item.get("occurrence").and_then(Value::as_u64).map(|o| o as usize);
        if find.is_empty() || !VAR_RE.is_match(var) {
            errors.push(format!("{item}: needs non-empty 'find' and an identifier 'var'"));
            continue;
        }
        let tag = format!("{{{{ {var} }}}}");
        let (mut seen, mut replaced) = (0, 0);
        for p in 0..body.paragraphs.len() {
            for &r in &body.paragraphs[p].runs.clone() {
                let text = body.runs[r].text.clone();
                let (mut parts, mut start) = (String::new(), 0);
                let mut touched = false;
                while let Some(pos) = text[start..].find(find).map(|p| p + start) {
                    seen += 1;
                    touched = true;
                    if occurrence.is_none() || occurrence == Some(seen) {
                        parts.push_str(&text[start..pos]);
                        parts.push_str(&tag);
                        replaced += 1;
                    } else {
                        parts.push_str(&text[start..pos + find.len()]);
                    }
                    start = pos + find.len();
                }
                if touched {
                    parts.push_str(&text[start..]);
                    if parts != text {
                        body.runs[r].text = parts;
                        body.runs[r].changed = true;
                    }
                }
            }
        }
        if replaced > 0 {
            report.push(format!("{var}: replaced {replaced}x {find:?}"));
            continue;
        }
        let split: Vec<usize> =
            (0..body.paragraphs.len()).filter(|&i| body.text(&body.paragraphs[i]).contains(find)).collect();
        if let (Some(wanted), true) = (occurrence, seen > 0) {
            errors.push(format!("{var}: {find:?} occurs {seen}x in runs, no occurrence {wanted}"));
        } else if !split.is_empty() {
            errors.push(format!("{var}: {find:?} spans several runs (formatting change) in paragraph(s) {split:?}"));
        } else if original.contains(find) {
            errors.push(format!("{var}: {find:?} was consumed by an earlier replacement; reorder or use occurrence"));
        } else {
            errors.push(format!("{var}: {find:?} not found"));
        }
    }
    if !errors.is_empty() {
        return Err(format!("Tokenizing failed:\n{}", errors.join("\n")));
    }
    let prefix = w_prefix(&xml, &doc);
    let edits = rewrite_runs(&xml, &doc, &body, &prefix);
    let updated = apply_edits(&xml, edits);
    let package = write_package(&bytes, &HashMap::from([(BODY_PART.to_string(), updated)]))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(target, package).map_err(|e| e.to_string())?;
    Ok(report)
}

/// Edits for changed runs: their text children are replaced by the new text, other children stay.
fn rewrite_runs(xml: &str, doc: &Document, body: &Body, prefix: &str) -> Vec<(Range<usize>, String)> {
    let mut edits = Vec::new();
    for run in body.runs.iter().filter(|r| r.changed) {
        let node = doc.get_node(run.node).expect("run is in the document");
        let texts: Vec<Node> = node.children().filter(is_text_child).collect();
        match (texts.first(), content_range(node, xml)) {
            (Some(first), _) => {
                edits.push((first.range(), run_xml(prefix, &run.text)));
                for other in &texts[1..] {
                    edits.push((other.range(), String::new()));
                }
            }
            (None, Some(content)) => edits.push((content.start..content.start, run_xml(prefix, &run.text))),
            (None, None) => {}
        }
    }
    edits
}

/// Renders every {{ ... }} tag in the body, headers and footers with `values` (var -> text). A tag split over
/// several w:t elements of one paragraph is rendered into the first of them.
pub fn render(source: &[u8], values: &IndexMap<String, String>) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(source)).map_err(|e| format!("Not a .docx: {e}"))?;
    let context = Value::Object(values.iter().map(|(k, v)| (k.clone(), json!(v))).collect());
    let mut replaced = HashMap::new();
    let names: Vec<String> = archive.file_names().map(str::to_string).collect();
    for name in names {
        let part = name == BODY_PART
            || (name.starts_with("word/header") || name.starts_with("word/footer")) && name.ends_with(".xml");
        if !part {
            continue;
        }
        let mut xml = String::new();
        archive.by_name(&name).map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
        if !xml.contains("{{") {
            continue;
        }
        let doc = parse(&xml)?;
        let prefix = w_prefix(&xml, &doc);
        let mut edits = Vec::new();
        for p in doc.descendants().filter(|n| is_w(n, "p")) {
            let texts: Vec<Node> =
                p.descendants().filter(|n| is_w(n, "t") && n.ancestors().find(|a| is_w(a, "p")) == Some(p)).collect();
            let pieces: Vec<&str> = texts.iter().map(|t| t.text().unwrap_or("")).collect();
            let joined: String = pieces.concat();
            if !ANY_TAG.is_match(&joined) {
                continue;
            }
            let rendered = render_texts(&pieces, &context)?;
            for (node, (old, new)) in texts.iter().zip(pieces.iter().zip(rendered)) {
                if *old != new {
                    edits.push((
                        node.range(),
                        format!("<{prefix}t xml:space=\"preserve\">{}</{prefix}t>", escape(&new)),
                    ));
                }
            }
        }
        replaced.insert(name, apply_edits(&xml, edits));
    }
    write_package(source, &replaced)
}

/// The texts of one paragraph's w:t elements after rendering the tags in their concatenation.
fn render_texts(pieces: &[&str], context: &Value) -> Result<Vec<String>, String> {
    let joined: String = pieces.concat();
    let mut owner = Vec::with_capacity(joined.len());
    for (i, piece) in pieces.iter().enumerate() {
        owner.extend(std::iter::repeat_n(i, piece.len()));
    }
    let mut out = vec![String::new(); pieces.len()];
    let mut cursor = 0;
    for m in ANY_TAG.find_iter(&joined) {
        for (offset, ch) in joined[cursor..m.start()].char_indices() {
            out[owner[cursor + offset]].push(ch);
        }
        let value = bindings::evaluate(m.as_str(), context).map_err(|e| format!("{}: {e}", m.as_str()))?;
        out[owner[m.start()]].push_str(&bindings::as_text(&value));
        cursor = m.end();
    }
    for (offset, ch) in joined[cursor..].char_indices() {
        out[owner[cursor + offset]].push(ch);
    }
    Ok(out)
}

pub fn render_file(path: &Path, values: &IndexMap<String, String>) -> Result<Vec<u8>, String> {
    render(&std::fs::read(path).map_err(|e| e.to_string())?, values)
}

/// LibreOffice converts the rendered .docx; None when it is not installed.
pub fn to_pdf(docx: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let installed =
        ["/Applications/LibreOffice.app/Contents/MacOS/soffice", r"C:\Program Files\LibreOffice\program\soffice.exe"];
    let soffice = ["soffice", "libreoffice"]
        .iter()
        .find_map(|name| which(name))
        .or_else(|| installed.iter().map(Path::new).find(|p| p.exists()).map(Path::to_path_buf));
    let Some(soffice) = soffice else { return Ok(None) };
    let dir = std::env::temp_dir().join(format!("blankey-docx-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let input = dir.join("document.docx");
    std::fs::write(&input, docx).map_err(|e| e.to_string())?;
    let status = std::process::Command::new(soffice)
        .args(["--headless", "--convert-to", "pdf", "--outdir"])
        .arg(&dir)
        .arg(&input)
        .output()
        .map_err(|e| e.to_string());
    let pdf = std::fs::read(dir.join("document.pdf"));
    let _ = std::fs::remove_dir_all(&dir);
    status?;
    pdf.map(Some).map_err(|e| format!("LibreOffice did not produce a PDF: {e}"))
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::split_paths(&std::env::var_os("PATH")?).map(|dir| dir.join(&file)).find(|p| p.is_file())
}

// -- HTML preview --------------------------------------------------------------------------------------

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A blank as a chip with data-var, so a preview can highlight it and react to clicks. It shows the value when one
/// is given, else the label (class "empty").
fn chip(var: &str, labels: &IndexMap<String, String>, values: Option<&IndexMap<String, String>>) -> String {
    let value = values.and_then(|v| v.get(var)).map(|v| v.trim()).unwrap_or("");
    let shown = if !value.is_empty() {
        value
    } else {
        labels.get(var).map(String::as_str).filter(|l| !l.is_empty()).unwrap_or(var)
    };
    let empty = if value.is_empty() && values.is_some() { " is-empty" } else { "" };
    format!(
        "<span class=\"blank b-{var}{empty}\" data-var=\"{var}\" id=\"blank-{var}\">&nbsp;{}&nbsp;</span>",
        html_escape(shown)
    )
}

fn on(node: Option<Node>) -> bool {
    node.is_some_and(|n| !matches!(attr(&n, "val").as_deref(), Some("0" | "false" | "off" | "none")))
}

/// Renders the text of one run instead of the default: (paragraph index as `inspect` numbers it, run index within
/// the paragraph, run text) -> HTML, or None for the default.
pub type RunHook<'a> = &'a dyn Fn(usize, usize, &str) -> Option<String>;

struct HtmlContext<'a> {
    labels: &'a IndexMap<String, String>,
    values: Option<&'a IndexMap<String, String>>,
    hook: Option<RunHook<'a>>,
}

fn run_html(run: Node, ctx: &HtmlContext, at: (usize, usize)) -> String {
    let raw = run_text(run);
    let mut text = match ctx.hook.and_then(|hook| hook(at.0, at.1, &raw)) {
        Some(html) => html,
        None => TAG_RE
            .replace_all(&html_escape(&raw), |c: &regex::Captures| chip(&c[1], ctx.labels, ctx.values))
            .into_owned(),
    };
    let props = children(run, "rPr").next();
    let prop = |name: &'static str| props.and_then(|p| children(p, name).next());
    if on(prop("b")) {
        text = format!("<b>{text}</b>");
    }
    if on(prop("i")) {
        text = format!("<i>{text}</i>");
    }
    if on(prop("u")) {
        text = format!("<u>{text}</u>");
    }
    if let Some(size) = prop("sz").and_then(|s| attr(&s, "val")).and_then(|v| v.parse::<f32>().ok()) {
        text = format!("<span style=\"font-size:{}pt\">{text}</span>", size / 2.0);
    }
    text
}

fn paragraph_html(p: Node, ctx: &HtmlContext, index: usize) -> String {
    let mut body: String = children(p, "r").enumerate().map(|(r, run)| run_html(run, ctx, (index, r))).collect();
    if body.is_empty() {
        body = "&nbsp;".into();
    }
    let props = children(p, "pPr").next();
    let align = props.and_then(|pp| children(pp, "jc").next()).and_then(|jc| attr(&jc, "val"));
    let align = match align.as_deref() {
        Some("center") => Some("center"),
        Some("right" | "end") => Some("right"),
        Some("both" | "distribute") => Some("justify"),
        _ => None,
    };
    let style = props.and_then(|pp| children(pp, "pStyle").next()).and_then(|s| attr(&s, "val")).unwrap_or_default();
    if style.starts_with("Heading") || style.starts_with("Title") {
        body = format!("<span class=\"heading\">{body}</span>");
    }
    match align {
        Some(a) => format!("<p style=\"text-align:{a}\">{body}</p>"),
        None => format!("<p>{body}</p>"),
    }
}

/// The document body as simple HTML for a read-only preview; {{ var }} tags become chips.
pub fn to_html(
    path: &Path,
    labels: &IndexMap<String, String>,
    values: Option<&IndexMap<String, String>>,
) -> Result<String, String> {
    body_html(path, &HtmlContext { labels, values, hook: None })
}

/// The preview with each run's text rendered by `hook`.
pub fn to_html_with(path: &Path, hook: RunHook) -> Result<String, String> {
    body_html(path, &HtmlContext { labels: &IndexMap::new(), values: None, hook: Some(hook) })
}

/// In document order, with each paragraph numbered as `read_body` numbers it: body paragraphs first, then the
/// paragraphs of tables.
fn body_html(path: &Path, ctx: &HtmlContext) -> Result<String, String> {
    let xml = read_part(path, BODY_PART)?;
    let doc = parse(&xml)?;
    let Some(body) = doc.root_element().children().find(|c| is_w(c, "body")) else { return Ok(String::new()) };
    let (mut in_body, mut in_tables) = (0, children(body, "p").count());
    let mut parts = Vec::new();
    for child in body.children() {
        if is_w(&child, "p") {
            parts.push(paragraph_html(child, ctx, in_body));
            in_body += 1;
        } else if is_w(&child, "tbl") {
            let mut rows = String::new();
            for row in children(child, "tr") {
                let mut cells = String::new();
                for cell in children(row, "tc") {
                    let mut inner = String::new();
                    for p in children(cell, "p") {
                        inner.push_str(&paragraph_html(p, ctx, in_tables));
                        in_tables += 1;
                    }
                    cells.push_str(&format!("<td>{inner}</td>"));
                }
                rows.push_str(&format!("<tr>{cells}</tr>"));
            }
            parts.push(format!("<table>{rows}</table>"));
        }
    }
    Ok(parts.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/filled.docx")
    }

    fn tokenized(dir: &Path) -> std::path::PathBuf {
        let target = dir.join("template.docx");
        let replacements = serde_json::json!([
            {"find": "...........2026", "var": "contract_year"},
            {"find": ".........", "var": "contract_no", "occurrence": 1},
            {"find": "John Smith", "var": "manager_name"},
            {"find": "34 000", "var": "amount"},
        ]);
        let report = tokenize(&fixture(), &target, replacements.as_array().unwrap()).unwrap();
        assert_eq!(report.len(), 4, "{report:?}");
        target
    }

    #[test]
    fn inspect_lists_runs() {
        let paragraphs = inspect(&fixture()).unwrap();
        assert_eq!(paragraphs[0].text, "Contract No ......... / ...........2026");
        assert_eq!(paragraphs[1].runs[0], "1. “ACME HOLDINGS” LTD");
        assert!(paragraphs.iter().any(|p| p.location == "table 0 row 0 cell 1" && p.text == "John Smith"));
    }

    #[test]
    fn tokenize_and_variables() {
        let dir = tempfile::tempdir().unwrap();
        let target = tokenized(dir.path());
        assert_eq!(variables(&target).unwrap(), ["contract_no", "contract_year", "manager_name", "amount"]);
        let contexts = blank_contexts(&target, 400).unwrap();
        assert_eq!(contexts["manager_name"].len(), 2);
        assert!(contexts["amount"][0].contains("Loan in the amount of [____] EUR."));
        let text = inspect(&target).unwrap()[0].text.clone();
        assert_eq!(text, "Contract No {{ contract_no }} / {{ contract_year }}");
    }

    #[test]
    fn tokenize_errors() {
        let dir = tempfile::tempdir().unwrap();
        let bad = serde_json::json!([
            {"find": "nowhere", "var": "a"},
            {"find": "LTD, Company", "var": "b"},
            {"find": "John Smith", "var": "c", "occurrence": 5},
            {"find": "x", "var": "not valid"},
        ]);
        let error = tokenize(&fixture(), &dir.path().join("t.docx"), bad.as_array().unwrap()).unwrap_err();
        assert!(error.contains("a: \"nowhere\" not found"), "{error}");
        assert!(error.contains("b: \"LTD, Company\" spans several runs"), "{error}");
        assert!(error.contains("c: \"John Smith\" occurs 2x in runs, no occurrence 5"), "{error}");
        assert!(error.contains("identifier 'var'"), "{error}");
    }

    #[test]
    fn render_fills_tags_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let target = tokenized(dir.path());
        let values: IndexMap<String, String> = [
            ("contract_no", "17"),
            ("contract_year", "01.02.2026"),
            ("manager_name", "Иван & <Петров>"),
            ("amount", "50 000"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let rendered = render_file(&target, &values).unwrap();
        let out = dir.path().join("out.docx");
        std::fs::write(&out, rendered).unwrap();
        let paragraphs = inspect(&out).unwrap();
        assert_eq!(paragraphs[0].text, "Contract No 17 / 01.02.2026");
        assert!(paragraphs.iter().any(|p| p.text == "Иван & <Петров>"));
        assert!(paragraphs.iter().any(|p| p.text == "Loan in the amount of 50 000 EUR."));
        assert_eq!(paragraphs[1].runs[0], "1. “ACME HOLDINGS” LTD");
    }

    #[test]
    fn tags_split_across_text_elements_render_once() {
        let pieces = ["Name: {{ na", "me }}!", " end"];
        let context = serde_json::json!({"name": "Ана"});
        assert_eq!(render_texts(&pieces, &context).unwrap(), ["Name: Ана", "!", " end"]);
    }

    #[test]
    fn html_shows_blanks_as_chips() {
        let dir = tempfile::tempdir().unwrap();
        let target = tokenized(dir.path());
        let labels: IndexMap<String, String> =
            [("amount".to_string(), "Loan amount".to_string())].into_iter().collect();
        let html = to_html(&target, &labels, None).unwrap();
        assert!(html.contains("data-var=\"amount\"") && html.contains("&nbsp;Loan amount&nbsp;"));
        assert!(html.contains("<b>1. “ACME HOLDINGS” LTD</b>"));
        assert!(html.contains("<table>"));
        let values: IndexMap<String, String> = [("amount".to_string(), "1 000".to_string())].into_iter().collect();
        let filled = to_html(&target, &labels, Some(&values)).unwrap();
        assert!(filled.contains("&nbsp;1 000&nbsp;") && filled.contains("b-manager_name is-empty"));
    }
}
