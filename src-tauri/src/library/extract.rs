//! Metadata of one library file from its structure: title, size in pages, existing fields or tags, blanks.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::mcp::find::clip;
use crate::render::{pdf_form, preview};
use crate::templates::docx;

/// Places a value goes in a printed or typed form: dot or underscore runs, empty brackets, «mail merge» names.
static BLANK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.{4,}|…{2,}|_{3,}|\[\s{2,}\]|\(\s{3,}\)|«[^»]{1,40}»").unwrap());
static TITLE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<dc:title>(.*?)</dc:title>").unwrap());
const TITLE_MAX: usize = 120;
/// Pages scanned for printed blanks in a PDF; enough to tell a form from a letter.
const PDF_PAGES: usize = 3;

pub const READY: &str = "ready";
pub const BLANKS: &str = "blanks";
pub const NEEDS_AI: &str = "needs_ai";
pub const SCAN: &str = "scan";

#[derive(Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub pages: Option<i64>,
    /// AcroForm fields of a PDF, {{ }} tags of a docx
    pub fields: i64,
    pub blanks: i64,
    /// ready | blanks | needs_ai | scan
    pub readiness: &'static str,
}

pub fn kind_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_lowercase().as_str() {
        "docx" => Some("docx"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

pub fn extract(path: &Path, kind: &str) -> Result<Meta, String> {
    match kind {
        "docx" => docx_meta(path),
        "pdf" => pdf_meta(&std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?),
        other => Err(format!("Not a library kind: {other}")),
    }
}

fn readiness(fields: i64, blanks: i64) -> &'static str {
    if fields > 0 {
        READY
    } else if blanks > 0 {
        BLANKS
    } else {
        NEEDS_AI
    }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn docx_meta(path: &Path) -> Result<Meta, String> {
    let paragraphs = docx::inspect(path)?;
    let fields = docx::variables(path)?.len() as i64;
    let blanks: usize = paragraphs.iter().map(|p| BLANK_RE.find_iter(&p.text).count()).sum();
    let title = docx::read_part(path, "docProps/core.xml")
        .ok()
        .and_then(|xml| TITLE_RE.captures(&xml).map(|c| unescape(c[1].trim())))
        .filter(|t| !t.is_empty())
        .or_else(|| paragraphs.iter().map(|p| p.text.trim()).find(|t| !t.is_empty()).map(str::to_string))
        .unwrap_or_default();
    Ok(Meta {
        title: clip(&title, TITLE_MAX),
        pages: None,
        fields,
        blanks: blanks as i64,
        readiness: readiness(fields, blanks as i64),
    })
}

fn pdf_meta(pdf: &[u8]) -> Result<Meta, String> {
    let pages = preview::page_count(pdf)?;
    let fields = pdf_form::inspect_form(pdf).map(|f| f.len()).unwrap_or(0) as i64;
    let mut lines = Vec::new();
    for page in 1..=pages.min(PDF_PAGES) {
        lines.extend(preview::layout_lines(pdf, page, 2000)?);
    }
    let blanks: usize = lines.iter().map(|l| BLANK_RE.find_iter(&l.text).count()).sum();
    let title = lines.iter().map(|l| l.text.trim()).find(|t| !t.is_empty()).unwrap_or_default();
    let readiness = if fields == 0 && lines.is_empty() { SCAN } else { readiness(fields, blanks as i64) };
    Ok(Meta { title: clip(title, TITLE_MAX), pages: Some(pages as i64), fields, blanks: blanks as i64, readiness })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)
    }

    #[test]
    fn filled_docx_has_blanks() {
        let meta = extract(&fixture("filled.docx"), "docx").unwrap();
        assert_eq!(meta.readiness, BLANKS, "{meta:?}");
        assert!(meta.blanks > 0 && meta.fields == 0);
        assert!(!meta.title.is_empty());
    }

    #[test]
    fn tagged_docx_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t.docx");
        let replacements = serde_json::json!([{"find": "John Smith", "var": "manager_name"}]);
        docx::tokenize(&fixture("filled.docx"), &target, replacements.as_array().unwrap()).unwrap();
        let meta = extract(&target, "docx").unwrap();
        assert_eq!((meta.readiness, meta.fields), (READY, 1));
    }

    #[test]
    fn pdf_form_is_ready_and_flat_pdf_is_not() {
        let form = extract(&fixture("form.pdf"), "pdf").unwrap();
        assert_eq!(form.readiness, READY);
        assert!(form.fields > 0 && form.pages == Some(1));
        let flat = extract(&fixture("flat.pdf"), "pdf").unwrap();
        assert_ne!(flat.readiness, READY);
        assert_ne!(flat.readiness, SCAN);
    }

    #[test]
    fn blank_patterns() {
        let count = |t: &str| BLANK_RE.find_iter(t).count();
        assert_eq!(count("Name: ........ EGN: ____ Date: [    ] «City»"), 4);
        assert_eq!(count("Price 1.000 lv. (VAT incl.)"), 0);
    }
}
