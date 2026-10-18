//! Client of a Blankey relay (protocol v1, `relay/README.md`): lets web agents reach the local MCP server through
//! a WebSocket the app keeps open to the relay.
//!
//! `Relay` owns the settings, tokens and the one background task (sign-in, then the connection loop in `client`).
//! The user is asked through an `Approver`; state changes go out on a watch channel (`subscribe`), so the core
//! runs without Tauri.

mod client;
pub mod discovery;
pub mod oauth;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::Router;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::config::{Config, OFFICIAL_RELAY};
use crate::core::Blankey;
use crate::crypto;
use crate::i18n::mark;
use crate::keychain::SecretStore;
use crate::links::{InboxEntry, LinkDraft};
use crate::vault::{now, Link, RelayGrant};
use discovery::Discovery;
use oauth::{Client, HttpError, Metadata};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
const GRANTS_TIMEOUT: Duration = Duration::from_secs(5);
const SWITCH_REVOKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Submissions are up to 500 MB: no overall timeout, only for connecting and for each read.
const DOWNLOAD_READ_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    SignedOut,
    SigningIn,
    Connecting,
    Connected,
    Offline,
    AccountNeedsAttention,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::SignedOut => mark("Signed out"),
            Self::SigningIn => mark("Signing in…"),
            Self::Connecting => mark("Connecting"),
            Self::Connected => mark("Connected"),
            Self::Offline => mark("Offline"),
            Self::AccountNeedsAttention => mark("Account needs attention"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Status {
    pub state: State,
    pub message: String,
}

/// An agent asking to connect (`approve` frame).
#[derive(Clone, Debug)]
pub struct Approval {
    pub grant: String,
    pub client_name: String,
    pub redirect_host: String,
    pub relay_host: String,
}

/// Asks the user; blocking, called on a blocking thread.
pub trait Approver: Send + Sync {
    fn approve(&self, approval: &Approval) -> bool;
}

impl<F: Fn(&Approval) -> bool + Send + Sync> Approver for F {
    fn approve(&self, approval: &Approval) -> bool {
        self(approval)
    }
}

/// A grant as the relay lists it (`grants` frame).
#[derive(Clone, Debug, Deserialize)]
pub struct RemoteGrant {
    pub grant: String,
}

/// A collection link as the relay counts it (`GET /device/links`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RemoteLink {
    pub id: String,
    /// open, expired, revoked or full
    pub state: String,
    pub submissions: i64,
    pub max_submissions: i64,
}

#[derive(Serialize)]
pub struct GrantInfo {
    #[serde(flatten)]
    pub grant: RelayGrant,
    /// whether the relay still honours it; None when the relay could not be asked
    pub on_relay: Option<bool>,
}

/// What the sidebar shows; no network.
#[derive(Serialize)]
pub struct Summary {
    pub signed_in: bool,
    pub state: State,
    pub account: Option<String>,
}

#[derive(Serialize)]
pub struct Info {
    pub url: String,
    pub official: bool,
    pub signed_in: bool,
    pub account: Option<String>,
    #[serde(flatten)]
    pub status: Status,
    pub account_url: Option<String>,
    /// the address agents connect to
    pub mcp_url: Option<String>,
    pub grants: Vec<GrantInfo>,
}

#[derive(Clone)]
struct Endpoints {
    url: String,
    discovery: Discovery,
    metadata: Metadata,
}

/// Why the relay cannot be used right now.
enum Failure {
    /// no usable refresh token: the user has to sign in
    SignedOut(String),
    Offline(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Self::SignedOut(m) | Self::Offline(m) => m,
        }
    }
}

struct Inner {
    url: String,
    enabled: bool,
    /// a refresh token is stored; kept here so the tray does not read the keychain on every refresh
    has_token: bool,
    endpoints: Option<Endpoints>,
    access_token: Option<String>,
    task: Option<JoinHandle<()>>,
    /// frames to the relay while connected
    outgoing: Option<mpsc::UnboundedSender<String>>,
    grant_queries: HashMap<String, oneshot::Sender<Vec<RemoteGrant>>>,
    /// unlocked since the relay last confirmed this computer as primary
    primary_pending: bool,
}

impl Inner {
    fn send_primary_claim(&self) {
        if let (true, Some(outgoing)) = (self.primary_pending, &self.outgoing) {
            let _ = outgoing.send(serde_json::json!({"v": 1, "type": "primary", "id": "p-1"}).to_string());
        }
    }
}

type Browser = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct Relay {
    app: Arc<Blankey>,
    tokens: Box<dyn SecretStore + Sync>,
    approver: Arc<dyn Approver>,
    browser: Browser,
    runtime: Handle,
    http: ureq::Agent,
    /// presigned downloads of submissions
    download: ureq::Agent,
    /// one inbox drain at a time, so a submission is never taken twice
    inbox: tokio::sync::Mutex<()>,
    mcp: Router,
    callbacks: oauth::Callbacks,
    status: watch::Sender<Status>,
    /// the account's UI language from its device profile; None when signed out or the relay names none
    language: watch::Sender<Option<String>>,
    inner: Mutex<Inner>,
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work).await.expect("relay worker panicked")
}

impl Relay {
    /// `tokens` holds the refresh token; `runtime` runs the background task.
    pub fn new(
        app: Arc<Blankey>,
        tokens: Box<dyn SecretStore + Sync>,
        approver: Arc<dyn Approver>,
        runtime: Handle,
    ) -> Self {
        let config = &app.config;
        let inner = Inner {
            url: discovery::normalize(&config.relay_url).unwrap_or_else(|_| OFFICIAL_RELAY.into()),
            enabled: config.relay_enabled,
            has_token: tokens.get().ok().flatten().is_some(),
            endpoints: None,
            access_token: None,
            task: None,
            outgoing: None,
            grant_queries: HashMap::new(),
            primary_pending: false,
        };
        let mcp = crate::mcp::router(app.clone(), config.port);
        let status = Status { state: State::SignedOut, message: String::new() };
        let language = app.vault().relay_language().ok().flatten();
        Self {
            app,
            tokens,
            approver,
            browser: Box::new(|url| open::that(url).map_err(|e| e.to_string())),
            runtime,
            // few calls far apart: a pooled keep-alive connection the server has since closed would fail a POST
            http: ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).max_idle_connections(0).build(),
            download: ureq::AgentBuilder::new()
                .timeout_connect(HTTP_TIMEOUT)
                .timeout_read(DOWNLOAD_READ_TIMEOUT)
                .max_idle_connections(0)
                .build(),
            inbox: tokio::sync::Mutex::new(()),
            mcp,
            callbacks: oauth::Callbacks::default(),
            status: watch::channel(status).0,
            language: watch::channel(language).0,
            inner: Mutex::new(inner),
        }
    }

    /// How the sign-in page is opened; the system browser by default.
    pub fn with_browser(mut self, browser: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Self {
        self.browser = Box::new(browser);
        self
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }

    pub fn status(&self) -> Status {
        self.status.borrow().clone()
    }

    /// Changes of the account's UI language.
    pub fn subscribe_language(&self) -> watch::Receiver<Option<String>> {
        self.language.subscribe()
    }

    pub fn language(&self) -> Option<String> {
        self.language.borrow().clone()
    }

    fn set_language(&self, language: Option<String>) {
        let _ = self.app.vault().set_relay_language(language.as_deref());
        self.language.send_if_modified(|current| std::mem::replace(current, language.clone()) != language);
    }

    /// Reads the account's UI language from the relay's device profile; a relay without one names none.
    pub async fn refresh_profile(&self) {
        if !self.signed_in() {
            return;
        }
        let Ok(device) = self.device_id() else { return };
        #[derive(Deserialize)]
        struct Profile {
            #[serde(default)]
            lang: Option<String>,
        }
        let profile = self
            .authorized(move |agent, discovery, token| match discovery.device_profile.as_deref() {
                None => Ok(Profile { lang: None }),
                Some(base) => oauth::get_json::<Profile>(agent, &device_url(Some(base), &[], &device)?, token),
            })
            .await;
        match profile {
            Ok(profile) => self.set_language(profile.lang.filter(|lang| !lang.is_empty())),
            Err(e) => eprintln!("relay: cannot read the device profile: {}", e.message()),
        }
    }

    fn set_status(&self, state: State, message: impl Into<String>) {
        let status = Status { state, message: message.into() };
        self.status.send_if_modified(|current| std::mem::replace(current, status.clone()) != status);
    }

    pub fn url(&self) -> String {
        self.inner().url.clone()
    }

    pub fn signed_in(&self) -> bool {
        self.inner().has_token
    }

    fn store_token(&self, token: &str) -> Result<(), String> {
        self.tokens.set(token)?;
        self.inner().has_token = true;
        Ok(())
    }

    fn forget_token(&self) {
        let _ = self.tokens.delete();
        self.inner().has_token = false;
    }

    /// The local route the browser returns to after signing in; merged into the MCP server.
    pub fn callback_router(&self) -> Router {
        self.callbacks.router()
    }

    /// Connects when the relay is enabled and a refresh token is stored.
    pub fn start(self: &Arc<Self>) {
        if self.inner().enabled && self.signed_in() {
            let relay = self.clone();
            self.replace_task(async move { client::run(relay).await });
        } else {
            self.set_status(State::SignedOut, "");
        }
    }

    fn replace_task(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
        let handle = self.runtime.spawn(task);
        if let Some(old) = self.inner().task.replace(handle) {
            old.abort();
        }
    }

    fn stop(&self) {
        let mut inner = self.inner();
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        inner.outgoing = None;
        inner.grant_queries.clear();
    }

    /// Starts signing in through the system browser; the outcome shows in the status.
    pub fn login(self: &Arc<Self>) {
        self.stop();
        let relay = self.clone();
        self.replace_task(async move {
            match relay.sign_in().await {
                Ok(()) => client::run(relay).await,
                Err(e) => relay.set_status(State::SignedOut, e),
            }
        });
    }

    async fn sign_in(self: &Arc<Self>) -> Result<(), String> {
        self.set_status(State::SigningIn, "Finish signing in in your browser.");
        let endpoints = self.endpoints().await.map_err(Failure::message)?;
        let port = self.app.config.port;
        let client = match self.client().filter(|c| c.relay_url == endpoints.url && c.port == port) {
            Some(client) => client,
            None => {
                let (agent, metadata, url) = (self.http.clone(), endpoints.metadata.clone(), endpoints.url.clone());
                let client = blocking(move || oauth::register(&agent, &metadata, &url, port)).await?;
                let text = serde_json::to_string(&client).map_err(|e| e.to_string())?;
                self.app.vault().set_relay_client(Some(&text)).map_err(|e| e.to_string())?;
                client
            }
        };
        let pkce = oauth::pkce();
        let state = oauth::random_token();
        let code = self.callbacks.expect(&state);
        let result = async {
            let resource = endpoints.discovery.device_resource.as_deref();
            (self.browser)(&oauth::authorize_url(&endpoints.metadata, &client, &pkce.challenge, &state, resource)?)?;
            match tokio::time::timeout(SIGN_IN_TIMEOUT, code).await {
                Ok(Ok(code)) => code.map_err(|e| format!("Sign-in failed: {e}")),
                _ => Err("Sign-in timed out. Try again.".to_string()),
            }
        }
        .await;
        self.callbacks.forget(&state);
        let code = result?;
        let (agent, metadata) = (self.http.clone(), endpoints.metadata.clone());
        let tokens = blocking(move || oauth::exchange_code(&agent, &metadata, &client, &code, &pkce.verifier))
            .await
            .map_err(|e| format!("Sign-in failed: {e}"))?;
        let refresh = tokens.refresh_token.ok_or("The relay issued no refresh token.")?;
        self.store_token(&refresh)?;
        {
            let mut inner = self.inner();
            inner.access_token = Some(tokens.access_token);
            inner.enabled = true;
        }
        self.save_settings();
        self.app.audit("user", "relay_sign_in", &endpoints.url);
        Ok(())
    }

    pub fn logout(&self) {
        self.stop();
        self.forget_token();
        let _ = self.app.vault().set_relay_account(None);
        self.set_language(None);
        {
            let mut inner = self.inner();
            inner.access_token = None;
            inner.enabled = false;
        }
        self.save_settings();
        self.app.audit("user", "relay_sign_out", &self.url());
        self.set_status(State::SignedOut, "");
    }

    fn save_settings(&self) {
        let inner = self.inner();
        let config = Config { relay_url: inner.url.clone(), relay_enabled: inner.enabled, ..self.app.config.clone() };
        if let Err(e) = config.save() {
            eprintln!("relay: cannot save the settings: {e}");
        }
    }

    fn client(&self) -> Option<Client> {
        let text = self.app.vault().relay_client().ok().flatten()?;
        serde_json::from_str(&text).ok()
    }

    /// Discovery and OAuth metadata of the current relay, fetched once.
    async fn endpoints(&self) -> Result<Endpoints, Failure> {
        let url = {
            let inner = self.inner();
            if let Some(endpoints) = inner.endpoints.as_ref().filter(|e| e.url == inner.url) {
                return Ok(endpoints.clone());
            }
            inner.url.clone()
        };
        let endpoints = fetch_endpoints(self.http.clone(), url).await.map_err(Failure::Offline)?;
        let mut inner = self.inner();
        if inner.url == endpoints.url {
            inner.endpoints = Some(endpoints.clone());
        }
        Ok(endpoints)
    }

    /// The access token, refreshed when there is none.
    async fn access_token(&self) -> Result<String, Failure> {
        if let Some(token) = self.inner().access_token.clone() {
            return Ok(token);
        }
        let sign_in = || Failure::SignedOut("Sign in to the relay.".into());
        let refresh_token = self.tokens.get().ok().flatten().ok_or_else(sign_in)?;
        let client = self.client().ok_or_else(sign_in)?;
        let endpoints = self.endpoints().await?;
        let agent = self.http.clone();
        let result =
            blocking(move || oauth::refresh(&agent, &endpoints.metadata, &client.client_id, &refresh_token)).await;
        match result {
            Ok(tokens) => {
                if let Some(rotated) = &tokens.refresh_token {
                    let _ = self.store_token(rotated);
                }
                self.inner().access_token = Some(tokens.access_token.clone());
                Ok(tokens.access_token)
            }
            Err(HttpError::Status(400 | 401, _)) => {
                self.forget_token();
                Err(Failure::SignedOut("The relay ended the session. Sign in again.".into()))
            }
            Err(e) => Err(Failure::Offline(e.to_string())),
        }
    }

    fn drop_access_token(&self) {
        self.inner().access_token = None;
    }

    /// Calls a device endpoint with the access token, refreshing it once on 401.
    async fn authorized<T: Send + 'static>(
        &self,
        call: impl Fn(&ureq::Agent, &Discovery, &str) -> Result<T, HttpError> + Clone + Send + 'static,
    ) -> Result<T, Failure> {
        let mut retried = false;
        loop {
            let token = self.access_token().await?;
            let endpoints = self.endpoints().await?;
            let (agent, call) = (self.http.clone(), call.clone());
            match blocking(move || call(&agent, &endpoints.discovery, &token)).await {
                Err(HttpError::Status(401, _)) if !retried => {
                    retried = true;
                    self.drop_access_token();
                }
                result => return result.map_err(|e| Failure::Offline(e.to_string())),
            }
        }
    }

    /// Called on unlock: the relay sends agents to the computer unlocked last. Sent now when connected, else on the
    /// next connection; a reconnect after the relay confirmed it does not claim again.
    pub fn claim_primary(&self) {
        let mut inner = self.inner();
        inner.primary_pending = true;
        inner.send_primary_claim();
    }

    /// Accepted agents on the current relay.
    pub fn grants(&self) -> Vec<RelayGrant> {
        let url = self.url();
        self.app.vault().relay_grants(&url).unwrap_or_default()
    }

    /// Removes the grant here first, so it is refused even if telling the relay fails.
    pub async fn revoke(&self, grant_id: &str) -> Result<(), String> {
        self.app.vault().revoke_relay_grant(grant_id).map_err(|e| e.to_string())?;
        self.app.audit("user", "relay_revoke", grant_id);
        let grant = grant_id.to_string();
        let result =
            self.authorized(move |agent, discovery, token| oauth::revoke_grant(agent, discovery, token, &grant));
        if let Err(e) = result.await {
            eprintln!("relay: the relay was not told about the revoked grant {grant_id}: {}", e.message());
        }
        Ok(())
    }

    /// The relay's own list of grants, when connected.
    pub async fn remote_grants(&self) -> Option<Vec<RemoteGrant>> {
        let id = format!("q-{}", oauth::random_token());
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.inner();
            let outgoing = inner.outgoing.clone()?;
            inner.grant_queries.insert(id.clone(), tx);
            outgoing.send(serde_json::json!({"v": 1, "type": "grants", "id": id}).to_string()).ok()?;
        }
        let grants = tokio::time::timeout(GRANTS_TIMEOUT, rx).await.ok().and_then(Result::ok);
        self.inner().grant_queries.remove(&id);
        grants
    }

    pub async fn account_url(&self) -> Option<String> {
        self.endpoints().await.ok().map(|e| e.discovery.account_url)
    }

    /// The account page if it is already known; no network.
    pub fn cached_account_url(&self) -> Option<String> {
        self.inner().endpoints.as_ref().map(|e| e.discovery.account_url.clone())
    }

    pub fn summary(&self) -> Summary {
        Summary { signed_in: self.signed_in(), state: self.status().state, account: self.account() }
    }

    fn account(&self) -> Option<String> {
        self.app.vault().relay_account().ok().flatten()
    }

    /// Everything the Connections page shows.
    pub async fn info(&self) -> Info {
        let url = self.url();
        let remote = match self.status().state {
            State::Connected => self.remote_grants().await,
            _ => None,
        };
        let grants = self
            .grants()
            .into_iter()
            .map(|grant| {
                let on_relay = remote.as_ref().map(|r| r.iter().any(|g| g.grant == grant.grant_id));
                GrantInfo { grant, on_relay }
            })
            .collect();
        let discovery = self.endpoints().await.ok().map(|e| e.discovery);
        Info {
            official: url == OFFICIAL_RELAY,
            signed_in: self.signed_in(),
            account: self.account(),
            status: self.status(),
            account_url: discovery.as_ref().map(|d| d.account_url.clone()),
            mcp_url: discovery.map(|d| d.mcp_url),
            grants,
            url,
        }
    }

    // -- collection links ----------------------------------------------------------------------------------

    fn device_id(&self) -> Result<String, String> {
        self.app.vault().relay_device_id().ok().flatten().ok_or_else(|| "Sign in to the relay first.".to_string())
    }

    /// Makes the link's key pair, registers the public key and spec on the relay and keeps the private key in the
    /// vault. The stored URL carries the key's fingerprint in its fragment, which the relay never sees.
    pub async fn create_link(&self, draft: &LinkDraft) -> Result<Link, String> {
        let spec = draft.spec()?;
        if !self.app.vault().unlocked() {
            return Err("Vault is locked".into());
        }
        let device = self.device_id()?;
        let (private, public) = crypto::new_seal_keypair();
        let expires_at = draft.expires_at();
        let body = json!({
            "public_key": URL_SAFE_NO_PAD.encode(public),
            "spec": spec,
            "expires_at": expires_at,
            "max_submissions": draft.max_submissions,
        });
        #[derive(Deserialize)]
        struct Created {
            id: String,
            url: String,
        }
        let call_device = device.clone();
        let created: Created = self
            .authorized(move |agent, discovery, token| {
                let url = device_url(discovery.device_links.as_deref(), &[], &call_device)?;
                oauth::parse(&oauth::post_json(agent, &url, token, &body)?)
            })
            .await
            .map_err(link_failure)?;
        discovery::check_url(&created.url, "https", "http")?;
        let link = Link {
            link_id: created.id.clone(),
            relay_url: self.url(),
            url: format!("{}#{}", created.url, crypto::fingerprint(&public)),
            title: spec.title.clone(),
            spec: serde_json::to_value(&spec).map_err(|e| e.to_string())?,
            profile_id: draft.profile_id,
            new_profile: draft.new_profile.clone().filter(|_| draft.profile_id.is_none()),
            created_at: now(),
            expires_at,
            max_submissions: draft.max_submissions as i64,
            revoked_at: None,
            received: 0,
        };
        let stored = self.app.vault().add_link(&link, private.as_ref());
        if let Err(e) = stored {
            // nothing here could open what the link receives
            let _ = self.delete_remote_link(device, created.id).await;
            return Err(e.to_string());
        }
        self.app.audit("user", "create_link", &link.link_id);
        Ok(link)
    }

    /// This device's links with the relay's counts.
    pub async fn remote_links(&self) -> Result<Vec<RemoteLink>, String> {
        let device = self.device_id()?;
        #[derive(Deserialize)]
        struct Links {
            links: Vec<RemoteLink>,
        }
        self.authorized(move |agent, discovery, token| {
            let url = device_url(discovery.device_links.as_deref(), &[], &device)?;
            Ok(oauth::get_json::<Links>(agent, &url, token)?.links)
        })
        .await
        .map_err(Failure::message)
    }

    /// Drops the private key here first, so nothing sent to the link opens even if telling the relay fails.
    pub async fn revoke_link(&self, link_id: &str) -> Result<(), String> {
        self.app.vault().revoke_link(link_id).map_err(|e| e.to_string())?;
        self.app.audit("user", "revoke_link", link_id);
        if let Ok(device) = self.device_id() {
            if let Err(e) = self.delete_remote_link(device, link_id.to_string()).await {
                eprintln!("relay: the relay was not told about the revoked link {link_id}: {}", e.message());
            }
        }
        Ok(())
    }

    async fn delete_remote_link(&self, device: String, link_id: String) -> Result<(), Failure> {
        self.authorized(move |agent, discovery, token| {
            let url = device_url(discovery.device_links.as_deref(), &[&link_id], &device)?;
            match oauth::delete(agent, &url, token) {
                Err(HttpError::Status(404, _)) => Ok(()),
                result => result,
            }
        })
        .await
    }

    /// Pulls every waiting submission: downloads it (still sealed), deletes it on the relay, then creates the
    /// request. A failure before the delete leaves the relay copy for the next drain.
    pub(crate) async fn drain_inbox(&self) {
        let _draining = self.inbox.lock().await;
        let Ok(device) = self.device_id() else { return };
        #[derive(Deserialize)]
        struct Inbox {
            inbox: Vec<InboxEntry>,
        }
        let list_device = device.clone();
        let listed = self
            .authorized(move |agent, discovery, token| {
                if discovery.device_inbox.is_none() {
                    return Ok(Vec::new());
                }
                let url = device_url(discovery.device_inbox.as_deref(), &[], &list_device)?;
                Ok(oauth::get_json::<Inbox>(agent, &url, token)?.inbox)
            })
            .await;
        let entries = match listed {
            Ok(entries) => entries,
            Err(e) => return eprintln!("relay: cannot list the inbox: {}", e.message()),
        };
        let listed: Vec<String> = entries.iter().map(|e| e.sid.clone()).collect();
        for entry in entries {
            let (app, agent, pulled) = (self.app.clone(), self.download.clone(), entry.clone());
            if let Err(e) = blocking(move || app.download_submission(&agent, &pulled)).await {
                eprintln!("relay: cannot download submission {}: {e}", entry.sid);
                continue;
            }
            let (device, sid) = (device.clone(), entry.sid.clone());
            let deleted = self
                .authorized(move |agent, discovery, token| {
                    let url = device_url(discovery.device_inbox.as_deref(), &[&sid], &device)?;
                    match oauth::delete(agent, &url, token) {
                        Err(HttpError::Status(404, _)) => Ok(()),
                        result => result,
                    }
                })
                .await;
            if let Err(e) = deleted {
                eprintln!("relay: cannot delete submission {} on the relay: {}", entry.sid, e.message());
                continue;
            }
            let app = self.app.clone();
            let accepted = blocking(move || app.accept_submission(&entry.link_id, &entry.sid)).await;
            if let Err(e) = accepted {
                eprintln!("relay: cannot store a submission: {e}");
            }
        }
        // downloaded earlier, gone from the relay (its delete went through even if the answer was lost), no request yet
        for (link_id, sid) in self.app.unaccepted_submissions() {
            if listed.contains(&sid) {
                continue;
            }
            let app = self.app.clone();
            if let Err(e) = blocking(move || app.accept_submission(&link_id, &sid)).await {
                eprintln!("relay: cannot store a submission: {e}");
            }
        }
    }

    /// Moves to another relay: checks its discovery document first, then forgets the old relay (grants, device,
    /// client, token; grants are revoked there best-effort) and starts signing in. None means the official relay.
    pub async fn set_url(self: &Arc<Self>, url: Option<&str>) -> Result<(), String> {
        let url = discovery::normalize(url.unwrap_or(OFFICIAL_RELAY))?;
        let old = self.url();
        if url == old {
            return Ok(());
        }
        let endpoints = fetch_endpoints(self.http.clone(), url.clone()).await?;

        let revoke_all = async {
            for grant in self.grants() {
                let id = grant.grant_id;
                let _ = self
                    .authorized(move |agent, discovery, token| oauth::revoke_grant(agent, discovery, token, &id))
                    .await;
            }
        };
        let _ = tokio::time::timeout(SWITCH_REVOKE_TIMEOUT, revoke_all).await;
        self.stop();
        {
            let vault = self.app.vault();
            vault.clear_relay_grants(&old).map_err(|e| e.to_string())?;
            vault.set_relay_device_id(None).map_err(|e| e.to_string())?;
            vault.set_relay_client(None).map_err(|e| e.to_string())?;
            vault.set_relay_account(None).map_err(|e| e.to_string())?;
        }
        self.set_language(None);
        self.forget_token();
        {
            let mut inner = self.inner();
            inner.url = url.clone();
            inner.enabled = false;
            inner.endpoints = Some(endpoints);
            inner.access_token = None;
        }
        self.save_settings();
        self.app.audit("user", "relay_switch", &url);
        self.login();
        Ok(())
    }
}

/// `{base}/{segments}?device_id=…`: a link and its submissions belong to the device that holds the key.
fn device_url(base: Option<&str>, segments: &[&str], device_id: &str) -> Result<String, HttpError> {
    let base = base.ok_or_else(|| HttpError::Transport("This relay does not offer collection links.".into()))?;
    let mut url = url::Url::parse(base).map_err(|e| HttpError::Transport(e.to_string()))?;
    if !segments.is_empty() {
        url.path_segments_mut().map_err(|_| HttpError::Transport(format!("Bad relay URL {base}")))?.extend(segments);
    }
    url.query_pairs_mut().append_pair("device_id", device_id);
    Ok(url.into())
}

fn link_failure(failure: Failure) -> String {
    let message = failure.message();
    if message.contains("account_needs_attention") {
        "Your relay account needs attention. Open Account and billing on the Connections page.".into()
    } else {
        message
    }
}

async fn fetch_endpoints(agent: ureq::Agent, url: String) -> Result<Endpoints, String> {
    blocking(move || {
        let discovery = discovery::fetch(&agent, &url)?;
        let metadata = oauth::metadata(&agent, &discovery)?;
        Ok(Endpoints { url, discovery, metadata })
    })
    .await
}
