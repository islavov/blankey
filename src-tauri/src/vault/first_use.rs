//! First use: where a new vault's owner is in the welcome, and which cases were made from a sample. Plaintext
//! metadata.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::error::Result;
use super::Vault;

const FIRST_USE: &str = "first_use";
/// The one sample case of vaults made while there was a single sample: its uuid.
const SAMPLE_CASE: &str = "sample_case";
const SAMPLE_CASES: &str = "sample_cases";
/// The sample that `SAMPLE_CASE` was made from.
const FIRST_SAMPLE: &str = "loan";

/// The stages a first use goes through; no stage means it is over, as in every vault made before it existed.
pub const FIRST_USE_STAGES: [&str; 2] = ["wizard", "case"];

/// A case made from a sample: which sample, and whether the user hid its guide.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SampleMark {
    pub sample: String,
    #[serde(default)]
    pub hidden: bool,
}

impl Vault {
    /// "wizard": the welcome shows; "case": the Cases page offers the sample case; None: done.
    pub fn first_use(&self) -> Result<Option<String>> {
        Ok(self.text_meta(FIRST_USE)?.filter(|stage| FIRST_USE_STAGES.contains(&stage.as_str())))
    }

    pub fn set_first_use(&self, stage: Option<&str>) -> Result<()> {
        self.set_text_meta(FIRST_USE, stage)
    }

    /// The cases made from a sample, by case uuid; a case may be deleted since.
    pub fn sample_cases(&self) -> Result<BTreeMap<String, SampleMark>> {
        let mut marks: BTreeMap<String, SampleMark> =
            self.text_meta(SAMPLE_CASES)?.and_then(|json| serde_json::from_str(&json).ok()).unwrap_or_default();
        if let Some(uuid) = self.text_meta(SAMPLE_CASE)? {
            marks.entry(uuid).or_insert_with(|| SampleMark { sample: FIRST_SAMPLE.to_string(), hidden: false });
        }
        Ok(marks)
    }

    /// Marks the case as made from `sample`.
    pub fn set_sample(&self, uuid: &str, sample: &str) -> Result<()> {
        let mut marks = self.sample_cases()?;
        marks.insert(uuid.to_string(), SampleMark { sample: sample.to_string(), hidden: false });
        self.set_sample_marks(&marks)
    }

    /// The case's guide no longer shows; nothing happens for a case that is not a sample.
    pub fn hide_sample_guide(&self, uuid: &str) -> Result<()> {
        let mut marks = self.sample_cases()?;
        let Some(mark) = marks.get_mut(uuid) else { return Ok(()) };
        mark.hidden = true;
        self.set_sample_marks(&marks)
    }

    fn set_sample_marks(&self, marks: &BTreeMap<String, SampleMark>) -> Result<()> {
        self.set_text_meta(SAMPLE_CASES, Some(&serde_json::to_string(marks).unwrap_or_default()))
    }
}
