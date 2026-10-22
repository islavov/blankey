//! Previews of a template, empty or with values: a docx as HTML with its blanks as chips, PDF forms and Typst
//! documents as page images with the rectangle of each field, so the UI can highlight and click them.

use base64::Engine;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use super::bindings::{as_text, is_empty};
use super::engine::{self, Values};
use super::{docx, Template, TemplateKind};
use crate::render::pdf_form;
use crate::render::preview::{self as pdf, Place};

pub const DPI: f32 = 110.0;
/// Pages shown for a library file; enough to recognise the document.
pub const FILE_PAGES: usize = 10;

#[derive(Serialize)]
pub struct PagePreview {
    /// data: URL of the PNG
    pub image: String,
    /// in PDF points
    pub width: f32,
    pub height: f32,
    /// field -> rectangles [x0, y0, x1, y1] in points, origin bottom-left
    pub boxes: IndexMap<String, Vec<[f64; 4]>>,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Preview {
    Html { html: String, names: Vec<String> },
    Pages { pages: Vec<PagePreview>, names: Vec<String> },
    Error { message: String },
}

pub fn preview(template: &Template, values: Option<&Values>) -> Preview {
    let result = match template.kind {
        TemplateKind::Docx => docx_preview(template, values),
        TemplateKind::PdfForm => form_preview(template, values),
        TemplateKind::Typst => typst_preview(template, values),
    };
    result.unwrap_or_else(|message| Preview::Error { message })
}

/// A library file as it is: a docx as HTML (existing {{ }} tags as chips), a PDF as its first pages with any form
/// fields outlined.
pub fn file_preview(path: &std::path::Path, kind: &str) -> Preview {
    let result = match kind {
        "docx" => docx::to_html(path, &IndexMap::new(), None)
            .and_then(|html| Ok(Preview::Html { html, names: docx::variables(path)? })),
        _ => std::fs::read(path).map_err(|e| e.to_string()).and_then(|pdf_bytes| {
            let mut boxes: IndexMap<String, Vec<Place>> = IndexMap::new();
            for field in pdf_form::inspect_form(&pdf_bytes).unwrap_or_default() {
                boxes.entry(field.name.clone()).or_default().extend(field.rects.iter().map(|r| (field.page, *r)));
            }
            let names = boxes.keys().cloned().collect();
            Ok(Preview::Pages { pages: pages(&pdf_bytes, &boxes, FILE_PAGES)?, names })
        }),
    };
    result.unwrap_or_else(|message| Preview::Error { message })
}

fn texts(values: &Values) -> IndexMap<String, String> {
    values.iter().map(|(k, v)| (k.clone(), as_text(v))).collect()
}

fn docx_preview(template: &Template, values: Option<&Values>) -> Result<Preview, String> {
    let path = template.source_path();
    let shown = values.map(texts);
    Ok(Preview::Html { html: docx::to_html(&path, &template.labels, shown.as_ref())?, names: docx::variables(&path)? })
}

/// The first `limit` pages as images with the given boxes.
pub fn pages(pdf_bytes: &[u8], boxes: &IndexMap<String, Vec<Place>>, limit: usize) -> Result<Vec<PagePreview>, String> {
    let sizes = pdf::page_sizes(pdf_bytes)?;
    let shown: Vec<usize> = (1..=sizes.len().min(limit)).collect();
    let images = pdf::render_pages(pdf_bytes, &shown, DPI)?;
    let encoder = base64::engine::general_purpose::STANDARD;
    Ok(images
        .into_iter()
        .enumerate()
        .map(|(index, png)| {
            let mut page_boxes: IndexMap<String, Vec<[f64; 4]>> = IndexMap::new();
            for (name, places) in boxes {
                for (page, rect) in places {
                    if *page == index + 1 {
                        page_boxes.entry(name.clone()).or_default().push(*rect);
                    }
                }
            }
            PagePreview {
                image: format!("data:image/png;base64,{}", encoder.encode(png)),
                width: sizes[index].0,
                height: sizes[index].1,
                boxes: page_boxes,
            }
        })
        .collect())
}

fn form_preview(template: &Template, values: Option<&Values>) -> Result<Preview, String> {
    let source = template.source().map_err(|e| e.to_string())?;
    let mut boxes: IndexMap<String, Vec<Place>> = IndexMap::new();
    for field in pdf_form::inspect_form(&source)? {
        boxes.entry(field.name.clone()).or_default().extend(field.rects.iter().map(|r| (field.page, *r)));
    }
    let pdf_bytes = match values.filter(|v| !v.is_empty()) {
        Some(values) => engine::render_values(template, values, Vec::new())?.content,
        None => source,
    };
    let names = boxes.keys().cloned().collect();
    Ok(Preview::Pages { pages: pages(&pdf_bytes, &boxes, usize::MAX)?, names })
}

/// Shown in place of empty values: the label in brackets, or the name when labels repeat.
fn markers(template: &Template) -> IndexMap<String, String> {
    let labels: Vec<String> = template
        .fields
        .keys()
        .map(|name| template.labels.get(name).filter(|l| !l.is_empty()).cloned().unwrap_or_else(|| name.clone()))
        .collect();
    template
        .fields
        .keys()
        .zip(&labels)
        .map(|(name, label)| {
            let unique = labels.iter().filter(|l| *l == label).count() == 1;
            (name.clone(), format!("[{}]", if unique { label } else { name }))
        })
        .collect()
}

/// For each anchor (where the marker was), the candidate on the same page closest to it; the anchor itself when
/// there is none (a value broken across lines is not found by text search).
fn nearest(candidates: &[Place], anchors: &[Place]) -> Vec<Place> {
    let mut picks: Vec<Place> = Vec::new();
    for (page, anchor) in anchors {
        let best = candidates
            .iter()
            .filter(|(p, _)| p == page)
            .min_by(|a, b| {
                let score = |c: &Place| (c.1[0] - anchor[0]).abs() + (c.1[3] - anchor[3]).abs() * 3.0;
                score(a).total_cmp(&score(b))
            })
            .copied()
            .unwrap_or((*page, *anchor));
        if !picks.contains(&best) {
            picks.push(best);
        }
    }
    picks
}

fn typst_preview(template: &Template, values: Option<&Values>) -> Result<Preview, String> {
    let markers = markers(template);
    let as_values = |map: &IndexMap<String, String>| -> Values {
        map.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()
    };
    let marker_pdf = engine::render_values(template, &as_values(&markers), Vec::new())?.content;
    let marker_boxes = pdf::find_text(&marker_pdf, &markers)?;
    let Some(values) = values else {
        let names = marker_boxes.keys().cloned().collect();
        return Ok(Preview::Pages { pages: pages(&marker_pdf, &marker_boxes, usize::MAX)?, names });
    };
    // empty values keep their marker; filled values are found near where their marker was
    let shown: IndexMap<String, String> = markers
        .iter()
        .map(|(name, marker)| {
            let value = values.get(name).cloned().unwrap_or(Value::Null);
            (name.clone(), if is_empty(&value) { marker.clone() } else { as_text(&value) })
        })
        .collect();
    let pdf_bytes = engine::render_values(template, &as_values(&shown), Vec::new())?.content;
    let needles: IndexMap<String, String> =
        shown.iter().map(|(k, v)| (k.clone(), v.split_whitespace().collect::<Vec<_>>().join(" "))).collect();
    let found = pdf::find_text(&pdf_bytes, &needles)?;
    let boxes: IndexMap<String, Vec<Place>> = marker_boxes
        .iter()
        .map(|(name, anchors)| (name.clone(), nearest(found.get(name).map_or(&[][..], Vec::as_slice), anchors)))
        .collect();
    let names = boxes.keys().cloned().collect();
    Ok(Preview::Pages { pages: pages(&pdf_bytes, &boxes, usize::MAX)?, names })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::templates::{SaveTemplate, StrMap, TemplateStore};
    use serde_json::json;

    fn typst_template(dir: &std::path::Path) -> Template {
        let store = TemplateStore::new(dir.to_path_buf());
        let fields: StrMap =
            [("buyer", "{{ buyer.name }}"), ("city", "София")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        let labels: StrMap = [("buyer".to_string(), "Купувач".to_string())].into_iter().collect();
        store
            .save(SaveTemplate {
                id: "sale",
                name: "Sale",
                kind: TemplateKind::Typst,
                roles: StrMap::new(),
                fields,
                description: "",
                source_file: None,
                source_bytes: None,
                source_text: Some("#let d = json(bytes(sys.inputs.data))\n#set text(font: \"Noto Sans\")\nГрад: #d.city\n\nКупувач: #d.buyer\n"),
                labels,
                boxes: vec![],
                origin: None,
            })
            .unwrap()
    }

    #[test]
    fn typst_preview_finds_markers_and_values() {
        let dir = tempfile::tempdir().unwrap();
        let template = typst_template(dir.path());
        let Preview::Pages { pages, names } = preview(&template, None) else { panic!("no pages") };
        assert_eq!(names, ["buyer", "city"]);
        assert!(pages[0].boxes.contains_key("buyer") && pages[0].image.starts_with("data:image/png;base64,"));
        let values: Values =
            [("buyer".to_string(), json!("Мария Петрова")), ("city".to_string(), json!(""))].into_iter().collect();
        let Preview::Pages { pages, .. } = preview(&template, Some(&values)) else { panic!("no pages") };
        let buyer = pages[0].boxes["buyer"][0];
        assert!(buyer[2] - buyer[0] > 40.0, "{buyer:?}");
    }

    #[test]
    fn library_files_preview_as_they_are() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
        let Preview::Html { html, names } = file_preview(&data.join("filled.docx"), "docx") else { panic!("no html") };
        assert!(html.contains("John Smith") && names.is_empty());
        let Preview::Pages { pages, names } = file_preview(&data.join("form.pdf"), "pdf") else { panic!("no pages") };
        assert_eq!(pages.len(), 1);
        assert!(names.contains(&"egn".to_string()));
        assert!(matches!(file_preview(&data.join("missing.pdf"), "pdf"), Preview::Error { .. }));
    }

    #[test]
    fn form_preview_has_field_boxes() {
        let dir = tempfile::tempdir().unwrap();
        let store = TemplateStore::new(dir.path().to_path_buf());
        let template = store
            .save(SaveTemplate {
                id: "form",
                name: "Form",
                kind: TemplateKind::PdfForm,
                roles: StrMap::new(),
                fields: [("name".to_string(), "x".to_string())].into_iter().collect(),
                description: "",
                source_file: Some(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/form.pdf")),
                source_bytes: None,
                source_text: None,
                labels: StrMap::new(),
                boxes: vec![],
                origin: None,
            })
            .unwrap();
        let values: Values = [("name".to_string(), json!("Ана"))].into_iter().collect();
        let Preview::Pages { pages, names } = preview(&template, Some(&values)) else { panic!("no pages") };
        assert!(names.contains(&"egn".to_string()));
        assert_eq!(pages[0].boxes["name"], [[100.0, 780.0, 400.0, 798.0]]);
    }
}
