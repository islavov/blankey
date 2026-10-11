//! PDFium: page images (form widgets included), printed text with positions, and text search.

use std::path::PathBuf;
use std::sync::OnceLock;

use indexmap::IndexMap;
use pdfium_render::prelude::*;
use serde::Serialize;

use super::pdf_form::FormField;
use super::raster;

pub const DEFAULT_DPI: f32 = 80.0;
const GRID_STEP: usize = 50;
const GRID_LABEL_STEP: usize = 100;
const RED: (u8, u8, u8) = (220, 30, 30);

/// (1-based page, [x0, y0, x1, y1] in points, origin bottom-left)
pub type Place = (usize, [f64; 4]);

fn library_candidates() -> Vec<PathBuf> {
    let name = Pdfium::pdfium_platform_library_name();
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("BLANKEY_PDFIUM") {
        candidates.push(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(&name));
            candidates.push(dir.join("../Frameworks").join(&name));
            candidates.push(dir.join("../Resources").join(&name));
            candidates.push(dir.join("../lib/blankey").join(&name));
        }
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pdfium").join(&name));
    candidates
}

pub fn pdfium() -> Result<&'static Pdfium, String> {
    static PDFIUM: OnceLock<Result<Pdfium, String>> = OnceLock::new();
    PDFIUM
        .get_or_init(|| {
            for path in library_candidates() {
                if path.exists() {
                    if let Ok(bindings) = Pdfium::bind_to_library(&path) {
                        return Ok(Pdfium::new(bindings));
                    }
                }
            }
            Pdfium::bind_to_system_library()
                .map(Pdfium::new)
                .map_err(|e| format!("PDFium library not found (run scripts/fetch-pdfium.sh): {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn open(pdf: &[u8]) -> Result<PdfDocument<'_>, String> {
    pdfium()?.load_pdf_from_byte_slice(pdf, None).map_err(|e| format!("Cannot open the PDF: {e}"))
}

fn page_of<'a>(doc: &'a PdfDocument, number: usize) -> Result<PdfPage<'a>, String> {
    let count = doc.pages().len() as usize;
    if number == 0 || number > count {
        return Err(format!("page must be 1..{count}"));
    }
    doc.pages().get((number - 1) as i32).map_err(|e| e.to_string())
}

pub fn page_sizes(pdf: &[u8]) -> Result<Vec<(f32, f32)>, String> {
    let doc = open(pdf)?;
    Ok(doc.pages().iter().map(|p| (p.width().value, p.height().value)).collect())
}

pub fn page_count(pdf: &[u8]) -> Result<usize, String> {
    Ok(open(pdf)?.pages().len() as usize)
}

fn render(page: &PdfPage, scale: f32) -> Result<tiny_skia::Pixmap, String> {
    let config = PdfRenderConfig::new().scale_page_by_factor(scale).render_form_data(true);
    let bitmap = page.render_with_config(&config).map_err(|e| e.to_string())?;
    let (width, height) = (bitmap.width() as u32, bitmap.height() as u32);
    raster::pixmap_from_rgba(width, height, bitmap.as_rgba_bytes())
}

/// PNGs of 1-based pages (all when `pages` is empty). Form widgets are drawn as well.
pub fn render_pages(pdf: &[u8], pages: &[usize], dpi: f32) -> Result<Vec<Vec<u8>>, String> {
    let doc = open(pdf)?;
    let selected: Vec<usize> =
        if pages.is_empty() { (1..=doc.pages().len() as usize).collect() } else { pages.to_vec() };
    selected.iter().map(|&n| raster::png(&render(&page_of(&doc, n)?, dpi / 72.0)?)).collect()
}

/// One page with each field's rectangle outlined and labelled with its name.
pub fn annotate_fields(pdf: &[u8], fields: &[FormField], page: usize, dpi: f32) -> Result<Vec<u8>, String> {
    let doc = open(pdf)?;
    let pdf_page = page_of(&doc, page)?;
    let height = pdf_page.height().value;
    let scale = dpi / 72.0;
    let mut image = render(&pdf_page, scale)?;
    for field in fields.iter().filter(|f| f.page == page) {
        for [x0, y0, x1, y1] in &field.rects {
            let (left, top) = (*x0 as f32 * scale, (height - *y1 as f32) * scale);
            let (right, bottom) = (*x1 as f32 * scale, (height - *y0 as f32) * scale);
            raster::outline(&mut image, left, top, right, bottom, (RED.0, RED.1, RED.2, 255));
            raster::text(&mut image, left + 1.0, top - 11.0, &field.name, 10.0, RED);
        }
    }
    raster::png(&image)
}

/// The page with a coordinate grid in PDF points (origin bottom-left) for placing boxes, also on scans.
pub fn grid_page(pdf: &[u8], page: usize, dpi: f32) -> Result<Vec<u8>, String> {
    let doc = open(pdf)?;
    let pdf_page = page_of(&doc, page)?;
    let (width, height) = (pdf_page.width().value, pdf_page.height().value);
    let scale = dpi / 72.0;
    let mut image = render(&pdf_page, scale)?;
    let (img_w, img_h) = (image.width() as f32, image.height() as f32);
    for x in (0..=width as usize).step_by(GRID_STEP) {
        let strong = x % GRID_LABEL_STEP == 0;
        let px = x as f32 * scale;
        raster::line(&mut image, px, 0.0, px, img_h, (RED.0, RED.1, RED.2, if strong { 110 } else { 50 }));
        if strong {
            raster::text(&mut image, px + 2.0, 2.0, &x.to_string(), 10.0, RED);
        }
    }
    for y in (0..=height as usize).step_by(GRID_STEP) {
        let strong = y % GRID_LABEL_STEP == 0;
        let top = (height - y as f32) * scale;
        raster::line(&mut image, 0.0, top, img_w, top, (RED.0, RED.1, RED.2, if strong { 110 } else { 50 }));
        if strong {
            raster::text(&mut image, 2.0, top - 12.0, &y.to_string(), 10.0, RED);
        }
    }
    raster::png(&image)
}

fn bounded(text: &PdfPageText, left: f32, bottom: f32, right: f32, top: f32) -> String {
    let rect = PdfRect::new_from_values(bottom, left, top, right);
    text.inside_rect(rect).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Nearby printed text as a label: right of small boxes (checkbox style), else above, else left of the field.
/// Empty for scans without a text layer.
pub fn field_labels(pdf: &[u8], fields: &[FormField], limit: usize) -> Result<IndexMap<String, String>, String> {
    let doc = open(pdf)?;
    let mut labels = IndexMap::new();
    for field in fields {
        let Some([x0, y0, x1, y1]) = field.rects.first().map(|r| r.map(|v| v as f32)) else { continue };
        let Ok(page) = page_of(&doc, field.page) else { continue };
        let Ok(text) = page.text() else { continue };
        let right = (x1 + 2.0, y0 - 1.0, x1 + 60.0, y1 + 1.0);
        let above = (x0 - 10.0, y1, x1 + 10.0, y1 + 14.0);
        let left = ((x0 - 200.0).max(0.0), y0 - 1.0, x0, y1 + 1.0);
        let order = if x1 - x0 < 20.0 { vec![right, above, left] } else { vec![above, left] };
        for (box_left, bottom, box_right, top) in order {
            let found = bounded(&text, box_left, bottom, box_right, top);
            if !found.is_empty() {
                let label = if found.chars().count() <= limit {
                    found
                } else {
                    format!("{}…", found.chars().take(limit - 1).collect::<String>().trim_end())
                };
                labels.insert(field.name.clone(), label);
                break;
            }
        }
    }
    Ok(labels)
}

fn place(page: usize, rect: PdfRect) -> Place {
    (page, [rect.left().value as f64, rect.bottom().value as f64, rect.right().value as f64, rect.top().value as f64])
}

/// Where each needle is printed: name -> places. A needle broken across lines yields one rectangle per line.
pub fn find_text(pdf: &[u8], needles: &IndexMap<String, String>) -> Result<IndexMap<String, Vec<Place>>, String> {
    let doc = open(pdf)?;
    let mut found: IndexMap<String, Vec<Place>> = IndexMap::new();
    let options = PdfSearchOptions::new().match_case(true);
    for (index, page) in doc.pages().iter().enumerate() {
        let Ok(text) = page.text() else { continue };
        for (name, needle) in needles {
            if needle.is_empty() {
                continue;
            }
            let Ok(search) = text.search(needle, &options) else { continue };
            while let Some(segments) = search.find_next() {
                for segment in segments.iter() {
                    found.entry(name.clone()).or_default().push(place(index + 1, segment.bounds()));
                }
            }
        }
    }
    Ok(found)
}

#[derive(Serialize, Debug)]
pub struct Line {
    pub text: String,
    pub rect: [f64; 4],
}

/// Printed text on a 1-based page as lines with [x0, y0, x1, y1] in PDF points (origin bottom-left).
pub fn layout_lines(pdf: &[u8], page: usize, limit: usize) -> Result<Vec<Line>, String> {
    let doc = open(pdf)?;
    let pdf_page = page_of(&doc, page)?;
    let text = pdf_page.text().map_err(|e| e.to_string())?;
    let round = |v: f32| ((v as f64) * 10.0).round() / 10.0;
    Ok(text
        .segments()
        .iter()
        .take(limit)
        .filter_map(|segment| {
            let r = segment.bounds();
            let (l, b, rt, t) = (r.left().value, r.bottom().value, r.right().value, r.top().value);
            let found = bounded(&text, l, b, rt, t);
            (!found.is_empty()).then(|| Line { text: found, rect: [round(l), round(b), round(rt), round(t)] })
        })
        .collect())
}

/// All printed text, page by page.
pub fn text(pdf: &[u8]) -> Result<String, String> {
    let doc = open(pdf)?;
    Ok(doc.pages().iter().filter_map(|p| p.text().ok().map(|t| t.all())).collect::<Vec<_>>().join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::pdf_form;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap()
    }

    #[test]
    fn renders_and_annotates() {
        let pdf = fixture("form.pdf");
        assert_eq!(page_count(&pdf).unwrap(), 1);
        assert_eq!(page_sizes(&pdf).unwrap(), [(595.0, 842.0)]);
        let [png] = &render_pages(&pdf, &[], 40.0).unwrap()[..] else { panic!() };
        assert!(png.starts_with(b"\x89PNG"));
        let fields = pdf_form::inspect_form(&pdf).unwrap();
        assert!(annotate_fields(&pdf, &fields, 1, 110.0).unwrap().starts_with(b"\x89PNG"));
        assert!(grid_page(&pdf, 1, 80.0).unwrap().starts_with(b"\x89PNG"));
        assert!(render_pages(&pdf, &[2], 40.0).unwrap_err().contains("page must be 1..1"));
    }

    #[test]
    fn printed_labels_and_lines() {
        let pdf = fixture("form.pdf");
        let fields = pdf_form::inspect_form(&pdf).unwrap();
        let labels = field_labels(&pdf, &fields, 60).unwrap();
        assert_eq!(labels["name"], "Name");
        assert_eq!(labels["citizen_bg"], "Bulgarian");
        let lines = layout_lines(&fixture("flat.pdf"), 1, 300).unwrap();
        assert!(lines.iter().any(|l| l.text == "Name:" && (l.rect[0] - 40.0).abs() < 1.0));
    }

    #[test]
    fn finds_text() {
        let needles: IndexMap<String, String> = [("label".to_string(), "Married".to_string())].into_iter().collect();
        let found = find_text(&fixture("flat.pdf"), &needles).unwrap();
        let (page, rect) = found["label"][0];
        assert_eq!(page, 1);
        assert!((rect[0] - 40.0).abs() < 1.0 && rect[1] > 700.0);
    }
}
