//! Sample cases: invented documents and the invented people in them, in the UI language, each with a guide to a first
//! document. A sample is a folder `<id>/<language>/` holding `case.yaml` and the document; adding one takes the
//! folder and an entry in `SAMPLES`. The contract names exactly the values the profiles hold, so an assistant can
//! bind them when it makes a template.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::Blankey;
use crate::recognize::recognize;
use crate::vault::{FieldInput, NewFile};

/// A sample in one language: (language, case.yaml, the document).
type Translation = (&'static str, &'static str, &'static [u8]);

struct Source {
    id: &'static str,
    /// English first: it stands in for a language the sample is not in
    languages: &'static [Translation],
}

/// The first one is what a first use offers.
const SAMPLES: &[Source] = &[Source {
    id: "loan",
    languages: &[
        ("en", include_str!("loan/en/case.yaml"), include_bytes!("loan/en/contract.docx")),
        ("bg", include_str!("loan/bg/case.yaml"), include_bytes!("loan/bg/contract.docx")),
    ],
}];

/// What a guide step can wait for: how many of each the case holds.
#[cfg(test)]
const COUNTS: [&str; 6] = ["files", "profiles", "templates", "fill_sets", "documents", "requests"];

#[derive(Deserialize)]
struct Sample {
    title: String,
    description: String,
    file: SampleFile,
    profiles: Vec<SampleProfile>,
    guide: Vec<GuideStep>,
}

#[derive(Deserialize)]
struct SampleFile {
    name: String,
    description: String,
}

#[derive(Deserialize)]
struct SampleProfile {
    name: String,
    kind: String,
    role: String,
    fields: Vec<SampleField>,
}

#[derive(Deserialize)]
struct SampleField {
    key: String,
    label: String,
    value: String,
}

/// One step of a sample's guide, as the case view shows it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuideStep {
    /// its name in the row of steps
    pub title: String,
    pub heading: String,
    pub text: String,
    /// what to ask the assistant; none for a step that is the user's own
    #[serde(default)]
    pub prompt: Option<String>,
    /// what the step offers instead of a prompt: "open_fill"
    #[serde(default)]
    pub action: Option<String>,
    /// the counts of `COUNTS` that show the step is done, any one reaching its number
    #[serde(skip_serializing)]
    done: BTreeMap<String, i64>,
}

/// A sample for the Cases page to offer.
#[derive(Debug, Serialize)]
pub struct SampleInfo {
    pub id: String,
    pub title: String,
    pub description: String,
}

/// Where a sample case is on the way to its first document.
#[derive(Debug, Serialize)]
pub struct Guide {
    pub sample: String,
    /// the index of the current step; the number of steps once all are done
    pub step: usize,
    pub steps: Vec<GuideStep>,
}

/// The sample in `language`, or in English when it is not in that language, with its document.
fn load(id: &str, language: &str) -> Result<(Sample, &'static [u8]), String> {
    let source = SAMPLES.iter().find(|s| s.id == id).ok_or_else(|| format!("unknown sample {id}"))?;
    let (_, yaml, document) =
        source.languages.iter().find(|(code, ..)| *code == language).unwrap_or(&source.languages[0]);
    Ok((serde_yaml::from_str(yaml).map_err(|e| e.to_string())?, document))
}

/// The samples there are, described in `language`.
pub fn list_samples(language: &str) -> Result<Vec<SampleInfo>, String> {
    SAMPLES
        .iter()
        .map(|source| {
            let (sample, _) = load(source.id, language)?;
            Ok(SampleInfo { id: source.id.to_string(), title: sample.title, description: sample.description })
        })
        .collect()
}

/// The current step: one past the last step that is done, since a later step done means the earlier ones are too.
fn current_step(steps: &[GuideStep], counts: &BTreeMap<&str, i64>) -> usize {
    let done =
        |step: &GuideStep| step.done.iter().any(|(name, n)| counts.get(name.as_str()).is_some_and(|have| have >= n));
    steps.iter().rposition(done).map_or(0, |last| last + 1)
}

impl Blankey {
    /// Makes a case from `sample` (the first one when none is named) in `language` (English when the sample is not
    /// in it) and ends the first use. Returns its id. Needs the vault unlocked.
    pub fn create_sample_case(&self, sample: Option<&str>, language: &str) -> Result<i64, String> {
        let sample_id = sample.unwrap_or(SAMPLES[0].id);
        let (sample, document) = load(sample_id, language)?;
        let id = self.create_case(&sample.title)?;
        let seen = recognize(&sample.file.name, document);
        let file = NewFile {
            profile_id: None,
            link_id: "",
            case_id: Some(id),
            name: &sample.file.name,
            file_type: seen.kind.media_type(),
            description: &sample.file.description,
            sha256: &seen.sha256,
        };
        self.vault().add_collected_file(file, document).map_err(|e| e.to_string())?;
        for profile in &sample.profiles {
            let profile_id = self.create_case_profile(id, &profile.name, &profile.kind, &profile.role)?;
            let values: Vec<FieldInput> =
                profile.fields.iter().map(|f| FieldInput::new(&f.key, &f.value, &f.label, "text")).collect();
            self.vault().set_values(profile_id, &values).map_err(|e| e.to_string())?;
        }
        let vault = self.vault();
        let uuid = vault.get_case(id).map_err(|e| e.to_string())?.uuid;
        vault.set_sample(&uuid, sample_id).map_err(|e| e.to_string())?;
        vault.set_first_use(None).map_err(|e| e.to_string())?;
        Ok(id)
    }

    /// The guide of a case made from a sample, in the UI language, with the step it is at; None for any other case,
    /// for a guide the user hid and for a sample this version no longer has.
    pub fn sample_guide(&self, case_id: i64) -> Result<Option<Guide>, String> {
        let scope = self.scope(Some(case_id))?;
        let Some(case) = scope.case.as_ref() else { return Ok(None) };
        let Some(mark) = self.vault().sample_cases().map_err(|e| e.to_string())?.remove(&case.uuid) else {
            return Ok(None);
        };
        if mark.hidden {
            return Ok(None);
        }
        let Ok((sample, _)) = load(&mark.sample, &self.language()) else { return Ok(None) };
        let held = self.vault().case_counts(case_id).map_err(|e| e.to_string())?;
        let counts = BTreeMap::from([
            ("files", held.files),
            ("profiles", self.case_profiles(case_id)?.len() as i64),
            ("templates", self.case_templates(&scope).len() as i64),
            ("fill_sets", held.fill_sets),
            ("documents", held.documents),
            ("requests", held.requests),
        ]);
        Ok(Some(Guide { sample: mark.sample, step: current_step(&sample.guide, &counts), steps: sample.guide }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::tests::app;

    #[test]
    fn sample_case_in_each_language() {
        for (language, title, lender) in
            [("en", "Sample: loan contract", "Maria Example"), ("bg", "Пример: договор за заем", "Мария Примерова")]
        {
            let dir = tempfile::tempdir().unwrap();
            let app = app(dir.path());
            app.vault().set_first_use(Some("case")).unwrap();
            let id = app.create_sample_case(None, language).unwrap();

            let vault = app.vault();
            let case = vault.get_case(id).unwrap();
            assert_eq!(case.title, title);
            assert_eq!(vault.sample_cases().unwrap()[&case.uuid].sample, "loan");
            assert_eq!(vault.first_use().unwrap(), None);

            let files: Vec<_> =
                vault.list_collected_files().unwrap().into_iter().filter(|f| f.case_id == Some(id)).collect();
            assert_eq!(files.len(), 1);
            let (_, content) = vault.load_collected_file(files[0].id).unwrap();
            assert_eq!(load("loan", language).unwrap().1, content.as_slice());
            drop(vault);

            let profiles = app.case_profiles(id).unwrap();
            assert_eq!(profiles.len(), 2);
            let (first, role, _) = &profiles[0];
            assert_eq!((first.name.as_str(), first.global), (lender, false));
            assert!(!role.is_empty());
            let values = app.vault().get_values(first.id).unwrap();
            assert_eq!(values["name"].1, lender);
            assert_eq!(values["id_card.number"].1, "640000001");
        }
    }

    #[test]
    fn unknown_language_gets_the_english_sample() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let id = app.create_sample_case(None, "de").unwrap();
        assert_eq!(app.vault().get_case(id).unwrap().title, "Sample: loan contract");
        assert_eq!(list_samples("de").unwrap()[0].title, "Sample: loan contract");
        assert!(app.create_sample_case(Some("lease"), "en").is_err(), "no such sample");
    }

    #[test]
    fn the_contract_names_what_the_profiles_hold() {
        for (_, yaml, docx) in SAMPLES.iter().flat_map(|source| source.languages) {
            let sample: Sample = serde_yaml::from_str(yaml).unwrap();
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("contract.docx");
            std::fs::write(&path, docx).unwrap();
            let text: String = crate::templates::docx::inspect(&path)
                .unwrap()
                .iter()
                .map(|p| serde_json::to_string(p).unwrap())
                .collect();
            for field in sample.profiles.iter().flat_map(|p| &p.fields) {
                let value = serde_json::to_string(&field.value).unwrap();
                assert!(text.contains(value.trim_matches('"')), "{} is not in the contract", field.value);
            }
        }
    }

    /// What a step is without its texts: whether it asks, what it offers and when it is done.
    type StepShape = (bool, Option<String>, BTreeMap<String, i64>);

    #[test]
    fn every_sample_has_the_same_guide_in_each_language() {
        for source in SAMPLES {
            assert_eq!(source.languages[0].0, "en", "{}: English stands in for other languages", source.id);
            let shapes: Vec<Vec<StepShape>> = source
                .languages
                .iter()
                .map(|(language, ..)| {
                    let (sample, _) = load(source.id, language).unwrap();
                    assert!(!sample.description.is_empty() && !sample.guide.is_empty(), "{} {language}", source.id);
                    sample.guide.into_iter().map(|step| (step.prompt.is_some(), step.action, step.done)).collect()
                })
                .collect();
            for shape in &shapes {
                assert_eq!(shape, &shapes[0], "{}: the steps differ between languages", source.id);
                for (asks, action, done) in shape {
                    assert!(*asks || action.as_deref() == Some("open_fill"), "a step asks or offers a known action");
                    assert!(!done.is_empty() && done.keys().all(|name| COUNTS.contains(&name.as_str())), "{done:?}");
                }
            }
        }
    }

    #[test]
    fn the_guide_is_at_the_step_after_the_last_one_done() {
        let (sample, _) = load("loan", "en").unwrap();
        let at = |counts: &[(&'static str, i64)]| current_step(&sample.guide, &counts.iter().copied().collect());
        assert_eq!(at(&[("files", 1), ("profiles", 2)]), 0);
        assert_eq!(at(&[("templates", 1)]), 1);
        assert_eq!(at(&[("templates", 1), ("requests", 1)]), 2);
        assert_eq!(at(&[("templates", 1), ("fill_sets", 1)]), 2);
        assert_eq!(at(&[("fill_sets", 1)]), 2, "filled from a global template: the first step is behind too");
        assert_eq!(at(&[("templates", 1), ("fill_sets", 1), ("documents", 2)]), 3);
    }

    #[test]
    fn only_a_sample_case_has_a_guide_until_it_is_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let own = app.create_case("Lease, Main Road").unwrap();
        assert!(app.sample_guide(own).unwrap().is_none());

        app.set_language("bg");
        let id = app.create_sample_case(Some("loan"), "bg").unwrap();
        let guide = app.sample_guide(id).unwrap().unwrap();
        assert_eq!((guide.sample.as_str(), guide.step, guide.steps.len()), ("loan", 0, 3));
        assert_eq!(guide.steps[0].title, "Поискайте шаблон");
        let shown = serde_json::to_value(&guide).unwrap();
        assert_eq!(shown["steps"][2]["action"], "open_fill");
        assert!(shown["steps"][2]["prompt"].is_null() && shown["steps"][0].get("done").is_none());

        let uuid = app.vault().get_case(id).unwrap().uuid;
        app.vault().hide_sample_guide(&uuid).unwrap();
        assert!(app.sample_guide(id).unwrap().is_none());
        assert!(app.vault().sample_cases().unwrap().contains_key(&uuid), "it is still a sample case");
    }
}
