//! Known documents in a case: an imported ID card or registration certificate is read on this machine and becomes
//! a profile of the case, so a fill binds to it like to any profile. `doctypes` tells what a file is and reads its
//! fields; this module gets the file's pages, makes or extends the profile, and serves the check of one beside the
//! other. Nothing read leaves the vault: the profile holds the values, the file's metadata only what it is.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{json, Value};

use crate::core::{Blankey, Problem, ProfileSave, Target};
use crate::doctypes::ocr::{shown_rect, Engine, OcrSettings};
use crate::doctypes::{self, Line, Page, Reading, Status};
use crate::render::preview as pdf;
use crate::templates::preview::{PagePreview, Preview};
use crate::vault::{CollectedFile, FieldInput};

/// Image types OCR reads.
const IMAGES: &[&str] = &["image/png", "image/jpeg", "image/webp"];
/// A known document is a page or two; a long PDF is something else.
const MAX_PAGES: usize = 4;
/// What a scanned PDF page is rendered at for OCR.
const SCAN_DPI: f32 = 200.0;

fn string(e: impl ToString) -> String {
    e.to_string()
}

/// What a file was read as, from the reading kept with it: it stays known without the config set.
fn kept(file: &CollectedFile) -> Option<Reading> {
    serde_json::from_str(&file.reading).ok()
}

/// The name of the document a file was read as, in the UI's language; empty for a plain file.
pub fn document_name(file: &CollectedFile, language: &str) -> String {
    kept(file).map(|reading| reading.document.get(language).to_string()).unwrap_or_default()
}

/// One field in the check of a document: what the profile holds and what the document says.
#[derive(Serialize)]
pub struct CheckField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub value: String,
    pub read: String,
    pub status: Status,
}

/// A recognized file beside the profile it became.
#[derive(Serialize)]
pub struct DocumentCheck {
    pub profile: Value,
    /// the document's name in the UI language
    pub document: String,
    pub fields: Vec<CheckField>,
    pub preview: Preview,
    /// what was read in the file is not kept with it (it was read before readings were kept): the fields are the
    /// profile's own and the picture has no boxes, until `keep_reading` has read it again
    #[serde(skip)]
    pub unread: bool,
}

impl Blankey {
    /// The OCR models, loaded on first use; None when the model files are not installed.
    fn ocr(&self) -> Option<&Engine> {
        self.ocr.get_or_init(|| doctypes::ocr::model_dir().and_then(|dir| Engine::load(&dir).ok())).as_ref()
    }

    /// Whether the file is one that can be a known document: an image OCR reads, or a PDF.
    pub fn is_document_candidate(file: &CollectedFile) -> bool {
        file.case_id.is_some() && (IMAGES.contains(&file.file_type.as_str()) || file.file_type == "application/pdf")
    }

    /// The text lines of the file's first pages: a PDF's own text where it has any, OCR for an image or a scanned
    /// page. Positions are in the units the file is shown in: points for a PDF, pixels for an image.
    fn pages_of(&self, file: &CollectedFile, content: &[u8], ocr: &OcrSettings) -> Result<Vec<Page>, String> {
        if IMAGES.contains(&file.file_type.as_str()) {
            return Ok(self.ocr().map(|engine| engine.read(content, ocr)).transpose()?.into_iter().collect());
        }
        let mut pages = Vec::new();
        for (index, (width, height)) in pdf::page_sizes(content)?.into_iter().take(MAX_PAGES).enumerate() {
            let (width, height) = (width as f64, height as f64);
            let printed = pdf::layout_lines(content, index + 1, 2000)?;
            if !printed.is_empty() {
                // a PDF counts up from the bottom of the page
                let lines = printed.into_iter().map(|l| Line {
                    text: l.text,
                    rect: [l.rect[0], height - l.rect[3], l.rect[2], height - l.rect[1]],
                    confidence: 1.0,
                });
                pages.push(Page { width, height, lines: lines.collect(), turned: 0, skew: 0.0 });
                continue;
            }
            let Some(engine) = self.ocr() else { continue };
            let image = pdf::render_pages(content, &[index + 1], SCAN_DPI)?.pop().ok_or("The page did not render")?;
            // a page scanned sideways is read upright: its sizes swap, and the turn is kept for showing it
            let mut page = engine.read(&image, ocr)?;
            let sideways = page.turned % 2 == 1;
            let scale = if sideways { height / page.width } else { width / page.width };
            for line in &mut page.lines {
                line.rect = line.rect.map(|v| v * scale);
            }
            let (width, height) = if sideways { (height, width) } else { (width, height) };
            pages.push(Page { width, height, lines: page.lines, turned: page.turned, skew: page.skew });
        }
        Ok(pages)
    }

    /// Reads a case file as a document of the config set held; None when it is none of them. Without a set nothing
    /// is read.
    pub fn read_document(&self, id: i64) -> Result<Option<Reading>, String> {
        let set = self.doc_set().ok_or("Document recognition needs Blankey Online")?;
        let (file, content) = self.vault().load_collected_file(id).map_err(string)?;
        if !Self::is_document_candidate(&file) {
            return Ok(None);
        }
        let pages = self.pages_of(&file, &content, &set.ocr)?;
        Ok(doctypes::read(&set, &pages, &self.language()))
    }

    /// The profile of the case that a file of the same document (its other side, its other part) already became:
    /// the one whose number is the reading's.
    fn sibling_profile(&self, case_id: i64, reading: &Reading) -> Result<Option<i64>, String> {
        let Some(number) = &reading.number else { return Ok(None) };
        let vault = self.vault();
        let same_family =
            |f: &CollectedFile| f.case_id == Some(case_id) && kept(f).is_some_and(|kept| kept.family == reading.family);
        for file in vault.list_collected_files().map_err(string)?.into_iter().filter(same_family) {
            let Some(profile_id) = file.profile_id else { continue };
            let held = vault.get_values(profile_id).map_err(string)?;
            if held.get(&reading.number_key).is_some_and(|(_, value)| value == number) {
                return Ok(Some(profile_id));
            }
        }
        Ok(None)
    }

    /// Makes the reading of a case file a profile of its case, or adds what is new to the profile the document's
    /// other side made. A value that failed its check is not saved: its field is there, empty. Returns the profile;
    /// None for a side that holds no fields.
    pub fn file_profile(&self, id: i64, reading: &Reading) -> Result<Option<i64>, String> {
        let file = self.vault().get_collected_file(id).map_err(string)?;
        let case_id = file.case_id.ok_or("Only a case file becomes a profile")?;
        let kept = serde_json::to_string(reading).map_err(string)?;
        if reading.fields.is_empty() {
            self.vault().set_file_document(id, &reading.doctype, None, &kept).map_err(string)?;
            return Ok(None);
        }
        fn saved(field: &doctypes::FieldRead) -> &str {
            if field.status == Status::Failed {
                ""
            } else {
                &field.value
            }
        }
        let sibling = self.sibling_profile(case_id, reading)?;
        let mut vault = self.vault();
        let profile_id = match sibling {
            Some(profile_id) => profile_id,
            None => vault.create_local_profile(&reading.name, &reading.kind, case_id, "").map_err(string)?,
        };
        // what the profile already holds stays: the user may have corrected it
        let held = vault.get_values(profile_id).map_err(string)?;
        let empty = |key: &str| held.get(key).is_none_or(|(_, value)| value.is_empty());
        let items: Vec<FieldInput> = reading
            .fields
            .iter()
            .filter(|field| empty(&field.key))
            .map(|field| FieldInput::new(&field.key, saved(field), &field.label, &field.field_type))
            .collect();
        vault.set_values(profile_id, &items).map_err(string)?;
        vault.set_file_document(id, &reading.doctype, Some(profile_id), &kept).map_err(string)?;
        let _ = vault.audit("app", "read_document", &id.to_string());
        Ok(Some(profile_id))
    }

    /// The case files being read right now.
    pub fn files_being_read(&self) -> Vec<i64> {
        self.reading.lock().unwrap().iter().copied().collect()
    }

    /// Reads newly imported case files in the background, one after another, and tells the window after each. It
    /// stops when the vault locks: what was not read stays a plain file.
    pub fn read_new_files(self: &Arc<Self>, ids: Vec<i64>) {
        if self.doc_set().is_none() {
            return;
        }
        let candidates: Vec<i64> = {
            let vault = self.vault();
            ids.into_iter()
                .filter(|id| vault.get_collected_file(*id).is_ok_and(|file| Self::is_document_candidate(&file)))
                .collect()
        };
        if candidates.is_empty() {
            return;
        }
        self.reading.lock().unwrap().extend(candidates.iter().copied());
        let app = self.clone();
        std::thread::spawn(move || {
            for id in &candidates {
                if app.vault().unlocked() {
                    if let Ok(Some(reading)) = app.read_document(*id) {
                        let _ = app.file_profile(*id, &reading);
                    }
                }
                app.reading.lock().unwrap().remove(id);
                app.cases_changed();
            }
        });
    }

    /// For each profile made from a case file: the file and the document it was read as.
    pub fn profile_documents(&self) -> Result<HashMap<i64, (i64, String)>, String> {
        let files = self.vault().list_collected_files().map_err(string)?;
        let made = files.into_iter().rev().filter(|f| !f.doctype.is_empty() && f.case_id.is_some());
        Ok(made.filter_map(|f| Some((f.profile_id?, (f.id, f.doctype)))).collect())
    }

    /// For each profile made from a case file: the name of the document it was read as.
    pub fn profile_document_names(&self) -> Result<HashMap<i64, String>, String> {
        let (files, language) = (self.vault().list_collected_files().map_err(string)?, self.language());
        let made = files.into_iter().rev().filter(|f| !f.doctype.is_empty() && f.case_id.is_some());
        Ok(made.filter_map(|f| Some((f.profile_id?, document_name(&f, &language)))).collect())
    }

    /// The file as the check sheet shows it, with a box on each value that was read.
    fn check_preview(&self, file: &CollectedFile, content: &[u8], reading: Option<&Reading>) -> Preview {
        let read: &[doctypes::FieldRead] = reading.map_or(&[], |reading| &reading.fields);
        // a value's box is in the page stood upright; the window shows the page as it is, and counts a box up
        // from the bottom of it
        let boxes = |page: usize, width: f64, height: f64| {
            let turned = reading.and_then(|r| r.turned.get(page).copied()).unwrap_or(0);
            let skew = reading.and_then(|r| r.skew.get(page).copied()).unwrap_or(0.0);
            let mut boxes: IndexMap<String, Vec<[f64; 4]>> = IndexMap::new();
            for field in read {
                if let Some((_, rect)) = field.place.filter(|(at, _)| *at == page) {
                    let [x0, y0, x1, y1] = shown_rect(rect, width, height, turned, skew);
                    boxes.entry(field.key.clone()).or_default().push([x0, height - y1, x1, height - y0]);
                }
            }
            boxes
        };
        let names = read.iter().map(|f| f.key.clone()).collect();
        if file.file_type == "application/pdf" {
            let Preview::Pages { mut pages, .. } = crate::templates::preview::pdf_view(content) else {
                return Preview::Error { message: "The file cannot be shown".into() };
            };
            pages.truncate(MAX_PAGES);
            for (index, page) in pages.iter_mut().enumerate() {
                page.boxes = boxes(index, page.width as f64, page.height as f64);
            }
            return Preview::Pages { pages, names };
        }
        let Some(size) = doctypes::ocr::shown_size(content) else {
            return Preview::Error { message: "The file cannot be shown".into() };
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(content);
        let page = PagePreview {
            image: format!("data:{};base64,{encoded}", file.file_type),
            width: size.0 as f32,
            height: size.1 as f32,
            boxes: boxes(0, size.0 as f64, size.1 as f64),
        };
        Preview::Pages { pages: vec![page], names }
    }

    /// A recognized case file beside its profile: each field that was read in it comes with what the profile
    /// holds for it. Nothing is read here, so it opens at once: it shows what was read when the file came in.
    pub fn check_document(&self, id: i64) -> Result<DocumentCheck, String> {
        let (file, content) = self.vault().load_collected_file(id).map_err(string)?;
        let profile_id =
            file.profile_id.filter(|_| !file.doctype.is_empty()).ok_or("This file was not read into a profile")?;
        let reading = serde_json::from_str::<Reading>(&file.reading).ok();
        let (profile, held, own) = {
            let vault = self.vault();
            let profile = vault.get_profile(profile_id).map_err(string)?;
            (profile, vault.get_values(profile_id).map_err(string)?, vault.fields(profile_id).map_err(string)?)
        };
        let fields = match &reading {
            Some(reading) => reading
                .fields
                .iter()
                .map(|field| CheckField {
                    key: field.key.clone(),
                    label: field.label.clone(),
                    field_type: field.field_type.clone(),
                    value: held.get(&field.key).map(|(_, value)| value.clone()).unwrap_or_default(),
                    read: field.value.clone(),
                    status: field.status,
                })
                .collect(),
            // without what was read: the profile as it is
            None => own
                .into_iter()
                .map(|field| CheckField {
                    key: field.key,
                    label: field.label,
                    field_type: field.field_type,
                    value: field.value.unwrap_or_default(),
                    read: String::new(),
                    status: Status::Unchecked,
                })
                .collect(),
        };
        Ok(DocumentCheck {
            profile: json!({"id": profile.id, "name": profile.name}),
            document: document_name(&file, &self.language()),
            fields,
            preview: self.check_preview(&file, &content, reading.as_ref()),
            unread: reading.is_none(),
        })
    }

    /// Reads a recognized file again in the background and keeps what was read with it, for a file read before
    /// readings were kept. Its profile is left as it is.
    pub fn keep_reading(self: &Arc<Self>, id: i64) {
        if self.doc_set().is_none() || !self.reading.lock().unwrap().insert(id) {
            return;
        }
        let app = self.clone();
        std::thread::spawn(move || {
            let kept = app.read_document(id).ok().flatten().and_then(|reading| {
                let file = app.vault().get_collected_file(id).ok()?;
                let json = serde_json::to_string(&reading).ok()?;
                app.vault().set_file_document(id, &reading.doctype, file.profile_id, &json).ok()
            });
            app.reading.lock().unwrap().remove(&id);
            if kept.is_some() {
                app.cases_changed();
            }
        });
    }

    /// Saves the checked values into the file's profile; the problems when a value is not valid for its type.
    pub fn save_document_check(&self, id: i64, values: &IndexMap<String, String>) -> Result<Vec<Problem>, String> {
        let file = self.vault().get_collected_file(id).map_err(string)?;
        let profile_id =
            file.profile_id.filter(|_| !file.doctype.is_empty()).ok_or("This file was not read into a profile")?;
        let known = self.vault().describe(profile_id).map_err(string)?;
        let specs: Vec<Value> = known
            .iter()
            .filter(|f| values.contains_key(&f.key))
            .map(|f| json!({"key": f.key, "label": f.label, "type": f.field_type}))
            .collect();
        Ok(match self.save_profile_values(&specs, values, Target::Existing(profile_id))? {
            ProfileSave::Problems(problems) => problems,
            ProfileSave::Saved { .. } => {
                self.audit("user", "save_document_check", &id.to_string());
                Vec::new()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctypes::FieldRead;
    use crate::mcp::tools::tests::app;

    fn field(key: &str, value: &str, status: Status) -> FieldRead {
        FieldRead {
            key: key.into(),
            label: key.to_uppercase(),
            field_type: if key == "egn" { "egn" } else { "text" }.into(),
            value: value.into(),
            place: None,
            status,
        }
    }

    fn id_card(side: &str, fields: Vec<FieldRead>) -> Reading {
        Reading {
            doctype: "bg_id_card".into(),
            document: doctypes::Text { en: "ID card".into(), bg: "Лична карта".into() },
            family: "bg_id_card".into(),
            sides: vec![side.into()],
            kind: "person".into(),
            name: "Jane Example".into(),
            number: fields.iter().find(|f| f.key == "id_card.number").map(|f| f.value.clone()),
            number_key: "id_card.number".into(),
            turned: vec![0],
            skew: vec![0.0],
            fields,
        }
    }

    /// A case with one image file in it, as an import leaves it.
    fn case_file(app: &Blankey, dir: &std::path::Path, case: i64, name: &str) -> i64 {
        let path = dir.join(name);
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).unwrap();
        app.keep_files(crate::links::FileOwner::Case(case), &[(path, name.into())]).unwrap()[0]
    }

    fn held(app: &Blankey, profile: i64, key: &str) -> Option<String> {
        app.vault().get_values(profile).unwrap().get(key).map(|(_, value)| value.clone())
    }

    #[test]
    fn a_box_of_a_turned_picture_lands_where_the_picture_shows_it() {
        // a 400 x 300 picture: its top-left corner, as each turn stands it upright
        assert_eq!(shown_rect([0.0, 0.0, 10.0, 20.0], 400.0, 300.0, 0, 0.0), [0.0, 0.0, 10.0, 20.0]);
        assert_eq!(
            shown_rect([0.0, 0.0, 10.0, 20.0], 400.0, 300.0, 1, 0.0),
            [0.0, 290.0, 20.0, 300.0],
            "turned clockwise: from its bottom-left"
        );
        assert_eq!(
            shown_rect([0.0, 0.0, 10.0, 20.0], 400.0, 300.0, 2, 0.0),
            [390.0, 280.0, 400.0, 300.0],
            "upside down: from its bottom-right"
        );
        assert_eq!(
            shown_rect([0.0, 0.0, 10.0, 20.0], 400.0, 300.0, 3, 0.0),
            [380.0, 0.0, 400.0, 10.0],
            "turned the other way: from its top-right"
        );
    }

    #[test]
    fn a_box_of_a_leaning_picture_goes_back_to_its_lean() {
        // a quarter turn of lean, to see it plainly: a box right of the middle was below it in the picture
        let quarter = std::f64::consts::FRAC_PI_2;
        let shown = shown_rect([290.0, 140.0, 310.0, 160.0], 400.0, 300.0, 0, quarter);
        assert_eq!(shown.map(|v| v.round()), [190.0, 240.0, 210.0, 260.0]);
        let middle = shown_rect([190.0, 140.0, 210.0, 160.0], 400.0, 300.0, 0, 0.3);
        assert_eq!(middle.map(|v| v.round()), [190.0, 140.0, 210.0, 160.0], "the middle stays");
    }

    #[test]
    fn a_reading_becomes_a_profile_of_the_case_and_a_failed_value_stays_out() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let file = case_file(&app, dir.path(), case, "front.png");
        let reading = id_card(
            "front",
            vec![
                field("name.first", "Jane", Status::Unchecked),
                field("egn", "7501020019", Status::Failed),
                field("id_card.number", "AA1234567", Status::Ok),
            ],
        );

        let profile = app.file_profile(file, &reading).unwrap().unwrap();
        let (found, role, cases) =
            app.case_profiles(case).unwrap().into_iter().find(|(p, ..)| p.id == profile).unwrap();
        assert_eq!(
            (found.name.as_str(), found.kind.as_str(), found.global, role.as_str(), cases),
            ("Jane Example", "person", false, "", 1)
        );
        assert_eq!(held(&app, profile, "name.first").as_deref(), Some("Jane"));
        assert_eq!(
            held(&app, profile, "egn").as_deref(),
            Some(""),
            "the field is there for the user to fill, without the misread"
        );
        let kept = app.vault().get_collected_file(file).unwrap();
        assert_eq!((kept.doctype.as_str(), kept.profile_id, kept.case_id), ("bg_id_card", Some(profile), Some(case)));
        let again: Reading = serde_json::from_str(&kept.reading).unwrap();
        assert_eq!(again.fields.len(), 3, "what was read is kept with the file, so showing it does not read it again");
        assert_eq!(app.profile_documents().unwrap().get(&profile), Some(&(file, "bg_id_card".to_string())));
    }

    #[test]
    fn the_other_side_joins_the_profile_by_the_documents_number() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let (front, back, other) = (
            case_file(&app, dir.path(), case, "front.png"),
            case_file(&app, dir.path(), case, "back.png"),
            case_file(&app, dir.path(), case, "other.png"),
        );
        let number = || field("id_card.number", "AA1234567", Status::Ok);
        let profile = app
            .file_profile(front, &id_card("front", vec![field("name.first", "Jane", Status::Unchecked), number()]))
            .unwrap()
            .unwrap();
        // the user corrects a value before the other side is read
        app.vault().set_values(profile, &[FieldInput::new("name.first", "Jayne", "NAME.FIRST", "text")]).unwrap();

        let joined = app
            .file_profile(
                back,
                &id_card(
                    "back",
                    vec![
                        field("name.first", "Jane", Status::Unchecked),
                        field("address", "12 Oak Street", Status::Unchecked),
                        number(),
                    ],
                ),
            )
            .unwrap();
        assert_eq!(joined, Some(profile), "the same card: one profile");
        assert_eq!(held(&app, profile, "address").as_deref(), Some("12 Oak Street"));
        assert_eq!(
            held(&app, profile, "name.first").as_deref(),
            Some("Jayne"),
            "what the profile holds is not overwritten"
        );

        let another =
            app.file_profile(other, &id_card("front", vec![field("id_card.number", "BB7654321", Status::Ok)])).unwrap();
        assert_ne!(another, Some(profile), "another card is another profile");
        assert_eq!(app.case_profiles(case).unwrap().len(), 2);
    }

    #[test]
    fn a_side_without_fields_is_recognized_and_makes_no_profile() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let file = case_file(&app, dir.path(), case, "legend.png");
        let mut legend = id_card("legend", vec![]);
        legend.doctype = "bg_vehicle_registration_1".into();
        assert_eq!(app.file_profile(file, &legend).unwrap(), None);
        let kept = app.vault().get_collected_file(file).unwrap();
        assert_eq!((kept.doctype.as_str(), kept.profile_id), ("bg_vehicle_registration_1", None));
        assert!(app.case_profiles(case).unwrap().is_empty());
    }

    #[test]
    fn removing_the_profile_leaves_the_case_file() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let file = case_file(&app, dir.path(), case, "front.png");
        let profile = app
            .file_profile(file, &id_card("front", vec![field("name.first", "Jane", Status::Unchecked)]))
            .unwrap()
            .unwrap();
        assert!(app.remove_case_profile(case, profile).unwrap(), "a profile only this case has goes with it");
        let kept = app.vault().get_collected_file(file).unwrap();
        assert_eq!((kept.case_id, kept.profile_id), (Some(case), None), "the file is the case's and stays");
    }

    #[test]
    fn a_check_opens_from_what_was_kept_and_without_it_from_the_profile() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let file = case_file(&app, dir.path(), case, "front.png");
        let reading = id_card(
            "front",
            vec![field("name.first", "Jane", Status::Unchecked), field("egn", "7501020019", Status::Failed)],
        );
        let profile = app.file_profile(file, &reading).unwrap().unwrap();

        let check = app.check_document(file).unwrap();
        assert!(!check.unread);
        assert_eq!(check.document, "ID card");
        let egn = check.fields.iter().find(|f| f.key == "egn").unwrap();
        assert_eq!(
            (egn.value.as_str(), egn.read.as_str(), egn.status),
            ("", "7501020019", Status::Failed),
            "what was read is offered, the profile was left empty"
        );

        // a file read before readings were kept: nothing to wait for, the profile's own fields show
        app.vault().set_file_document(file, "bg_id_card", Some(profile), "").unwrap();
        let check = app.check_document(file).unwrap();
        assert!(check.unread);
        let name = check.fields.iter().find(|f| f.key == "name.first").unwrap();
        assert_eq!((name.value.as_str(), name.read.as_str()), ("Jane", ""));
    }

    #[test]
    fn saving_a_check_validates_and_writes_into_the_files_profile() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let case = app.create_case("Sale").unwrap();
        let file = case_file(&app, dir.path(), case, "front.png");
        let profile = app
            .file_profile(
                file,
                &id_card(
                    "front",
                    vec![field("name.first", "Jane", Status::Unchecked), field("egn", "7501020019", Status::Failed)],
                ),
            )
            .unwrap()
            .unwrap();

        let values = |egn: &str| {
            IndexMap::from([("name.first".to_string(), "Jane".to_string()), ("egn".to_string(), egn.to_string())])
        };
        let problems = app.save_document_check(file, &values("123")).unwrap();
        assert_eq!(problems, [Problem::new("egn", "EGN", "An EGN or LNCh has 10 digits")]);
        assert_eq!(held(&app, profile, "egn").as_deref(), Some(""));
        assert!(app.save_document_check(file, &values("7501020018")).unwrap().is_empty());
        assert_eq!(held(&app, profile, "egn").as_deref(), Some("7501020018"));

        let plain = case_file(&app, dir.path(), case, "notes.png");
        assert!(
            app.save_document_check(plain, &values("7501020018")).is_err(),
            "a plain file has no profile to save into"
        );
    }
}
