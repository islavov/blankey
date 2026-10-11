//! AcroForm inspection and Unicode-safe filling.
//!
//! Text values are drawn onto the page with the embedded Noto Sans and the filled widget is removed
//! (flattened), so output never depends on the form's /DA fonts. Checkboxes and radio buttons keep their native
//! widgets and are switched via /AS.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use indexmap::IndexMap;
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use regex::Regex;
use serde::Serialize;
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use super::fonts::{self, EmbeddedFont};

pub const DEFAULT_SIZE: f32 = 9.0;
pub const MIN_SIZE: f32 = 5.0;
pub const PADDING: f32 = 2.0;
const COMB_FLAG: i64 = 1 << 24;
const RADIO_FLAG: i64 = 1 << 15;
const OFF_VALUES: &[&str] = &["", "false", "none", "0", "off", "/off"];
const DEFAULT_APPEARANCE: &str = "/Helv 0 Tf 0 g";
const FONT_RESOURCE: &str = "BlankeySans";
static FIELD_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z_]\w*$").unwrap());

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FormField {
    pub name: String,
    /// text | checkbox | radio | choice | signature
    #[serde(rename = "type")]
    pub field_type: String,
    /// 1-based
    pub page: usize,
    /// [x0, y0, x1, y1] in PDF points, origin bottom-left
    pub rects: Vec<[f64; 4]>,
    pub max_len: Option<i64>,
    pub comb: bool,
    pub states: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Fit {
    pub fits: bool,
    pub size: f32,
    /// needed width / available width at the chosen size
    pub ratio: f64,
}

// -- lopdf helpers ------------------------------------------------------------------------------------

pub fn load(pdf: &[u8]) -> Result<Document, String> {
    let mut doc = Document::load_mem(pdf).map_err(|e| format!("Cannot read the PDF: {e}"))?;
    if doc.encryption_state.is_some() || doc.is_encrypted() {
        // opened with the empty user password; saved without encryption
        doc.trailer.remove(b"Encrypt");
        doc.encryption_state = None;
    }
    Ok(doc)
}

pub fn save(doc: &mut Document) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|e| format!("Cannot write the PDF: {e}"))?;
    Ok(out)
}

fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    doc.dereference(obj).map(|(_, o)| o).unwrap_or(obj)
}

fn get<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    dict.get(key).ok().map(|o| resolve(doc, o))
}

fn get_dict<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    get(doc, dict, key).and_then(|o| o.as_dict().ok())
}

fn number(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn name_of(obj: &Object) -> Option<String> {
    obj.as_name().ok().map(|n| String::from_utf8_lossy(n).into_owned())
}

/// A PDF text string: UTF-16BE with BOM, UTF-8 with BOM, or PDFDocEncoding (read as Latin-1).
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    bytes.iter().map(|&b| b as char).collect()
}

fn parent<'a>(doc: &'a Document, dict: &'a Dictionary) -> Option<&'a Dictionary> {
    get_dict(doc, dict, b"Parent")
}

fn inherited<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let mut node = Some(dict);
    let mut depth = 0;
    while let Some(current) = node {
        if let Some(value) = get(doc, current, key) {
            return Some(value);
        }
        node = parent(doc, current);
        depth += 1;
        if depth > 32 {
            break;
        }
    }
    None
}

fn flags(doc: &Document, dict: &Dictionary) -> i64 {
    inherited(doc, dict, b"Ff").and_then(|o| o.as_i64().ok()).unwrap_or(0)
}

pub fn field_name(doc: &Document, dict: &Dictionary) -> Option<String> {
    let mut parts = Vec::new();
    let mut node = Some(dict);
    let mut depth = 0;
    while let Some(current) = node {
        if let Some(Object::String(bytes, _)) = get(doc, current, b"T") {
            parts.push(decode_text(bytes));
        }
        node = parent(doc, current);
        depth += 1;
        if depth > 32 {
            break;
        }
    }
    parts.reverse();
    (!parts.is_empty()).then(|| parts.join("."))
}

fn field_type(doc: &Document, dict: &Dictionary) -> String {
    let ft = inherited(doc, dict, b"FT").and_then(name_of).unwrap_or_default();
    match ft.as_str() {
        "Btn" if flags(doc, dict) & RADIO_FLAG != 0 => "radio",
        "Btn" => "checkbox",
        "Ch" => "choice",
        "Sig" => "signature",
        _ => "text",
    }
    .into()
}

fn rect(doc: &Document, dict: &Dictionary) -> Option<[f64; 4]> {
    let values: Vec<f64> =
        get(doc, dict, b"Rect")?.as_array().ok()?.iter().filter_map(|o| number(resolve(doc, o))).collect();
    let [a, b, c, d] = values[..] else { return None };
    Some([a.min(c), b.min(d), a.max(c), b.max(d)])
}

/// The widget annotations of a page: (annotation entry, its object id when it is a reference).
fn annotations(doc: &Document, page_id: ObjectId) -> Vec<(Object, Option<ObjectId>)> {
    let Ok(page) = doc.get_dictionary(page_id) else { return Vec::new() };
    let Some(Object::Array(items)) = get(doc, page, b"Annots") else { return Vec::new() };
    items.iter().map(|item| (item.clone(), item.as_reference().ok())).collect()
}

fn annotation_dict<'a>(doc: &'a Document, item: &'a Object) -> Option<&'a Dictionary> {
    resolve(doc, item).as_dict().ok()
}

fn is_widget(doc: &Document, dict: &Dictionary) -> bool {
    get(doc, dict, b"Subtype").and_then(name_of).as_deref() == Some("Widget")
}

/// On-states of a button widget ("/Yes", "/Choice2"), from its normal appearance.
fn button_states(doc: &Document, dict: &Dictionary) -> (Vec<String>, bool) {
    let normal = get_dict(doc, dict, b"AP").and_then(|ap| get_dict(doc, ap, b"N"));
    let Some(normal) = normal else { return (Vec::new(), false) };
    let mut states = Vec::new();
    let mut has_off = false;
    for (key, _) in normal.iter() {
        let key = String::from_utf8_lossy(key);
        if key == "Off" {
            has_off = true;
        } else {
            states.push(format!("/{key}"));
        }
    }
    (states, has_off)
}

fn page_box(doc: &Document, page_id: ObjectId) -> [f64; 4] {
    let Ok(page) = doc.get_dictionary(page_id) else { return [0.0, 0.0, 612.0, 792.0] };
    inherited(doc, page, b"MediaBox")
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(|o| number(resolve(doc, o))).collect::<Vec<_>>())
        .and_then(|v| <[f64; 4]>::try_from(v).ok())
        .map(|[a, b, c, d]| [a.min(c), b.min(d), a.max(c), b.max(d)])
        .unwrap_or([0.0, 0.0, 612.0, 792.0])
}

fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

// -- inspection ------------------------------------------------------------------------------------------

pub fn inspect_form(pdf: &[u8]) -> Result<Vec<FormField>, String> {
    let doc = load(pdf)?;
    Ok(inspect_doc(&doc))
}

fn inspect_doc(doc: &Document) -> Vec<FormField> {
    let mut fields: IndexMap<String, FormField> = IndexMap::new();
    for (page_no, page_id) in doc.get_pages() {
        for (item, _) in annotations(doc, page_id) {
            let Some(dict) = annotation_dict(doc, &item) else { continue };
            if !is_widget(doc, dict) {
                continue;
            }
            let Some(name) = field_name(doc, dict) else { continue };
            let Some([x0, y0, x1, y1]) = rect(doc, dict) else { continue };
            let field = fields.entry(name.clone()).or_insert_with(|| {
                let field_type = field_type(doc, dict);
                let max_len = inherited(doc, dict, b"MaxLen").and_then(|o| o.as_i64().ok());
                let comb = flags(doc, dict) & COMB_FLAG != 0 && field_type == "text";
                FormField {
                    name,
                    field_type,
                    page: page_no as usize,
                    rects: Vec::new(),
                    max_len,
                    comb,
                    states: Vec::new(),
                }
            });
            field.rects.push([round1(x0), round1(y0), round1(x1), round1(y1)]);
            if field.field_type == "checkbox" || field.field_type == "radio" {
                for state in button_states(doc, dict).0 {
                    if !field.states.contains(&state) {
                        field.states.push(state);
                    }
                }
            }
        }
    }
    fields.into_values().collect()
}

/// Picks a font size for a single-line value and reports whether it fits.
pub fn measure(rect: [f64; 4], value: &str, comb_len: usize) -> Fit {
    let [x0, y0, x1, y1] = rect.map(|v| v as f32);
    let (width, height) = (x1 - x0, y1 - y0);
    if comb_len > 0 {
        let cell = width / comb_len as f32;
        let size = DEFAULT_SIZE.min(height - 1.0);
        let widest = value.chars().map(|ch| fonts::string_width(&ch.to_string(), size)).fold(0.0, f32::max);
        let count = value.chars().count() as f32;
        let ratio = (count / comb_len as f32).max(if cell > 0.0 { widest / cell } else { 0.0 });
        return Fit { fits: ratio <= 1.0, size, ratio: round2(ratio as f64) };
    }
    let available = width - 2.0 * PADDING;
    let mut size = DEFAULT_SIZE.min(height - 3.0);
    let mut needed = fonts::string_width(value, size);
    if needed > available {
        size = MIN_SIZE.max(size * available / needed);
        needed = fonts::string_width(value, size);
    }
    let ratio = if available > 0.0 { needed as f64 / available as f64 } else { f64::INFINITY };
    Fit { fits: ratio <= 1.0001, size, ratio: round2(ratio) }
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

pub fn is_on(value: &str) -> bool {
    !OFF_VALUES.contains(&value.trim().to_lowercase().as_str())
}

// -- filling -----------------------------------------------------------------------------------------------

fn draw(ops: &mut String, font: &mut EmbeddedFont, rect: [f64; 4], value: &str, comb_len: usize) {
    let [x0, y0, x1, y1] = rect.map(|v| v as f32);
    let height = y1 - y0;
    let fit = measure(rect, value, comb_len);
    let baseline = y0 + (height - fit.size * 0.7) / 2.0;
    let mut show = |text: &str, x: f32| {
        let hex: String = font.encode(text).iter().map(|b| format!("{b:02X}")).collect();
        ops.push_str(&format!("BT /{FONT_RESOURCE} {:.2} Tf {x:.2} {baseline:.2} Td <{hex}> Tj ET\n", fit.size));
    };
    if comb_len > 0 {
        let cell = (x1 - x0) / comb_len as f32;
        let chars: Vec<char> = value.chars().collect();
        let shown: Vec<char> = if chars.len() >= comb_len {
            chars[chars.len() - comb_len..].to_vec()
        } else {
            std::iter::repeat_n(' ', comb_len - chars.len()).chain(chars).collect()
        };
        for (i, ch) in shown.iter().enumerate() {
            if ch.is_whitespace() {
                continue;
            }
            let text = ch.to_string();
            let x = x0 + cell * (i as f32 + 0.5) - fonts::string_width(&text, fit.size) / 2.0;
            show(&text, x);
        }
    } else {
        show(value, x0 + PADDING);
    }
}

/// Fills fields by name. Text is flattened; buttons get their on-state when the value is truthy. For radio
/// groups and multi-state checkboxes the value may name the state, e.g. "/Choice2".
pub fn fill_form(pdf: &[u8], values: &IndexMap<String, String>) -> Result<Vec<u8>, String> {
    let mut doc = load(pdf)?;
    let values: IndexMap<String, String> = values.iter().map(|(k, v)| (k.clone(), v.nfc().collect())).collect();
    let mut font = EmbeddedFont::new();
    let mut flattened: HashSet<String> = HashSet::new();
    let mut page_ops: BTreeMap<ObjectId, String> = BTreeMap::new();
    let mut buttons: Vec<(ObjectId, String)> = Vec::new();
    let mut kept_annots: Vec<(ObjectId, Vec<Object>)> = Vec::new();

    for (_, page_id) in doc.get_pages() {
        let mut kept = Vec::new();
        let mut removed = false;
        let mut ops = String::new();
        for (item, id) in annotations(&doc, page_id) {
            let dict = annotation_dict(&doc, &item);
            let target = dict.filter(|d| is_widget(&doc, d)).and_then(|d| {
                let name = field_name(&doc, d)?;
                Some((name.clone(), values.get(&name)?.clone(), d))
            });
            let (Some((name, value, dict)), Some(id)) = (target, id) else {
                kept.push(item);
                continue;
            };
            if inherited(&doc, dict, b"FT").and_then(name_of).as_deref() == Some("Btn") {
                kept.push(item);
                buttons.push((id, value));
                continue;
            }
            if !value.is_empty() {
                let comb_len = if flags(&doc, dict) & COMB_FLAG != 0 {
                    inherited(&doc, dict, b"MaxLen").and_then(|o| o.as_i64().ok()).unwrap_or(0).max(0) as usize
                } else {
                    0
                };
                if let Some(r) = rect(&doc, dict) {
                    draw(&mut ops, &mut font, r, &value, comb_len);
                }
            }
            flattened.insert(name);
            removed = true;
        }
        if removed {
            kept_annots.push((page_id, kept));
        }
        if !ops.is_empty() {
            page_ops.insert(page_id, ops);
        }
    }

    for (id, value) in buttons {
        set_button(&mut doc, id, &value);
    }
    for (page_id, kept) in kept_annots {
        if let Ok(page) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
            page.set("Annots", Object::Array(kept));
        }
    }
    if !font.is_empty() {
        let font_id = font.embed(&mut doc)?;
        for (page_id, ops) in page_ops {
            add_font_resource(&mut doc, page_id, font_id)?;
            append_content(&mut doc, page_id, format!("q 0 g\n{ops}Q\n").into_bytes())?;
        }
    }
    remove_fields(&mut doc, &flattened);
    save(&mut doc)
}

fn set_button(doc: &mut Document, annot_id: ObjectId, value: &str) {
    let Ok(dict) = doc.get_dictionary(annot_id) else { return };
    let (states, has_off) = button_states(doc, dict);
    let on = if value.starts_with('/') {
        states.iter().find(|s| *s == value).cloned()
    } else if is_on(value) && states.len() == 1 {
        Some(states[0].clone())
    } else {
        None
    };
    let owner =
        if dict.has(b"T") { Some(annot_id) } else { dict.get(b"Parent").ok().and_then(|p| p.as_reference().ok()) };
    match on {
        Some(state) => {
            let name = Object::Name(state.as_bytes()[1..].to_vec());
            if let Ok(annot) = doc.get_object_mut(annot_id).and_then(Object::as_dict_mut) {
                annot.set("AS", name.clone());
            }
            if let Some(owner) = owner {
                if let Ok(field) = doc.get_object_mut(owner).and_then(Object::as_dict_mut) {
                    field.set("V", name);
                }
            }
        }
        None if has_off => {
            if let Ok(annot) = doc.get_object_mut(annot_id).and_then(Object::as_dict_mut) {
                annot.set("AS", Object::Name(b"Off".to_vec()));
            }
        }
        None => {}
    }
}

fn add_font_resource(doc: &mut Document, page_id: ObjectId, font_id: ObjectId) -> Result<(), String> {
    let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
    let mut resources = inherited(doc, page, b"Resources").and_then(|o| o.as_dict().ok()).cloned().unwrap_or_default();
    let mut fonts = get_dict(doc, &resources, b"Font").cloned().unwrap_or_default();
    fonts.set(FONT_RESOURCE, Object::Reference(font_id));
    resources.set("Font", Object::Dictionary(fonts));
    let page = doc.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?;
    page.set("Resources", Object::Dictionary(resources));
    Ok(())
}

/// Wraps the existing content in q/Q and appends `ops` after it.
fn append_content(doc: &mut Document, page_id: ObjectId, ops: Vec<u8>) -> Result<(), String> {
    let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
    let existing: Vec<Object> = match page.get(b"Contents").ok() {
        Some(Object::Reference(id)) => match doc.get_object(*id) {
            Ok(Object::Array(items)) => items.clone(),
            _ => vec![Object::Reference(*id)],
        },
        Some(Object::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let open = doc.add_object(Stream::new(dictionary! {}, b"q\n".to_vec()));
    let overlay =
        doc.add_object(Stream::new(dictionary! {}, [b"Q\n".as_slice(), &ops].concat()).with_compression(true));
    let mut contents = vec![Object::Reference(open)];
    contents.extend(existing);
    contents.push(Object::Reference(overlay));
    let page = doc.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?;
    page.set("Contents", Object::Array(contents));
    Ok(())
}

fn root_id(doc: &Document) -> Option<ObjectId> {
    doc.trailer.get(b"Root").ok()?.as_reference().ok()
}

/// The AcroForm dictionary's object id, or None when it sits inline in the catalog.
fn acroform_id(doc: &Document) -> Option<ObjectId> {
    let root = doc.get_dictionary(root_id(doc)?).ok()?;
    root.get(b"AcroForm").ok()?.as_reference().ok()
}

fn with_acroform<T>(doc: &mut Document, f: impl FnOnce(&mut Dictionary) -> T) -> Option<T> {
    match acroform_id(doc) {
        Some(id) => doc.get_object_mut(id).ok()?.as_dict_mut().ok().map(f),
        None => {
            let root = root_id(doc)?;
            let catalog = doc.get_object_mut(root).ok()?.as_dict_mut().ok()?;
            catalog.get_mut(b"AcroForm").ok()?.as_dict_mut().ok().map(f)
        }
    }
}

fn acroform_fields(doc: &Document) -> Vec<Object> {
    let Some(root) = root_id(doc).and_then(|id| doc.get_dictionary(id).ok()) else { return Vec::new() };
    get_dict(doc, root, b"AcroForm")
        .and_then(|form| get(doc, form, b"Fields"))
        .and_then(|f| f.as_array().ok())
        .cloned()
        .unwrap_or_default()
}

fn remove_fields(doc: &mut Document, flattened: &HashSet<String>) {
    let fields = acroform_fields(doc);
    let kept: Vec<Object> = fields
        .into_iter()
        .filter(|f| match annotation_dict(doc, f).and_then(|d| field_name(doc, d)) {
            Some(name) => !flattened.contains(&name),
            None => true,
        })
        .collect();
    with_acroform(doc, |form| {
        form.set("Fields", Object::Array(kept));
        form.remove(b"NeedAppearances");
    });
}

// -- forms on demand: turn a flat PDF into an AcroForm --------------------------------------------------

/// Problems with boxes: unknown page, empty or off-page rect, bad or duplicate name, bad type.
pub fn validate_boxes(pdf: &[u8], boxes: &[Value]) -> Result<Vec<String>, String> {
    let doc = load(pdf)?;
    let pages = doc.get_pages();
    let existing: HashSet<String> = inspect_doc(&doc).into_iter().map(|f| f.name).collect();
    let mut problems = Vec::new();
    let mut seen = HashSet::new();
    for (number, b) in boxes.iter().enumerate() {
        let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let shown = if name.is_empty() { "unnamed" } else { &name };
        let place = format!("box {} ({shown})", number + 1);
        if !FIELD_NAME_RE.is_match(&name) {
            problems.push(format!("{place}: name must be letters, digits or _ and start with a letter or _"));
        } else if seen.contains(&name) || existing.contains(&name) {
            problems.push(format!("{place}: duplicate field name"));
        }
        seen.insert(name.clone());
        let kind = b.get("type").and_then(Value::as_str).unwrap_or("text");
        if kind != "text" && kind != "check" {
            problems.push(format!("{place}: type must be text or check"));
        }
        let page = b.get("page").and_then(Value::as_u64).filter(|p| *p >= 1 && *p as usize <= pages.len());
        let Some(page) = page else {
            problems.push(format!("{place}: page must be 1..{}", pages.len()));
            continue;
        };
        let rect: Option<Vec<f64>> =
            b.get("rect").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_f64).collect());
        let Some([x0, y0, x1, y1]) = rect.and_then(|r| <[f64; 4]>::try_from(r).ok()) else {
            problems.push(format!("{place}: rect must be [x0, y0, x1, y1] in PDF points"));
            continue;
        };
        let [left, bottom, right, top] = page_box(&doc, pages[&(page as u32)]);
        if !(x1 > x0 && y1 > y0) {
            problems.push(format!("{place}: rect must have x1 > x0 and y1 > y0 (origin bottom-left)"));
        } else if x0 < left || y0 < bottom || x1 > right || y1 > top {
            problems.push(format!("{place}: rect is outside the page ({} x {} pt)", right - left, top - bottom));
        }
    }
    Ok(problems)
}

/// Adds one text field per box: {name, page, rect, type: text|check, max_len?, comb?}. A check box is a one-cell
/// comb field, so "X" lands centered. Existing fields are kept.
pub fn add_fields(pdf: &[u8], boxes: &[Value]) -> Result<Vec<u8>, String> {
    let problems = validate_boxes(pdf, boxes)?;
    if !problems.is_empty() {
        return Err(format!("Invalid boxes:\n{}", problems.join("\n")));
    }
    let mut doc = load(pdf)?;
    let pages = doc.get_pages();
    let root = root_id(&doc).ok_or("The PDF has no catalog")?;
    let has_form = doc.get_dictionary(root).map(|c| c.has(b"AcroForm")).unwrap_or(false);
    if !has_form {
        let form = doc.add_object(dictionary! {});
        doc.get_object_mut(root).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?.set("AcroForm", form);
    }
    let mut fields = acroform_fields(&doc);
    for b in boxes {
        let page_id = pages[&(b["page"].as_u64().unwrap_or(1) as u32)];
        let check = b.get("type").and_then(Value::as_str) == Some("check");
        let max_len = if check { Some(1) } else { b.get("max_len").and_then(Value::as_i64) };
        let comb = check || (b.get("comb").and_then(Value::as_bool).unwrap_or(false) && max_len.is_some());
        let rect: Vec<Object> = b["rect"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_f64)
            .map(|v| Object::Real(v as f32))
            .collect();
        let mut widget = dictionary! {
            "Type" => "Annot",
            "Subtype" => "Widget",
            "FT" => "Tx",
            "T" => Object::String(b["name"].as_str().unwrap_or("").as_bytes().to_vec(), StringFormat::Literal),
            "Rect" => rect,
            "F" => 4,
            "DA" => Object::string_literal(DEFAULT_APPEARANCE),
            "P" => page_id,
        };
        if let Some(max_len) = max_len {
            widget.set("MaxLen", max_len);
        }
        if comb {
            widget.set("Ff", COMB_FLAG);
        }
        let id = doc.add_object(widget);
        let page = doc.get_dictionary(page_id).map_err(|e| e.to_string())?;
        let mut annots = get(&doc, page, b"Annots").and_then(|a| a.as_array().ok()).cloned().unwrap_or_default();
        annots.push(Object::Reference(id));
        doc.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(|e| e.to_string())?.set("Annots", annots);
        fields.push(Object::Reference(id));
    }
    with_acroform(&mut doc, |form| {
        form.set("Fields", Object::Array(fields));
        form.set("DA", Object::string_literal(DEFAULT_APPEARANCE));
    });
    save(&mut doc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name)).unwrap()
    }

    #[test]
    fn inspects_the_form() {
        let fields = inspect_form(&fixture("form.pdf")).unwrap();
        let by_name: IndexMap<&str, &FormField> = fields.iter().map(|f| (f.name.as_str(), f)).collect();
        assert_eq!(by_name["name"].field_type, "text");
        assert_eq!(by_name["name"].rects, [[100.0, 780.0, 400.0, 798.0]]);
        assert!(by_name["egn"].comb && by_name["egn"].max_len == Some(10));
        assert_eq!(by_name["citizen_bg"].field_type, "checkbox");
        assert_eq!(by_name["status"].field_type, "radio");
        assert_eq!(by_name["status"].states, ["/single", "/married"]);
        assert_eq!(by_name["status"].rects.len(), 2);
    }

    #[test]
    fn fills_text_and_buttons() {
        let values: IndexMap<String, String> = [
            ("name", "Jane Austen Иванова"),
            ("egn", "7501020018"),
            ("citizen_bg", "true"),
            ("status", "/married"),
            ("email", ""),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let pdf = fill_form(&fixture("form.pdf"), &values).unwrap();
        let fields = inspect_form(&pdf).unwrap();
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["citizen_bg", "status"]);
        let text = crate::render::preview::text(&pdf).unwrap();
        assert!(text.contains("Jane Austen Иванова"), "{text}");
        assert!(text.split_whitespace().collect::<String>().contains("7501020018"), "{text}");

        let doc = load(&pdf).unwrap();
        let mut values_of = IndexMap::new();
        for field in acroform_fields(&doc) {
            let dict = annotation_dict(&doc, &field).unwrap();
            values_of.insert(field_name(&doc, dict).unwrap(), get(&doc, dict, b"V").and_then(name_of));
        }
        assert_eq!(values_of["citizen_bg"].as_deref(), Some("Yes"));
        assert_eq!(values_of["status"].as_deref(), Some("married"));
    }

    #[test]
    fn measures_fit() {
        assert!(measure([0.0, 0.0, 300.0, 18.0], "short", 0).fits);
        let long = measure([0.0, 0.0, 40.0, 16.0], "a very long value that cannot fit", 0);
        assert!(!long.fits && long.size == MIN_SIZE && long.ratio > 1.0);
        assert!(measure([0.0, 0.0, 120.0, 16.0], "7501020018", 10).fits);
        assert!(!measure([0.0, 0.0, 120.0, 16.0], "75010200189", 10).fits);
    }

    #[test]
    fn add_fields_turns_a_flat_pdf_into_a_form() {
        let boxes = json!([
            {"name": "name", "page": 1, "rect": [90, 785, 400, 803]},
            {"name": "married", "page": 1, "rect": [100, 706, 112, 718], "type": "check"},
        ]);
        let form = add_fields(&fixture("flat.pdf"), boxes.as_array().unwrap()).unwrap();
        let fields = inspect_form(&form).unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].rects, [[90.0, 785.0, 400.0, 803.0]]);
        assert!(fields[1].comb && fields[1].max_len == Some(1));
        let values: IndexMap<String, String> =
            [("name", "Example"), ("married", "X")].into_iter().map(|(k, v)| (k.into(), v.into())).collect();
        let filled = fill_form(&form, &values).unwrap();
        assert!(crate::render::preview::text(&filled).unwrap().contains("Example"));
    }

    #[test]
    fn add_fields_rejects_bad_boxes() {
        let boxes = json!([
            {"name": "1bad", "page": 1, "rect": [0, 0, 10, 10]},
            {"name": "ok", "page": 3, "rect": [0, 0, 10, 10]},
            {"name": "inverted", "page": 1, "rect": [10, 10, 0, 0]},
            {"name": "off", "page": 1, "rect": [0, 0, 900, 10]},
            {"name": "dup", "page": 1, "rect": [0, 0, 10, 10]},
            {"name": "dup", "page": 1, "rect": [0, 0, 10, 10], "type": "radio"},
        ]);
        let problems = validate_boxes(&fixture("flat.pdf"), boxes.as_array().unwrap()).unwrap();
        let text = problems.join("\n");
        assert!(text.contains("box 1 (1bad): name must be"));
        assert!(text.contains("box 2 (ok): page must be 1..1"));
        assert!(text.contains("box 3 (inverted): rect must have x1 > x0"));
        assert!(text.contains("box 4 (off): rect is outside the page (595 x 842 pt)"));
        assert!(text.contains("box 6 (dup): duplicate field name"));
        assert!(text.contains("box 6 (dup): type must be text or check"));
    }
}
