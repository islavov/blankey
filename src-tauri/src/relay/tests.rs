//! A stub relay: the HTTP endpoints of protocol v1 (axum) and the device WebSocket (tokio-tungstenite), driven
//! frame by frame from the tests. Small enough to read as a reference for the app side of the protocol.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State as Shared};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response as Handshake};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use super::discovery::tests::document;
use super::*;
use crate::keychain::MemoryStore;
use crate::vault::now;

const ACCESS_TOKEN: &str = "access-1";
const DEVICE_ID: &str = "device-1";
const WAIT: Duration = Duration::from_secs(20);

struct StubState {
    url: String,
    socket_url: String,
    protocol: u32,
    /// the user signs in on the relay as soon as the browser opens it
    auto_sign_in: AtomicBool,
    /// authorization code -> PKCE challenge
    codes: Mutex<HashMap<String, String>>,
    revoked: Mutex<Vec<String>>,
}

struct Stub {
    state: Arc<StubState>,
    sockets: TcpListener,
}

impl Stub {
    async fn start(protocol: u32) -> Self {
        let http = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let sockets = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let state = Arc::new(StubState {
            url: format!("http://127.0.0.1:{}", http.local_addr().unwrap().port()),
            socket_url: format!("ws://127.0.0.1:{}/device/connect", sockets.local_addr().unwrap().port()),
            protocol,
            auto_sign_in: AtomicBool::new(true),
            codes: Mutex::default(),
            revoked: Mutex::default(),
        });
        let routes = Router::new()
            .route("/.well-known/blankey-relay", get(discovery))
            .route("/.well-known/oauth-authorization-server", get(metadata))
            .route("/register", post(|| async { Json(json!({"client_id": "client-1"})) }))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route(
                "/device/register",
                post(|| async { Json(json!({"device_id": DEVICE_ID, "account": "you@example.com"})) }),
            )
            .route("/device/grants/revoke", post(revoke))
            .with_state(state.clone());
        tokio::spawn(async move { axum::serve(http, routes).await });
        Self { state, sockets }
    }

    fn url(&self) -> String {
        self.state.url.clone()
    }

    /// The next device connection.
    async fn accept(&self) -> WebSocketStream<TcpStream> {
        let (stream, _) =
            tokio::time::timeout(WAIT, self.sockets.accept()).await.expect("no device connection").unwrap();
        tokio_tungstenite::accept_hdr_async(stream, check_device).await.unwrap()
    }
}

/// The handshake check: the device token, and a device id the relay knows.
#[allow(clippy::result_large_err)] // the signature tungstenite's callback requires
fn check_device(request: &Request, response: Handshake) -> Result<Handshake, ErrorResponse> {
    let reject = |status: u16| {
        let mut error = ErrorResponse::new(None);
        *error.status_mut() = StatusCode::from_u16(status).unwrap();
        error
    };
    let bearer = request.headers().get("authorization").and_then(|v| v.to_str().ok());
    if bearer != Some(&format!("Bearer {ACCESS_TOKEN}")) {
        return Err(reject(401));
    }
    if request.uri().query() != Some(&format!("device_id={DEVICE_ID}")) {
        return Err(reject(404));
    }
    Ok(response)
}

async fn discovery(Shared(stub): Shared<Arc<StubState>>) -> Json<Value> {
    let mut doc = serde_json::to_value(document(&stub.url, &stub.socket_url)).unwrap();
    doc["protocol"] = json!(stub.protocol);
    Json(doc)
}

async fn metadata(Shared(stub): Shared<Arc<StubState>>) -> Json<Value> {
    let url = &stub.url;
    Json(json!({
        "issuer": url,
        "authorization_endpoint": format!("{url}/authorize"),
        "token_endpoint": format!("{url}/token"),
        "registration_endpoint": format!("{url}/register"),
        "code_challenge_methods_supported": ["S256"],
    }))
}

/// Signs the user in at once and sends the browser back to the app with a code.
async fn authorize(Shared(stub): Shared<Arc<StubState>>, Query(query): Query<HashMap<String, String>>) -> Response {
    if !stub.auto_sign_in.load(Ordering::SeqCst) {
        return (StatusCode::OK, "Sign in").into_response();
    }
    assert_eq!((query["scope"].as_str(), query["code_challenge_method"].as_str()), ("device", "S256"));
    assert_eq!(query["resource"], format!("{}/device", stub.url));
    let code = format!("code-{}", stub.codes.lock().unwrap().len());
    stub.codes.lock().unwrap().insert(code.clone(), query["code_challenge"].clone());
    Redirect::to(&format!("{}?code={code}&state={}", query["redirect_uri"], query["state"])).into_response()
}

async fn token(Shared(stub): Shared<Arc<StubState>>, Form(form): Form<HashMap<String, String>>) -> Response {
    let valid = match form["grant_type"].as_str() {
        "authorization_code" => {
            let challenge = stub.codes.lock().unwrap().remove(&form["code"]);
            challenge == Some(URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())))
        }
        "refresh_token" => form["refresh_token"] == "refresh-1",
        _ => false,
    };
    if !valid {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
    }
    Json(json!({"access_token": ACCESS_TOKEN, "refresh_token": "refresh-1", "token_type": "Bearer"})).into_response()
}

async fn revoke(Shared(stub): Shared<Arc<StubState>>, Json(body): Json<Value>) -> StatusCode {
    stub.revoked.lock().unwrap().push(body["grant"].as_str().unwrap_or_default().into());
    StatusCode::NO_CONTENT
}

/// Sends a frame and returns the device's answer with the same id, answering pings meanwhile.
async fn ask(socket: &mut WebSocketStream<TcpStream>, frame: Value) -> Value {
    socket.send(Message::text(frame.to_string())).await.unwrap();
    loop {
        let message = tokio::time::timeout(WAIT, socket.next()).await.unwrap().unwrap().unwrap();
        let reply: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        if reply["type"] == "ping" {
            socket.send(Message::text(json!({"v": 1, "type": "pong"}).to_string())).await.unwrap();
        } else if reply["id"] == frame["id"] {
            return reply;
        }
    }
}

/// An app on `stub`, with the local callback route served on the app's port as the MCP server would.
async fn signed_in_app(dir: &Path, stub: &Stub) -> (Arc<Relay>, Arc<Blankey>) {
    let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = local.local_addr().unwrap().port();
    std::fs::create_dir_all(dir.join("data")).unwrap();
    std::fs::write(dir.join("data/config.toml"), format!("port = {port}\nrelay_url = \"{}\"\n", stub.url())).unwrap();
    let app = crate::mcp::tools::tests::app(dir);
    let approver = |_: &Approval| true;
    let browser = |url: &str| {
        let url = url.to_string();
        // the browser follows the relay's redirect back to /relay/callback
        std::thread::spawn(move || ureq::get(&url).call().map(|_| ()).unwrap());
        Ok(())
    };
    let relay = Relay::new(app.clone(), Box::new(MemoryStore::default()), Arc::new(approver), Handle::current())
        .with_browser(browser);
    let relay = Arc::new(relay);
    let callback = relay.callback_router();
    tokio::spawn(async move { axum::serve(local, callback).await });
    relay.login();
    (relay, app)
}

async fn wait_for(relay: &Relay, state: State) {
    let mut status = relay.subscribe();
    let reached = tokio::time::timeout(WAIT, status.wait_for(|s| s.state == state)).await;
    assert!(reached.is_ok(), "expected {state:?}, still {:?}", relay.status());
}

fn grant(id: &str, relay_url: &str) -> RelayGrant {
    RelayGrant {
        grant_id: id.into(),
        relay_url: relay_url.into(),
        client_name: "Claude".into(),
        redirect_host: "claude.ai".into(),
        accepted_at: now(),
    }
}

fn mcp_frame(id: &str, grant: &str, body: Value) -> Value {
    json!({"v": 1, "type": "mcp", "id": id, "grant": grant, "client_name": "Claude", "method": "POST",
           "headers": {"content-type": "application/json", "accept": "application/json, text/event-stream",
                       "x-forwarded-for": "203.0.113.9"},
           "body": body.to_string()})
}

#[tokio::test(flavor = "multi_thread")]
async fn signs_in_and_answers_frames() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, app) = signed_in_app(dir.path(), &stub).await;
    let mut socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;
    assert!(relay.signed_in());
    assert_eq!(app.vault().relay_device_id().unwrap().as_deref(), Some(DEVICE_ID));
    let summary = relay.summary();
    assert_eq!((summary.signed_in, summary.account.as_deref()), (true, Some("you@example.com")));
    let config = Config::load_from(dir.path().join("data")).unwrap();
    assert!(config.relay_enabled);

    app.vault().add_relay_grant(&grant("g-ok", &stub.url())).unwrap();
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}});
    let reply = ask(&mut socket, mcp_frame("r-1", "g-ok", initialize.clone())).await;
    assert_eq!((reply["type"].as_str(), reply["status"].as_u64()), (Some("mcp_result"), Some(200)), "{reply}");
    assert_eq!(reply["headers"]["content-type"], "application/json");
    let body: Value = serde_json::from_str(reply["body"].as_str().unwrap()).unwrap();
    assert!(body["result"]["instructions"].as_str().unwrap().contains("Blankey"));

    let reply = ask(&mut socket, mcp_frame("r-2", "g-unknown", initialize)).await;
    assert_eq!(reply["status"], 403);
    let body: Value = serde_json::from_str(reply["body"].as_str().unwrap()).unwrap();
    assert_eq!(body["id"], 1);

    let approve = json!({"v": 1, "type": "approve", "id": "a-1", "grant": "g-new", "client_name": "Claude",
                         "redirect_host": "claude.ai"});
    assert_eq!(ask(&mut socket, approve).await, json!({"v": 1, "type": "approval", "id": "a-1", "accepted": true}));
    assert!(app.vault().relay_grant_allowed("g-new", &stub.url()).unwrap());

    // the relay's own list, as the Connections page asks for it
    let query = tokio::spawn({
        let relay = relay.clone();
        async move { relay.remote_grants().await }
    });
    let message = tokio::time::timeout(WAIT, socket.next()).await.unwrap().unwrap().unwrap();
    let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
    assert_eq!(frame["type"], "grants");
    let answer = json!({"v": 1, "type": "grants", "id": frame["id"], "grants": [
        {"grant": "g-new", "client_name": "Claude", "created_at": "2026-10-12T10:00:00Z"}]});
    socket.send(Message::text(answer.to_string())).await.unwrap();
    let remote = query.await.unwrap().unwrap();
    assert_eq!(remote.iter().map(|g| g.grant.as_str()).collect::<Vec<_>>(), ["g-new"]);

    relay.revoke("g-new").await.unwrap();
    assert!(!app.vault().relay_grant_allowed("g-new", &stub.url()).unwrap());
    assert_eq!(*stub.state.revoked.lock().unwrap(), ["g-new"]);

    relay.logout();
    assert!(!relay.signed_in());
    assert_eq!(relay.summary().account, None);
    assert_eq!(relay.status().state, State::SignedOut);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_relay_clears_grants() {
    let dir = tempfile::tempdir().unwrap();
    let old = Stub::start(1).await;
    let (relay, app) = signed_in_app(dir.path(), &old).await;
    let _socket = old.accept().await;
    wait_for(&relay, State::Connected).await;
    app.vault().add_relay_grant(&grant("g-1", &old.url())).unwrap();

    let unsupported = Stub::start(2).await;
    assert!(relay.set_url(Some(&unsupported.url())).await.unwrap_err().contains("protocol 2"));
    assert_eq!((relay.url(), relay.grants().len()), (old.url(), 1));

    let new = Stub::start(1).await;
    new.state.auto_sign_in.store(false, Ordering::SeqCst);
    relay.set_url(Some(&new.url())).await.unwrap();
    assert_eq!(*old.state.revoked.lock().unwrap(), ["g-1"]);
    assert!(app.vault().relay_grants(&old.url()).unwrap().is_empty());
    assert_eq!(app.vault().relay_device_id().unwrap(), None);
    assert!(!relay.signed_in());
    assert_eq!(relay.url(), new.url());
    assert_eq!(Config::load_from(dir.path().join("data")).unwrap().relay_url, new.url());
    wait_for(&relay, State::SigningIn).await;
}

/// The next frame from the device that is not a ping (pings are answered).
async fn next_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        let message = tokio::time::timeout(WAIT, socket.next()).await.unwrap().unwrap().unwrap();
        let frame: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        if frame["type"] != "ping" {
            return frame;
        }
        socket.send(Message::text(json!({"v": 1, "type": "pong"}).to_string())).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn claims_primary_on_unlock_until_the_relay_confirms() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, _app) = signed_in_app(dir.path(), &stub).await;
    let mut socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;

    relay.claim_primary();
    let claim = next_frame(&mut socket).await;
    assert_eq!((claim["v"].as_u64(), claim["type"].as_str()), (Some(1), Some("primary")), "{claim}");

    // lost before the relay answered: claimed again on the next connection
    drop(socket);
    let mut socket = stub.accept().await;
    let claim = next_frame(&mut socket).await;
    assert_eq!(claim["type"], "primary");
    socket.send(Message::text(json!({"v": 1, "type": "primary", "id": claim["id"]}).to_string())).await.unwrap();

    // a plain reconnect after the relay confirmed does not move agents to this computer
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
    let reply = ask(&mut socket, mcp_frame("r-1", "g-unknown", initialize.clone())).await;
    assert_eq!(reply["type"], "mcp_result");
    drop(socket);
    let mut socket = stub.accept().await;
    socket.send(Message::text(mcp_frame("r-2", "g-unknown", initialize).to_string())).await.unwrap();
    let frame = next_frame(&mut socket).await;
    assert_eq!((frame["type"].as_str(), frame["id"].as_str()), (Some("mcp_result"), Some("r-2")), "{frame}");
}
