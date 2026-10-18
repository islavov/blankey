//! The app as an OAuth client of the relay (scope `device`): dynamic registration, authorization code + PKCE
//! through the system browser, refresh, and the device endpoints. Blocking (ureq); `Relay` runs these on blocking
//! threads.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::Query;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use url::Url;

use super::discovery::{check_url, Discovery};

pub const CALLBACK_PATH: &str = "/relay/callback";
const SCOPE: &str = "device";

#[derive(Debug)]
pub enum HttpError {
    Status(u16, String),
    Transport(String),
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(code, body) => {
                write!(f, "the online server answered {code}: {}", body.chars().take(300).collect::<String>())
            }
            Self::Transport(e) => write!(f, "{e}"),
        }
    }
}

type Http<T> = Result<T, HttpError>;

fn read(result: Result<ureq::Response, ureq::Error>) -> Http<String> {
    match result {
        Ok(response) => response.into_string().map_err(|e| HttpError::Transport(e.to_string())),
        Err(ureq::Error::Status(code, response)) => {
            Err(HttpError::Status(code, response.into_string().unwrap_or_default()))
        }
        Err(e) => Err(HttpError::Transport(e.to_string())),
    }
}

pub(super) fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Http<T> {
    serde_json::from_str(text).map_err(|e| HttpError::Transport(format!("unexpected answer from the relay: {e}")))
}

/// RFC 8414 authorization server metadata.
#[derive(Clone, Debug, Deserialize)]
pub struct Metadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
}

pub fn metadata(agent: &ureq::Agent, discovery: &Discovery) -> Result<Metadata, String> {
    let url = format!("{}/.well-known/oauth-authorization-server", discovery.issuer.trim_end_matches('/'));
    let metadata: Metadata = read(agent.get(&url).call()).and_then(|text| parse(&text)).map_err(|e| e.to_string())?;
    for url in [&metadata.authorization_endpoint, &metadata.token_endpoint] {
        check_url(url, "https", "http")?;
    }
    let methods = &metadata.code_challenge_methods_supported;
    if !methods.is_empty() && !methods.iter().any(|m| m == "S256") {
        return Err("The online server does not support PKCE S256.".into());
    }
    Ok(metadata)
}

/// The app's client registration, kept per relay and port: the redirect URI contains the port.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Client {
    pub relay_url: String,
    pub port: u16,
    pub client_id: String,
}

impl Client {
    pub fn redirect_uri(&self) -> String {
        redirect_uri(self.port)
    }
}

fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}{CALLBACK_PATH}")
}

pub fn register(agent: &ureq::Agent, metadata: &Metadata, relay_url: &str, port: u16) -> Result<Client, String> {
    let endpoint =
        metadata.registration_endpoint.as_deref().ok_or("The online server does not allow client registration.")?;
    let body = json!({
        "client_name": "Blankey",
        "redirect_uris": [redirect_uri(port)],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
        "scope": SCOPE,
    });
    let reply: Value =
        read(agent.post(endpoint).set("Content-Type", "application/json").send_string(&body.to_string()))
            .and_then(|text| parse(&text))
            .map_err(|e| format!("Registration failed: {e}"))?;
    let client_id = reply["client_id"].as_str().ok_or("The online server returned no client_id.")?;
    Ok(Client { relay_url: relay_url.into(), port, client_id: client_id.into() })
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn pkce() -> Pkce {
    let verifier = random_token();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge }
}

/// `resource` is the relay's `device_resource`, when it names one.
pub fn authorize_url(
    metadata: &Metadata,
    client: &Client,
    challenge: &str,
    state: &str,
    resource: Option<&str>,
) -> Result<String, String> {
    let mut url = Url::parse(&metadata.authorization_endpoint).map_err(|e| e.to_string())?;
    if let Some(resource) = resource {
        url.query_pairs_mut().append_pair("resource", resource);
    }
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client.client_id)
        .append_pair("redirect_uri", &client.redirect_uri())
        .append_pair("scope", SCOPE)
        .append_pair("state", state)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(url.into())
}

#[derive(Debug, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

pub fn exchange_code(
    agent: &ureq::Agent,
    metadata: &Metadata,
    client: &Client,
    code: &str,
    verifier: &str,
) -> Http<Tokens> {
    let redirect = client.redirect_uri();
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect),
        ("client_id", &client.client_id),
        ("code_verifier", verifier),
    ];
    read(agent.post(&metadata.token_endpoint).send_form(&form)).and_then(|text| parse(&text))
}

pub fn refresh(agent: &ureq::Agent, metadata: &Metadata, client_id: &str, refresh_token: &str) -> Http<Tokens> {
    let form = [("grant_type", "refresh_token"), ("refresh_token", refresh_token), ("client_id", client_id)];
    read(agent.post(&metadata.token_endpoint).send_form(&form)).and_then(|text| parse(&text))
}

fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

pub(super) fn get_json<T: serde::de::DeserializeOwned>(agent: &ureq::Agent, url: &str, token: &str) -> Http<T> {
    parse(&read(agent.get(url).set("Authorization", &format!("Bearer {token}")).call())?)
}

/// Only the status counts: reading the empty body of a 204 through some proxies fails with "unexpected end of file".
pub(super) fn delete(agent: &ureq::Agent, url: &str, token: &str) -> Http<()> {
    match agent.delete(url).set("Authorization", &format!("Bearer {token}")).call() {
        Ok(_) => Ok(()),
        Err(e) => read(Err(e)).map(|_| ()),
    }
}

pub(super) fn post_json(agent: &ureq::Agent, url: &str, token: &str, body: &Value) -> Http<String> {
    read(
        agent
            .post(url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_string(&body.to_string()),
    )
}

/// Registers this computer; with the stored id the relay keeps the same device.
/// `device_register`'s answer.
pub struct Registration {
    pub device_id: String,
    /// the signed-in account, when the relay names it
    pub account: Option<String>,
}

pub fn register_device(
    agent: &ureq::Agent,
    discovery: &Discovery,
    token: &str,
    device_id: Option<&str>,
) -> Http<Registration> {
    let mut body = json!({"name": gethostname::gethostname().to_string_lossy(), "platform": platform()});
    if let Some(id) = device_id {
        body["device_id"] = json!(id);
    }
    let reply: Value = parse(&post_json(agent, &discovery.device_register, token, &body)?)?;
    let device_id = reply["device_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| HttpError::Transport("the online server returned no device_id".into()))?;
    Ok(Registration { device_id, account: reply["account"].as_str().map(str::to_string) })
}

pub fn revoke_grant(agent: &ureq::Agent, discovery: &Discovery, token: &str, grant: &str) -> Http<()> {
    post_json(agent, &discovery.device_revoke, token, &json!({"grant": grant})).map(|_| ())
}

type Waiting = HashMap<String, oneshot::Sender<Result<String, String>>>;

/// Sign-ins waiting for the browser to come back to `/relay/callback`, by `state`.
#[derive(Clone, Default)]
pub struct Callbacks(Arc<Mutex<Waiting>>);

#[derive(Deserialize)]
struct CallbackQuery {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

impl Callbacks {
    pub fn expect(&self, state: &str) -> oneshot::Receiver<Result<String, String>> {
        let (tx, rx) = oneshot::channel();
        self.0.lock().unwrap().insert(state.into(), tx);
        rx
    }

    pub fn forget(&self, state: &str) {
        self.0.lock().unwrap().remove(state);
    }

    fn resolve(&self, query: CallbackQuery) -> bool {
        let Some(tx) = query.state.and_then(|state| self.0.lock().unwrap().remove(&state)) else { return false };
        let result = match (query.code, query.error) {
            (Some(code), None) => Ok(code),
            (_, error) => Err(query.error_description.or(error).unwrap_or_else(|| "no code".into())),
        };
        let ok = result.is_ok();
        let _ = tx.send(result);
        ok
    }

    /// `/relay/callback` for the local server.
    pub fn router(&self) -> Router {
        let callbacks = self.clone();
        Router::new().route(
            CALLBACK_PATH,
            get(move |Query(query): Query<CallbackQuery>| async move {
                let text = if callbacks.resolve(query) {
                    "Blankey is signed in to Blankey Online. You can close this tab."
                } else {
                    "This sign-in did not complete. Start it again from Blankey."
                };
                Html(format!("<!doctype html><meta charset=utf-8><title>Blankey</title><p>{text}</p>"))
            }),
        )
    }
}
