//! The sample case of a first use: an invented loan contract and the two invented people in it, in the UI language.
//! The contract names exactly the values the profiles hold, so an assistant can bind them when it makes a template.

use serde::Deserialize;

use crate::core::Blankey;
use crate::recognize::recognize;
use crate::vault::{FieldInput, NewFile};

/// (language, case.yaml, contract.docx)
const SAMPLES: [(&str, &str, &[u8]); 2] = [
    ("en", include_str!("en/case.yaml"), include_bytes!("en/contract.docx")),
    ("bg", include_str!("bg/case.yaml"), include_bytes!("bg/contract.docx")),
];

#[derive(Deserialize)]
struct Sample {
    title: String,
    file: SampleFile,
    profiles: Vec<SampleProfile>,
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

impl Blankey {
    /// Makes the sample case in `language` (English when there is none in it) and ends the first use. Returns its
    /// id. Needs the vault unlocked.
    pub fn create_sample_case(&self, language: &str) -> Result<i64, String> {
        let (_, yaml, contract) = SAMPLES.iter().find(|(code, ..)| *code == language).unwrap_or(&SAMPLES[0]);
        let sample: Sample = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        let id = self.create_case(&sample.title)?;
        let seen = recognize(&sample.file.name, contract);
        let file = NewFile {
            profile_id: None,
            link_id: "",
            case_id: Some(id),
            name: &sample.file.name,
            file_type: seen.kind.media_type(),
            description: &sample.file.description,
            sha256: &seen.sha256,
        };
        self.vault().add_collected_file(file, contract).map_err(|e| e.to_string())?;
        for profile in &sample.profiles {
            let profile_id = self.create_case_profile(id, &profile.name, &profile.kind, &profile.role)?;
            let values: Vec<FieldInput> =
                profile.fields.iter().map(|f| FieldInput::new(&f.key, &f.value, &f.label, "text")).collect();
            self.vault().set_values(profile_id, &values).map_err(|e| e.to_string())?;
        }
        let vault = self.vault();
        let uuid = vault.get_case(id).map_err(|e| e.to_string())?.uuid;
        vault.set_sample_case(&uuid).map_err(|e| e.to_string())?;
        vault.set_first_use(None).map_err(|e| e.to_string())?;
        Ok(id)
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
            let id = app.create_sample_case(language).unwrap();

            let vault = app.vault();
            let case = vault.get_case(id).unwrap();
            assert_eq!(case.title, title);
            assert_eq!(vault.sample_case().unwrap().as_deref(), Some(case.uuid.as_str()));
            assert_eq!(vault.first_use().unwrap(), None);

            let files: Vec<_> =
                vault.list_collected_files().unwrap().into_iter().filter(|f| f.case_id == Some(id)).collect();
            assert_eq!(files.len(), 1);
            let (_, content) = vault.load_collected_file(files[0].id).unwrap();
            assert!(SAMPLES.iter().any(|(code, _, docx)| *code == language && *docx == content.as_slice()));
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
        let id = app.create_sample_case("de").unwrap();
        assert_eq!(app.vault().get_case(id).unwrap().title, "Sample: loan contract");
    }

    #[test]
    fn the_contract_names_what_the_profiles_hold() {
        for (_, yaml, docx) in SAMPLES {
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
}
