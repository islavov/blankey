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
use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::config::{Config, OFFICIAL_RELAY};
use crate::core::Blankey;
use crate::keychain::SecretStore;
use crate::vault::RelayGrant;
use discovery::Discovery;
use oauth::{Client, HttpError, Metadata};

const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(600);
const GRANTS_TIMEOUT: Duration = Duration::from_secs(5);
const SWITCH_REVOKE_TIMEOUT: Duration = Duration::from_secs(10);

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
            Self::SignedOut => "Signed out",
            Self::SigningIn => "Signing in…",
            Self::Connecting => "Connecting",
            Self::Connected => "Connected",
            Self::Offline => "Offline",
            Self::AccountNeedsAttention => "Account needs attention",
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
    mcp: Router,
    callbacks: oauth::Callbacks,
    status: watch::Sender<Status>,
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
        Self {
            app,
            tokens,
            approver,
            browser: Box::new(|url| open::that(url).map_err(|e| e.to_string())),
            runtime,
            // few calls far apart: a pooled keep-alive connection the server has since closed would fail a POST
            http: ureq::AgentBuilder::new().timeout(HTTP_TIMEOUT).max_idle_connections(0).build(),
            mcp,
            callbacks: oauth::Callbacks::default(),
            status: watch::channel(status).0,
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

async fn fetch_endpoints(agent: ureq::Agent, url: String) -> Result<Endpoints, String> {
    blocking(move || {
        let discovery = discovery::fetch(&agent, &url)?;
        let metadata = oauth::metadata(&agent, &discovery)?;
        Ok(Endpoints { url, discovery, metadata })
    })
    .await
}
