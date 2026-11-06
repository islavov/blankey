//! A stub relay: the HTTP endpoints of protocol v1 (axum) and the device WebSocket (tokio-tungstenite), driven
//! frame by frame from the tests. Small enough to read as a reference for the app side of the protocol.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path as UrlPath, Query, State as Shared};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
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
    /// bodies of POST /device/links
    links: Mutex<Vec<Value>>,
    revoked_links: Mutex<Vec<String>>,
    /// (sid, link id, sealed blob)
    inbox: Mutex<Vec<(String, String, Vec<u8>)>>,
    /// the next blob download stops halfway
    cut_download: AtomicBool,
    /// the next inbox delete goes through but answers 500, as if the reply was lost
    lose_delete_reply: AtomicBool,
    /// the account's language in the device profile; no `device_profile` in discovery when false
    profile: AtomicBool,
    lang: Mutex<Option<String>>,
    /// the account has the case chat; no `chat_url` in discovery when `chat_offered` is false
    chat: AtomicBool,
    chat_offered: AtomicBool,
    /// bodies of POST /device/chat
    chat_turns: Mutex<Vec<Value>>,
    /// what the next turn streams, chunk by chunk, until the sender is dropped
    chat_stream: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<String>>>,
    /// the next turn is refused with this status and JSON body
    chat_refusal: Mutex<Option<(u16, Value)>>,
    /// turn ids of DELETE /device/chat/{turn}
    chat_cancelled: Mutex<Vec<String>>,
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
            links: Mutex::default(),
            revoked_links: Mutex::default(),
            inbox: Mutex::default(),
            cut_download: AtomicBool::new(false),
            lose_delete_reply: AtomicBool::new(false),
            profile: AtomicBool::new(true),
            lang: Mutex::default(),
            chat: AtomicBool::new(false),
            chat_offered: AtomicBool::new(true),
            chat_turns: Mutex::default(),
            chat_stream: Mutex::default(),
            chat_refusal: Mutex::default(),
            chat_cancelled: Mutex::default(),
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
            .route("/device/links", post(create_link).get(list_links))
            .route("/device/links/{id}", delete(revoke_link))
            .route("/device/inbox", get(list_inbox))
            .route("/device/inbox/{sid}", delete(delete_submission))
            .route("/blob/{sid}", get(blob))
            .route("/device/profile", get(profile))
            .route("/device/chat", post(chat_turn))
            .route("/device/chat/{turn}", delete(cancel_chat_turn))
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
    if !stub.profile.load(Ordering::SeqCst) {
        doc.as_object_mut().unwrap().remove("device_profile");
    }
    if !stub.chat_offered.load(Ordering::SeqCst) {
        doc.as_object_mut().unwrap().remove("chat_url");
    }
    Json(doc)
}

async fn profile(
    Shared(stub): Shared<Arc<StubState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    own_device(&query)?;
    Ok(Json(json!({"account": "you@example.com", "lang": *stub.lang.lock().unwrap(),
                   "chat": stub.chat.load(Ordering::SeqCst)})))
}

/// POST /device/chat: records the turn, then refuses it or streams what the test sends.
async fn chat_turn(
    Shared(stub): Shared<Arc<StubState>>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Response {
    if let Err(status) = own_device(&query) {
        return status.into_response();
    }
    stub.chat_turns.lock().unwrap().push(body);
    if let Some((status, error)) = stub.chat_refusal.lock().unwrap().take() {
        return (StatusCode::from_u16(status).unwrap(), Json(error)).into_response();
    }
    let Some(chunks) = stub.chat_stream.lock().unwrap().take() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let stream = futures_util::stream::unfold(chunks, |mut chunks| async move {
        chunks.recv().await.map(|chunk| (Ok::<_, std::io::Error>(chunk), chunks))
    });
    ([("content-type", "text/event-stream")], axum::body::Body::from_stream(stream)).into_response()
}

async fn cancel_chat_turn(
    Shared(stub): Shared<Arc<StubState>>,
    UrlPath(turn): UrlPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> StatusCode {
    if let Err(status) = own_device(&query) {
        return status;
    }
    stub.chat_cancelled.lock().unwrap().push(turn);
    StatusCode::NO_CONTENT
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

const LINK_ID: &str = "Ab3dE9z";

fn own_device(query: &HashMap<String, String>) -> Result<(), StatusCode> {
    (query.get("device_id").map(String::as_str) == Some(DEVICE_ID)).then_some(()).ok_or(StatusCode::NOT_FOUND)
}

async fn create_link(
    Shared(stub): Shared<Arc<StubState>>,
    Query(query): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, StatusCode> {
    own_device(&query)?;
    stub.links.lock().unwrap().push(body);
    Ok(Json(json!({"id": LINK_ID, "url": format!("{}/s/{LINK_ID}", stub.url)})))
}

async fn list_links(
    Shared(stub): Shared<Arc<StubState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    own_device(&query)?;
    let revoked = stub.revoked_links.lock().unwrap().contains(&LINK_ID.to_string());
    let links: Vec<Value> = stub
        .links
        .lock()
        .unwrap()
        .iter()
        .map(|body| {
            json!({"id": LINK_ID, "state": if revoked { "revoked" } else { "open" }, "submissions": 2,
                   "max_submissions": body["max_submissions"]})
        })
        .collect();
    Ok(Json(json!({"links": links})))
}

async fn revoke_link(
    Shared(stub): Shared<Arc<StubState>>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<StatusCode, StatusCode> {
    own_device(&query)?;
    stub.revoked_links.lock().unwrap().push(id);
    Ok(StatusCode::NO_CONTENT)
}

async fn list_inbox(
    Shared(stub): Shared<Arc<StubState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    own_device(&query)?;
    let inbox: Vec<Value> = stub
        .inbox
        .lock()
        .unwrap()
        .iter()
        .map(|(sid, link, blob)| {
            json!({"sid": sid, "link_id": link, "size": blob.len(), "url": format!("{}/blob/{sid}", stub.url)})
        })
        .collect();
    Ok(Json(json!({"inbox": inbox})))
}

async fn delete_submission(
    Shared(stub): Shared<Arc<StubState>>,
    UrlPath(sid): UrlPath<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<StatusCode, StatusCode> {
    own_device(&query)?;
    let mut inbox = stub.inbox.lock().unwrap();
    let before = inbox.len();
    inbox.retain(|(id, _, _)| *id != sid);
    if stub.lose_delete_reply.swap(false, Ordering::SeqCst) {
        return Ok(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Ok(if inbox.len() < before { StatusCode::NO_CONTENT } else { StatusCode::NOT_FOUND })
}

/// A presigned GET: no bearer token.
async fn blob(Shared(stub): Shared<Arc<StubState>>, UrlPath(sid): UrlPath<String>) -> Response {
    let found = stub.inbox.lock().unwrap().iter().find(|(id, _, _)| *id == sid).map(|(_, _, blob)| blob.clone());
    match found {
        Some(blob) if stub.cut_download.swap(false, Ordering::SeqCst) => {
            blob[..blob.len() / 2].to_vec().into_response()
        }
        Some(blob) => blob.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
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

    // MCP 2026-07-28 (ChatGPT): no initialize; server/discover needs the Mcp-Method header.
    let discover = json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": {"_meta": {
        "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}}}});
    let mut frame = mcp_frame("r-d", "g-ok", discover);
    frame["headers"]["mcp-protocol-version"] = json!("2026-07-28");
    frame["headers"]["mcp-method"] = json!("server/discover");
    let reply = ask(&mut socket, frame).await;
    assert_eq!(reply["status"], 200, "{reply}");
    assert!(reply["body"].as_str().unwrap().contains("supportedVersions"));

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

/// The next frame from the device that is not a ping or a usage report (both are answered).
async fn next_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        let frame = next_raw_frame(socket).await;
        match frame["type"].as_str() {
            Some("ping") => socket.send(Message::text(json!({"v": 1, "type": "pong"}).to_string())).await.unwrap(),
            Some("report") => {
                let ok = json!({"v": 1, "type": "report", "id": frame["id"], "ok": true});
                socket.send(Message::text(ok.to_string())).await.unwrap();
            }
            _ => return frame,
        }
    }
}

async fn next_raw_frame(socket: &mut WebSocketStream<TcpStream>) -> Value {
    let message = tokio::time::timeout(WAIT, socket.next()).await.unwrap().unwrap().unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

/// The next usage report from the device, pings answered on the way.
async fn next_report(socket: &mut WebSocketStream<TcpStream>) -> Value {
    loop {
        let frame = next_raw_frame(socket).await;
        match frame["type"].as_str() {
            Some("ping") => socket.send(Message::text(json!({"v": 1, "type": "pong"}).to_string())).await.unwrap(),
            Some("report") => return frame,
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reports_local_usage_by_month() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, app) = signed_in_app(dir.path(), &stub).await;
    let mut socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;

    // right after connecting: every kept month, every kind, nothing done yet
    let first = next_report(&mut socket).await;
    assert_eq!(first["v"], 1);
    assert_eq!(first["app"], env!("CARGO_PKG_VERSION"));
    let months = first["months"].as_object().unwrap();
    assert_eq!(months.len(), 4);
    let this_month = now()[..7].to_string();
    assert_eq!(
        months[&this_month],
        json!({"doc_generated": 0, "template_saved": 0, "fill_set_saved": 0, "profile_created": 0}),
        "{first}"
    );
    let refused = json!({"v": 1, "type": "report", "id": first["id"], "ok": false, "error": "invalid"});
    socket.send(Message::text(refused.to_string())).await.unwrap();

    app.audit("user", "generate", "template=t document=1");
    app.audit("user", "generate", "template=t document=2");
    app.audit("user", "create_profile", "7");
    app.audit("user", "open_document", "1");
    relay.report();
    let second = next_report(&mut socket).await;
    assert_eq!(
        second["months"][&this_month],
        json!({"doc_generated": 2, "template_saved": 0, "fill_set_saved": 0, "profile_created": 1}),
        "{second}"
    );
    assert_ne!(first["id"], second["id"]);

    // a refused report does not disturb the connection
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
    let reply = ask(&mut socket, mcp_frame("r-1", "g-unknown", initialize)).await;
    assert_eq!(reply["type"], "mcp_result");
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

async fn wait_for_submissions(app: &Blankey, count: usize) -> Vec<crate::vault::Request> {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let requests: Vec<_> = app
            .vault()
            .list_requests(Some("pending"))
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == crate::links::LINK_SUBMISSION)
            .collect();
        if requests.len() >= count || tokio::time::Instant::now() > deadline {
            return requests;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn collection_links_reach_the_device() {
    use crate::links::tests::{draft, plaintext};

    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, app) = signed_in_app(dir.path(), &stub).await;
    let mut socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;

    let link = relay.create_link(&draft()).await.unwrap();
    let body = stub.state.links.lock().unwrap()[0].clone();
    let public = URL_SAFE_NO_PAD.decode(body["public_key"].as_str().unwrap()).unwrap();
    assert_eq!(link.url, format!("{}/s/{LINK_ID}#{}", stub.url(), crate::crypto::fingerprint(&public)));
    assert_eq!(app.vault().get_link(LINK_ID).unwrap(), link);
    assert!(!body.to_string().contains("New tenant"), "the target profile stays on this computer: {body}");
    assert_eq!(body["spec"]["fields"].as_array().unwrap().len(), 2);
    let remote = relay.remote_links().await.unwrap();
    assert_eq!((remote[0].id.as_str(), remote[0].submissions), (LINK_ID, 2));

    let sealed = |values: Value| {
        let plain = plaintext(values, &[("id.pdf", "application/pdf", b"%PDF-1.7 scan")]);
        crate::crypto::seal(&public, &plain, format!("link:{LINK_ID}").as_bytes()).unwrap()
    };
    let inbox_frame = |sid: &str| Message::text(json!({"v": 1, "type": "inbox", "id": sid}).to_string());

    // the download is cut short: nothing is deleted on the relay and no request is made
    stub.state.cut_download.store(true, Ordering::SeqCst);
    stub.state.inbox.lock().unwrap().push(("sub-1".into(), LINK_ID.into(), sealed(json!({"name": "Jane Roe"}))));
    socket.send(inbox_frame("sub-1")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(stub.state.inbox.lock().unwrap().len(), 1);
    assert!(wait_for_submissions(&app, 0).await.is_empty());
    assert_eq!(std::fs::read_dir(app.inbox_dir()).unwrap().count(), 0, "no partial download is left");

    // the next frame retries it
    socket.send(inbox_frame("sub-1")).await.unwrap();
    let requests = wait_for_submissions(&app, 1).await;
    assert_eq!(requests.len(), 1);
    assert!(stub.state.inbox.lock().unwrap().is_empty(), "deleted on the relay once on disk");
    assert!(app.inbox_dir().join("sub-1.bin").exists());
    let opened = app.open_submission("sub-1").unwrap();
    assert_eq!(opened.values["name"], "Jane Roe");
    assert_eq!(opened.files[0].name, "id.pdf");

    assert!(app.unaccepted_submissions().is_empty());

    // a submission that arrived while the app was offline lands on the next connection
    stub.state.inbox.lock().unwrap().push(("sub-2".into(), LINK_ID.into(), sealed(json!({"name": "John Doe"}))));
    drop(socket);
    let mut socket = stub.accept().await;
    assert_eq!(wait_for_submissions(&app, 2).await.len(), 2);
    assert_eq!(app.vault().get_link(LINK_ID).unwrap().received, 2);

    // the relay deletes it but the answer is lost: the next drain still makes the request
    stub.state.lose_delete_reply.store(true, Ordering::SeqCst);
    stub.state.inbox.lock().unwrap().push(("sub-3".into(), LINK_ID.into(), sealed(json!({"name": "Ann Lee"}))));
    socket.send(inbox_frame("sub-3")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(stub.state.inbox.lock().unwrap().is_empty());
    assert_eq!(wait_for_submissions(&app, 0).await.len(), 2);
    assert_eq!(app.unaccepted_submissions(), [(LINK_ID.to_string(), "sub-3".to_string())]);
    socket.send(inbox_frame("sub-3")).await.unwrap();
    assert_eq!(wait_for_submissions(&app, 3).await.len(), 3);
    assert!(app.unaccepted_submissions().is_empty());

    relay.revoke_link(LINK_ID).await.unwrap();
    assert_eq!(*stub.state.revoked_links.lock().unwrap(), [LINK_ID]);
    assert!(app.vault().get_link(LINK_ID).unwrap().revoked_at.is_some());

    // deleting it revokes it on the relay again and forgets it here
    relay.delete_link(LINK_ID).await.unwrap();
    assert_eq!(*stub.state.revoked_links.lock().unwrap(), [LINK_ID, LINK_ID]);
    assert!(app.vault().get_link(LINK_ID).is_err());
}

async fn wait_for_language(relay: &Relay, wanted: Option<&str>) {
    let mut language = relay.subscribe_language();
    let reached = tokio::time::timeout(WAIT, language.wait_for(|l| l.as_deref() == wanted)).await;
    assert!(reached.is_ok(), "expected {wanted:?}, still {:?}", relay.language());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_account_language_comes_from_the_device_profile() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    *stub.state.lang.lock().unwrap() = Some("bg".into());
    let (relay, app) = signed_in_app(dir.path(), &stub).await;
    let mut socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;
    wait_for_language(&relay, Some("bg")).await;
    assert_eq!(app.vault().relay_language().unwrap().as_deref(), Some("bg"));

    // changed on the relay: a profile frame, or the window regaining focus, reads it again
    *stub.state.lang.lock().unwrap() = Some("en".into());
    socket.send(Message::text(json!({"v": 1, "type": "profile"}).to_string())).await.unwrap();
    wait_for_language(&relay, Some("en")).await;
    *stub.state.lang.lock().unwrap() = None;
    relay.refresh_profile().await;
    assert_eq!(relay.language(), None);

    *stub.state.lang.lock().unwrap() = Some("bg".into());
    relay.refresh_profile().await;
    assert_eq!(relay.language().as_deref(), Some("bg"));
    relay.logout();
    assert_eq!(relay.language(), None);
    assert_eq!(app.vault().relay_language().unwrap(), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_without_profiles_names_no_language() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    stub.state.profile.store(false, Ordering::SeqCst);
    *stub.state.lang.lock().unwrap() = Some("bg".into());
    let (relay, app) = signed_in_app(dir.path(), &stub).await;
    let _socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;
    app.vault().set_relay_language(Some("bg")).unwrap();
    relay.refresh_profile().await;
    assert_eq!(relay.language(), None);
    assert_eq!(app.vault().relay_language().unwrap(), None);
}

fn sse(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

/// Polls until `check` holds, for state that changes on other tasks.
async fn eventually(what: &str, check: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// An app signed in to a relay that offers the chat to the account, and a chat service recording its updates.
async fn chat_app(
    dir: &Path,
    stub: &Stub,
) -> (Arc<Relay>, Arc<Blankey>, Arc<crate::chat::Chat>, Arc<Mutex<Vec<crate::chat::Update>>>, WebSocketStream<TcpStream>)
{
    stub.state.chat.store(true, Ordering::SeqCst);
    let (relay, app) = signed_in_app(dir, stub).await;
    let socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;
    eventually("the chat flag from the profile", || relay.chat_available()).await;
    let updates = Arc::new(Mutex::new(Vec::new()));
    let seen = updates.clone();
    let chat = crate::chat::Chat::new(app.clone(), relay.clone(), Handle::current(), move |update| {
        seen.lock().unwrap().push(update);
    });
    (relay, app, chat, updates, socket)
}

fn events(updates: &Mutex<Vec<crate::chat::Update>>) -> Vec<String> {
    updates.lock().unwrap().iter().map(|u| u.event.clone()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_a_case_chat_turn_and_answers_its_grant_only_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, app, chat, updates, mut socket) = chat_app(dir.path(), &stub).await;
    assert!(relay.summary().chat);
    let case_id = app.create_case("Sale of an apartment").unwrap();
    let other = app.create_case("Lease").unwrap();
    let uuid = app.vault().get_case(case_id).unwrap().uuid;
    let other_uuid = app.vault().get_case(other).unwrap().uuid;
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "agent", "version": "1"}}});
    let case_grant = format!("case:{uuid}");
    assert_eq!(ask(&mut socket, mcp_frame("c-0", &case_grant, initialize.clone())).await["status"], 403);

    let (chunks, stream) = tokio::sync::mpsc::unbounded_channel();
    *stub.state.chat_stream.lock().unwrap() = Some(stream);
    let turn_id = chat.send(case_id, "  Find Ivan  ").unwrap();
    assert_eq!(chat.running(), [case_id]);
    assert!(chat.send(case_id, "Again").is_err(), "one turn per case at a time");
    eventually("the turn on the relay", || stub.state.chat_turns.lock().unwrap().len() == 1).await;
    let posted = stub.state.chat_turns.lock().unwrap()[0].clone();
    assert_eq!((posted["turn_id"].as_str(), posted["case"].as_str()), (Some(turn_id.as_str()), Some(uuid.as_str())));
    assert_eq!(posted["user"], json!([{"type": "text", "text": "Find Ivan"}]));
    assert_eq!((posted["messages"].clone(), posted["summary"].clone()), (json!([]), Value::Null));
    let context = posted["context"].as_str().unwrap();
    assert!(context.contains(&format!("case_id {case_id}")) && context.contains("Sale of an apartment"), "{context}");

    // the relay's agent reaches the app's tools under the case's grant while the turn runs, and only that case's
    let reply = ask(&mut socket, mcp_frame("c-1", &case_grant, initialize.clone())).await;
    assert_eq!(reply["status"], 200, "{reply}");
    let other_grant = format!("case:{other_uuid}");
    assert_eq!(ask(&mut socket, mcp_frame("c-2", &other_grant, initialize.clone())).await["status"], 403);

    let assistant = json!({"role": "assistant", "content": [{"type": "text", "text": "Ivan Petrov is profile 7."}]});
    chunks.send(sse("text_delta", json!({"text": "Ivan Petrov"}))).unwrap();
    chunks.send(": ping\n\n".into()).unwrap();
    chunks.send(sse("done", json!({"messages": [assistant], "stopped": null, "model": "m", "usage": {}}))).unwrap();
    drop(chunks);
    eventually("the end of the turn", || chat.running().is_empty()).await;
    assert_eq!(events(&updates), ["started", "text_delta", "done"]);
    let last = updates.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        (last.case_id, last.turn_id.as_str(), last.data["stopped"].clone()),
        (case_id, turn_id.as_str(), Value::Null)
    );
    let thread = chat.thread(case_id).unwrap();
    assert!(!thread.running);
    let stored: Vec<Value> = thread.messages.iter().map(|m| m.message.clone()).collect();
    assert_eq!(
        stored,
        [json!({"role": "user", "content": [{"type": "text", "text": "Find Ivan"}]}), assistant.clone()]
    );
    assert_eq!(ask(&mut socket, mcp_frame("c-3", &case_grant, initialize)).await["status"], 403);

    // the next turn sends the thread; the relay folds the first two messages into a summary
    let (chunks, stream) = tokio::sync::mpsc::unbounded_channel();
    *stub.state.chat_stream.lock().unwrap() = Some(stream);
    chat.send(case_id, "And the lease?").unwrap();
    eventually("the second turn", || stub.state.chat_turns.lock().unwrap().len() == 2).await;
    assert_eq!(stub.state.chat_turns.lock().unwrap()[1]["messages"].as_array().unwrap().len(), 2);
    let reply = json!({"role": "assistant", "content": [{"type": "text", "text": "Not yet."}]});
    chunks
        .send(sse(
            "done",
            json!({"messages": [reply], "stopped": null, "model": "m", "usage": {},
                                    "compaction": {"summary": "Ivan Petrov is the seller.", "folded": 2}}),
        ))
        .unwrap();
    drop(chunks);
    eventually("the end of the second turn", || chat.running().is_empty()).await;
    let thread = chat.thread(case_id).unwrap();
    assert_eq!((thread.messages.len(), thread.summary.as_deref()), (4, Some("Ivan Petrov is the seller.")));

    let (chunks, stream) = tokio::sync::mpsc::unbounded_channel();
    *stub.state.chat_stream.lock().unwrap() = Some(stream);
    chat.send(case_id, "Thanks").unwrap();
    eventually("the third turn", || stub.state.chat_turns.lock().unwrap().len() == 3).await;
    let third = stub.state.chat_turns.lock().unwrap()[2].clone();
    assert_eq!(third["summary"], "Ivan Petrov is the seller.");
    assert_eq!(third["messages"].as_array().unwrap().len(), 2, "only what the summary does not cover");
    drop(chunks);
    eventually("the lost third turn", || chat.running().is_empty()).await;
    let last = updates.lock().unwrap().last().unwrap().clone();
    assert_eq!((last.event.as_str(), last.data["code"].as_str()), ("error", Some("lost")));
    assert_eq!(chat.thread(case_id).unwrap().messages.len(), 4, "a turn without done stores nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_turns_are_refused_without_the_chat_and_can_be_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    let (relay, app, chat, updates, _socket) = chat_app(dir.path(), &stub).await;
    let case_id = app.create_case("Power of attorney").unwrap();

    stub.state.chat.store(false, Ordering::SeqCst);
    relay.refresh_profile().await;
    assert!(!relay.chat_available() && !relay.summary().chat);
    assert!(chat.send(case_id, "Hello").is_err());
    assert!(stub.state.chat_turns.lock().unwrap().is_empty());
    stub.state.chat.store(true, Ordering::SeqCst);
    relay.refresh_profile().await;
    assert!(relay.chat_available());

    *stub.state.chat_refusal.lock().unwrap() = Some((
        429,
        json!({"error": "chat_budget", "error_description": "Used up", "resets_at": "2026-12-01T00:00:00Z"}),
    ));
    chat.send(case_id, "Hello").unwrap();
    eventually("the refusal", || chat.running().is_empty()).await;
    let last = updates.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        (last.event.as_str(), last.data["code"].as_str(), last.data["message"].as_str()),
        ("error", Some("chat_budget"), Some("Used up"))
    );
    assert!(chat.thread(case_id).unwrap().messages.is_empty());

    let (chunks, stream) = tokio::sync::mpsc::unbounded_channel();
    *stub.state.chat_stream.lock().unwrap() = Some(stream);
    let turn_id = chat.send(case_id, "Draft the power of attorney").unwrap();
    eventually("the turn on the relay", || stub.state.chat_turns.lock().unwrap().len() == 2).await;
    chat.stop(case_id).await.unwrap();
    assert_eq!(*stub.state.chat_cancelled.lock().unwrap(), [turn_id]);
    chunks.send(sse("done", json!({"messages": [], "stopped": "cancelled", "model": null, "usage": {}}))).unwrap();
    drop(chunks);
    eventually("the stopped turn", || chat.running().is_empty()).await;
    let last = updates.lock().unwrap().last().unwrap().clone();
    assert_eq!((last.event.as_str(), last.data["stopped"].as_str()), ("done", Some("cancelled")));
    assert_eq!(chat.thread(case_id).unwrap().messages.len(), 1, "the user's message stays with a stopped turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_without_chat_url_offers_no_chat() {
    let dir = tempfile::tempdir().unwrap();
    let stub = Stub::start(1).await;
    stub.state.chat.store(true, Ordering::SeqCst);
    stub.state.chat_offered.store(false, Ordering::SeqCst);
    let (relay, _app) = signed_in_app(dir.path(), &stub).await;
    let _socket = stub.accept().await;
    wait_for(&relay, State::Connected).await;
    relay.refresh_profile().await;
    assert!(!relay.chat_available());
}
