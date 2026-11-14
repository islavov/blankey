//! Known documents: tells from a page's text lines which document it is (an ID card, a vehicle registration
//! certificate) and reads its fields, each checked where it can be. Pure functions over lines with boxes, whether
//! they come from a PDF's text layer or from OCR; no vault, no files. What the documents are comes in a config set
//! (`set`): this module is the engine that runs one, and knows no document itself.
//!
//! Positions are the page's own: `rect` is [x0, y0, x1, y1] with y growing down, in whatever unit the lines came in.

mod fold;
mod locate;
mod mrz;
pub mod ocr;
mod patterns;
mod select;
mod set;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::example_set_files;

use serde::{Deserialize, Serialize};

use locate::Scan;
use set::{Check, Config, Side};
pub use set::{Set, Text, ENGINE};

/// One line of text on a page.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Line {
    pub text: String,
    pub rect: [f64; 4],
    /// how sure the reader is, 0 to 1; a text layer is 1
    pub confidence: f32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Page {
    pub width: f64,
    pub height: f64,
    pub lines: Vec<Line>,
    /// quarter turns clockwise that stood the picture upright for reading; positions are in the upright picture
    #[serde(default)]
    pub turned: u8,
    /// the lean of the upright picture, in radians clockwise, that positions were laid level by
    #[serde(default)]
    pub skew: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// the value passed its check: a check digit, a checksum, a date or number that parses
    Ok,
    /// nothing to check it against
    Unchecked,
    /// read, and not a valid value: shown to the user, never saved
    Failed,
}

/// A field read from the document.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FieldRead {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub value: String,
    /// (index of the page, the value's box on it)
    pub place: Option<(usize, [f64; 4])>,
    pub status: Status,
}

/// What a file turned out to be and hold.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Reading {
    /// the document's id in its config set
    pub doctype: String,
    /// the document's name, kept with the reading so the file shows it without the set
    #[serde(default)]
    pub document: Text,
    /// documents of one family with the same `number` describe one thing
    pub family: String,
    /// the sides found, in page order: "front", "back"
    pub sides: Vec<String>,
    /// the kind of profile it makes: "person", "vehicle"
    pub kind: String,
    /// the profile's name; the document's name when no field of it was read
    pub name: String,
    /// what identifies the physical thing (the card's number, the VIN), when read and valid
    pub number: Option<String>,
    /// the key of the field `number` is
    pub number_key: String,
    /// per page read: the quarter turns clockwise that stood it upright (`Page::turned`)
    pub turned: Vec<u8>,
    /// per page read: the lean its positions were laid level by (`Page::skew`)
    pub skew: Vec<f64>,
    pub fields: Vec<FieldRead>,
}

/// Reads the pages of one file as a document of the set. None when no page is a side of one.
pub fn read(set: &Set, pages: &[Page], language: &str) -> Option<Reading> {
    let (config, sides) = select::pick(set, pages)?;
    Some(read_as(set, config, &sides, pages, language))
}

fn read_as(set: &Set, config: &Config, sides: &[(usize, &Side)], pages: &[Page], language: &str) -> Reading {
    let scans: Vec<Scan> = pages.iter().map(|page| Scan::new(page, config, &set.patterns)).collect();
    let mut fields: Vec<FieldRead> = Vec::new();
    for (index, field) in config.fields.iter().enumerate() {
        // over the sides that hold fields, a value that holds beats one that failed
        let found = sides
            .iter()
            .filter(|(_, side)| side.read)
            .filter_map(|(page, _)| Some((*page, scans[*page].field(index, field)?)));
        let Some((page, found)) = found.min_by_key(|(_, found)| found.checked == Some(false)) else { continue };
        if found.value.is_empty() {
            continue;
        }
        let label = field.label.get(language);
        fields.push(FieldRead {
            key: field.key.clone(),
            // the printed code stays on the label, so a form that asks for "(F.1)" finds its field
            label: field.code.as_ref().map_or(label.to_string(), |code| format!("{label} ({code})")),
            field_type: field.field_type.clone(),
            value: found.value,
            place: found.place.map(|rect| (page, rect)),
            status: match found.checked {
                Some(true) => Status::Ok,
                Some(false) => Status::Failed,
                None => Status::Unchecked,
            },
        });
    }
    for check in &config.checks {
        apply(check, &mut fields);
    }

    let valid =
        |key: &str| fields.iter().find(|f| f.key == key && f.status != Status::Failed).map(|f| f.value.as_str());
    let mut named = config.profile.name.clone();
    for field in &config.fields {
        named = named.replace(&format!("{{{}}}", field.key), valid(&field.key).unwrap_or(""));
    }
    let named = named.split_whitespace().collect::<Vec<_>>().join(" ");
    Reading {
        doctype: config.id.clone(),
        document: config.name.clone(),
        family: config.family.clone(),
        sides: sides.iter().map(|(_, side)| side.id.clone()).collect(),
        kind: config.profile.kind.clone(),
        name: if named.is_empty() { config.name.get(language).to_string() } else { named },
        number: valid(&config.profile.number).map(str::to_string),
        number_key: config.profile.number.clone(),
        turned: pages.iter().map(|page| page.turned).collect(),
        skew: pages.iter().map(|page| page.skew).collect(),
        fields,
    }
}

/// Fails the fields a cross-check finds at odds.
fn apply(check: &Check, fields: &mut [FieldRead]) {
    let [egn, date] = &check.egn_birth;
    let value =
        |key: &String| fields.iter().find(|f| &f.key == key && f.status != Status::Failed).map(|f| f.value.clone());
    let (Some(number), Some(printed)) = (value(egn), value(date)) else { return };
    let born = crate::pii::egn_birth_date(&number).map(|d| d.format("%d.%m.%Y").to_string());
    if born.as_deref() != Some(printed.as_str()) {
        fields.iter_mut().filter(|f| &f.key == egn || &f.key == date).for_each(|f| f.status = Status::Failed);
    }
}
