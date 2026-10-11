//! Template store: `templates/<id>/manifest.yaml` plus the source file and plaintext example datasets.

pub mod bindings;
pub mod docx;
pub mod engine;
pub mod fill;
pub mod preview;
pub mod typst;

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub static ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z0-9][a-z0-9_-]{0,63}$").unwrap());

pub type StrMap = IndexMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    PdfForm,
    Typst,
    Docx,
}

impl TemplateKind {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "pdf_form" => Ok(Self::PdfForm),
            "typst" => Ok(Self::Typst),
            "docx" => Ok(Self::Docx),
            other => Err(format!("Unknown template kind {other:?}; use pdf_form, typst or docx")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PdfForm => "pdf_form",
            Self::Typst => "typst",
            Self::Docx => "docx",
        }
    }

    pub fn source_name(self) -> &'static str {
        match self {
            Self::PdfForm => "form.pdf",
            Self::Typst => "main.typ",
            Self::Docx => "template.docx",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Template {
    pub id: String,
    pub name: String,
    pub kind: TemplateKind,
    pub directory: PathBuf,
    /// role -> description
    pub roles: StrMap,
    /// field / variable -> Jinja expression
    pub fields: StrMap,
    pub description: String,
    /// variable -> human label
    pub labels: StrMap,
    /// fields drawn on a flat PDF (see flat.pdf)
    pub boxes: Vec<Value>,
}

#[derive(Serialize)]
struct ManifestOut<'a> {
    id: &'a str,
    name: &'a str,
    kind: &'a str,
    description: &'a str,
    roles: &'a StrMap,
    fields: &'a StrMap,
    labels: &'a StrMap,
    #[serde(skip_serializing_if = "<[Value]>::is_empty")]
    boxes: &'a [Value],
}

impl Template {
    pub fn source_path(&self) -> PathBuf {
        self.directory.join(self.kind.source_name())
    }

    pub fn source(&self) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.source_path())
    }

    pub fn manifest(&self) -> Value {
        serde_json::to_value(self.manifest_out()).expect("manifest serializes")
    }

    fn manifest_out(&self) -> ManifestOut<'_> {
        ManifestOut {
            id: &self.id,
            name: &self.name,
            kind: self.kind.as_str(),
            description: &self.description,
            roles: &self.roles,
            fields: &self.fields,
            labels: &self.labels,
            boxes: &self.boxes,
        }
    }
}

/// YAML scalars as the Python app read them with str(); null becomes "".
fn scalar_text(value: &serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(true) => "True".into(),
        serde_yaml::Value::Bool(false) => "False".into(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Null => String::new(),
        other => serde_yaml::to_string(other).unwrap_or_default().trim().to_string(),
    }
}

fn str_map(value: Option<&serde_yaml::Value>) -> StrMap {
    match value {
        Some(serde_yaml::Value::Mapping(map)) => map.iter().map(|(k, v)| (scalar_text(k), scalar_text(v))).collect(),
        _ => StrMap::new(),
    }
}

pub fn yaml_dump<T: Serialize>(data: &T) -> String {
    serde_yaml::to_string(data).unwrap_or_default()
}

pub struct TemplateStore {
    root: PathBuf,
}

pub struct SaveTemplate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub kind: TemplateKind,
    pub roles: StrMap,
    pub fields: StrMap,
    pub description: &'a str,
    pub source_file: Option<&'a Path>,
    pub source_bytes: Option<&'a [u8]>,
    pub source_text: Option<&'a str>,
    pub labels: StrMap,
    pub boxes: Vec<Value>,
}

impl TemplateStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn all(&self) -> Vec<Template> {
        let Ok(entries) = std::fs::read_dir(&self.root) else { return Vec::new() };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter(|e| e.path().join("manifest.yaml").exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        ids.sort();
        ids.iter().filter_map(|id| self.get(id).ok()).collect()
    }

    pub fn get(&self, id: &str) -> Result<Template, String> {
        let unknown = || format!("Unknown template {id:?}");
        if !ID_RE.is_match(id) {
            return Err(unknown());
        }
        let directory = self.root.join(id);
        let text = std::fs::read_to_string(directory.join("manifest.yaml")).map_err(|_| unknown())?;
        let data: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| format!("{id}: {e}"))?;
        let kind = TemplateKind::parse(data.get("kind").and_then(|k| k.as_str()).unwrap_or(""))?;
        let boxes = data
            .get("boxes")
            .and_then(|b| serde_json::to_value(b).ok())
            .and_then(|b| b.as_array().cloned())
            .unwrap_or_default();
        Ok(Template {
            id: id.to_string(),
            name: data.get("name").map(scalar_text).filter(|n| !n.is_empty()).unwrap_or_else(|| id.to_string()),
            kind,
            directory,
            roles: str_map(data.get("roles")),
            fields: str_map(data.get("fields")),
            description: data.get("description").map(scalar_text).unwrap_or_default(),
            labels: str_map(data.get("labels")),
            boxes,
        })
    }

    pub fn save(&self, spec: SaveTemplate) -> Result<Template, String> {
        if !ID_RE.is_match(spec.id) {
            return Err("Template id must be lowercase letters, digits, '-' or '_'".into());
        }
        let template = Template {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            kind: spec.kind,
            directory: self.root.join(spec.id),
            roles: spec.roles,
            fields: spec.fields,
            description: spec.description.to_string(),
            labels: spec.labels,
            boxes: spec.boxes,
        };
        std::fs::create_dir_all(&template.directory).map_err(|e| e.to_string())?;
        let target = template.source_path();
        if let Some(source) = spec.source_file {
            std::fs::copy(source, &target).map_err(|e| format!("{}: {e}", source.display()))?;
        } else if let Some(bytes) = spec.source_bytes {
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
        } else if let Some(text) = spec.source_text {
            std::fs::write(&target, text).map_err(|e| e.to_string())?;
        }
        if !target.exists() {
            return Err(format!("Template {:?} has no source; pass source_path or source_text", spec.id));
        }
        std::fs::write(template.directory.join("manifest.yaml"), yaml_dump(&template.manifest_out()))
            .map_err(|e| e.to_string())?;
        Ok(template)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        std::fs::remove_dir_all(self.get(id)?.directory).map_err(|e| e.to_string())
    }

    pub fn examples(&self, id: &str) -> Result<Vec<String>, String> {
        let directory = self.get(id)?.directory.join("examples");
        let mut names: Vec<String> = std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter_map(|n| n.strip_suffix(".yaml").map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        Ok(names)
    }

    pub fn get_example(&self, id: &str, name: &str) -> Result<Value, String> {
        let path = self.get(id)?.directory.join("examples").join(format!("{name}.yaml"));
        if !ID_RE.is_match(name) || !path.exists() {
            return Err(format!("Unknown example {name:?} for template {id:?}"));
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let data: Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
        Ok(if data.is_null() { Value::Object(Default::default()) } else { data })
    }

    pub fn save_example(&self, id: &str, name: &str, data: &Value) -> Result<(), String> {
        if !ID_RE.is_match(name) {
            return Err("Example name must be lowercase letters, digits, '-' or '_'".into());
        }
        let directory = self.get(id)?.directory.join("examples");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        std::fs::write(directory.join(format!("{name}.yaml")), yaml_dump(data)).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn save_get_and_examples() {
        let dir = tempfile::tempdir().unwrap();
        let store = TemplateStore::new(dir.path().to_path_buf());
        let fields: StrMap = [("name".into(), "{{ applicant.name }}".into())].into_iter().collect();
        let template = store
            .save(SaveTemplate {
                id: "t",
                name: "Т",
                kind: TemplateKind::Typst,
                roles: [("applicant".into(), "person".into())].into_iter().collect(),
                fields,
                description: "",
                source_file: None,
                source_bytes: None,
                source_text: Some("#let d = json(bytes(sys.inputs.data))"),
                labels: StrMap::new(),
                boxes: vec![],
            })
            .unwrap();
        let loaded = store.get("t").unwrap();
        assert_eq!(loaded.fields, template.fields);
        assert_eq!(loaded.name, "Т");
        store.save_example("t", "ex", &json!({"applicant": {"name": "Иван"}})).unwrap();
        assert_eq!(store.examples("t").unwrap(), ["ex"]);
        assert_eq!(store.get_example("t", "ex").unwrap()["applicant"]["name"], "Иван");
        assert!(store.get("../x").is_err());
        assert_eq!(store.all().len(), 1);
    }

    #[test]
    fn reads_python_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("main.typ"), "").unwrap();
        std::fs::write(
            path.join("manifest.yaml"),
            "id: x\nname: Договор\nkind: typst\nroles:\n  seller: company\nfields:\n  price: ''\n  count: 3\n  empty:\nlabels: {}\n",
        )
        .unwrap();
        let template = TemplateStore::new(dir.path().to_path_buf()).get("x").unwrap();
        assert_eq!(template.fields["count"], "3");
        assert_eq!(template.fields["empty"], "");
        assert_eq!(template.roles["seller"], "company");
    }
}
