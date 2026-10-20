//! What the vault stores and returns.

use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Serialize, Clone, Debug)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub kind: String,
}

/// Metadata only: safe for MCP.
#[derive(Serialize, Clone, Debug)]
pub struct FieldInfo {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub length: i64,
}

#[derive(Serialize)]
pub struct Field {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    pub length: i64,
    pub value: Option<String>,
}

#[derive(Clone, Debug)]
pub struct FieldInput {
    pub key: String,
    pub value: String,
    pub label: String,
    pub field_type: String,
}

impl FieldInput {
    pub fn new(key: &str, value: &str, label: &str, field_type: &str) -> Self {
        Self { key: key.into(), value: value.into(), label: label.into(), field_type: field_type.into() }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct DocumentInfo {
    pub id: i64,
    pub template_id: String,
    pub title: String,
    pub profiles: Map<String, Value>,
    pub pages: i64,
    pub filename: String,
    pub created_at: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct FillSetInfo {
    pub id: i64,
    pub name: String,
    pub templates: Vec<String>,
    pub profiles: Profiles,
    pub updated_at: String,
}

/// role -> profile id
pub type Profiles = indexmap::IndexMap<String, i64>;
/// var -> {"path": "role.key"} | {"path": "key", "profile": id} | {"value": "..."} | {"default": true}
pub type Bindings = Map<String, Value>;

#[derive(Serialize, Clone, Debug)]
pub struct Request {
    pub id: i64,
    pub kind: String,
    pub payload: Map<String, Value>,
    pub status: String,
    pub result: Option<Value>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub grant_id: Option<String>,
    /// payload sealed and the vault is locked
    pub hidden: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RelayGrant {
    pub grant_id: String,
    pub relay_url: String,
    pub client_name: String,
    pub redirect_host: String,
    pub accepted_at: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Link {
    pub link_id: String,
    pub relay_url: String,
    /// the short URL with the key fingerprint in its fragment
    pub url: String,
    pub title: String,
    pub spec: Value,
    pub profile_id: Option<i64>,
    /// name of the profile to create when the link has no profile_id
    pub new_profile: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub max_submissions: i64,
    pub revoked_at: Option<String>,
    /// submissions this computer has pulled from the relay
    pub received: i64,
}

/// A submission to a collection link. status: pending, saved or discarded.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SubmissionInfo {
    pub sid: String,
    pub link_id: String,
    pub request_id: Option<i64>,
    pub size: i64,
    pub received_at: String,
    pub status: String,
    pub handled_at: Option<String>,
    /// the profile it was last saved into
    pub profile_id: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CollectedFile {
    pub id: i64,
    pub profile_id: Option<i64>,
    pub link_id: String,
    pub size: i64,
    pub created_at: String,
    pub name: String,
    #[serde(rename = "type")]
    pub file_type: String,
}
