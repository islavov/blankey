//! First use: where a new vault's owner is in the welcome, and which case is the sample one. Plaintext metadata.

use super::error::Result;
use super::Vault;

const FIRST_USE: &str = "first_use";
const SAMPLE_CASE: &str = "sample_case";

/// The stages a first use goes through; no stage means it is over, as in every vault made before it existed.
pub const FIRST_USE_STAGES: [&str; 2] = ["wizard", "case"];

impl Vault {
    /// "wizard": the welcome shows; "case": the Cases page offers the sample case; None: done.
    pub fn first_use(&self) -> Result<Option<String>> {
        Ok(self.text_meta(FIRST_USE)?.filter(|stage| FIRST_USE_STAGES.contains(&stage.as_str())))
    }

    pub fn set_first_use(&self, stage: Option<&str>) -> Result<()> {
        self.set_text_meta(FIRST_USE, stage)
    }

    /// The uuid of the case made as the sample; it may be deleted since.
    pub fn sample_case(&self) -> Result<Option<String>> {
        self.text_meta(SAMPLE_CASE)
    }

    pub fn set_sample_case(&self, uuid: &str) -> Result<()> {
        self.set_text_meta(SAMPLE_CASE, Some(uuid))
    }
}
