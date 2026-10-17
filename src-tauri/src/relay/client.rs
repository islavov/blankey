//! The device WebSocket: connects, keeps it alive, reconnects with backoff and answers the relay's frames.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Method};
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tower::ServiceExt;
use url::Url;

use super::{blocking, oauth, Approval, Failure, Relay, RemoteGrant, State};
use crate::mcp::Remote;
use crate::vault::{now, RelayGrant};

const PING_INTERVAL: Duration = Duration::from_secs(30);
const MIN_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// the relay waits at least 60 s for an mcp_result
const MCP_TIMEOUT: Duration = Duration::from_secs(58);
/// the relay gives up on an approval after 120 s
const APPROVAL_WINDOW: Duration = Duration::from_secs(120);
const MAX_BODY: usize = 64 << 20;
/// Plus every `mcp-*` header: session and version, and the Mcp-Method/Mcp-Name/Mcp-Param-* that MCP 2026-07-28
/// requires.
const FORWARDED_HEADERS: [&str; 3] = ["content-type", "accept", "last-event-id"];
const RETURNED_HEADERS: [&str; 2] = ["content-type", "mcp-session-id"];

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Runs until the relay signs the app out or the task is aborted.
pub(super) async fn run(relay: Arc<Relay>) {
    let mut backoff = MIN_BACKOFF;
    let mut registered = false;
    relay.set_status(State::Connecting, "");
    loop {
        match connect(&relay, &mut registered).await {
            Ok(socket) => {
                relay.set_status(State::Connected, "");
                serve(&relay, socket).await;
                relay.set_status(State::Connecting, "The connection to the relay was lost.");
                backoff = MIN_BACKOFF;
            }
            Err((State::SignedOut, message)) => {
                relay.set_status(State::SignedOut, message);
                return;
            }
            Err((state, message)) => relay.set_status(state, message),
        }
        tokio::time::sleep(jitter(backoff)).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

fn jitter(backoff: Duration) -> Duration {
    let ms = backoff.as_millis() as u64;
    Duration::from_millis(ms + rand::thread_rng().gen_range(0..=ms / 2))
}

async fn connect(relay: &Relay, registered: &mut bool) -> Result<Socket, (State, String)> {
    let failed = |failure: Failure| match failure {
        Failure::SignedOut(message) => (State::SignedOut, message),
        Failure::Offline(message) => (State::Offline, message),
    };
    let endpoints = relay.endpoints().await.map_err(failed)?;
    if !*registered {
        let known = relay.app.vault().relay_device_id().ok().flatten();
        let registration = relay
            .authorized(move |agent, discovery, token| {
                oauth::register_device(agent, discovery, token, known.as_deref())
            })
            .await
            .map_err(failed)?;
        {
            let vault = relay.app.vault();
            let stored = |e: crate::vault::VaultError| (State::Offline, e.to_string());
            vault.set_relay_device_id(Some(&registration.device_id)).map_err(stored)?;
            vault.set_relay_account(registration.account.as_deref()).map_err(stored)?;
        }
        *registered = true;
    }
    let device_id = relay.app.vault().relay_device_id().ok().flatten().unwrap_or_default();
    let token = relay.access_token().await.map_err(failed)?;

    let offline = |e: &dyn std::fmt::Display| (State::Offline, e.to_string());
    let mut url = Url::parse(&endpoints.discovery.device_connect).map_err(|e| offline(&e))?;
    url.query_pairs_mut().append_pair("device_id", &device_id);
    let mut request = url.as_str().into_client_request().map_err(|e| offline(&e))?;
    let bearer = format!("Bearer {token}").parse().map_err(|e| offline(&e))?;
    request.headers_mut().insert(header::AUTHORIZATION, bearer);
    match tokio_tungstenite::connect_async(request).await {
        Ok((socket, _)) => Ok(socket),
        Err(tungstenite::Error::Http(response)) => Err(match response.status().as_u16() {
            401 => {
                relay.drop_access_token();
                (State::Connecting, String::new())
            }
            402 => (State::AccountNeedsAttention, endpoints.discovery.account_url),
            404 => {
                *registered = false;
                (State::Connecting, String::new())
            }
            code => (State::Offline, format!("The relay answered {code}.")),
        }),
        Err(e) => Err(offline(&e)),
    }
}

async fn serve(relay: &Arc<Relay>, socket: Socket) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    {
        let mut inner = relay.inner();
        inner.outgoing = Some(tx.clone());
        inner.send_primary_claim();
    }
    // submissions that arrived while the app was offline
    drain(relay);
    let start = tokio::time::Instant::now() + PING_INTERVAL;
    let mut ping = tokio::time::interval_at(start, PING_INTERVAL);
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            message = stream.next() => {
                let text = match message {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                let Ok(frame) = serde_json::from_str::<Value>(&text) else { continue };
                if frame["v"] != 1 {
                    continue;
                }
                match frame["type"].as_str() {
                    Some("pong") => awaiting_pong = false,
                    Some("primary") => relay.inner().primary_pending = false,
                    Some("mcp") => reply(&tx, mcp(relay.clone(), frame)),
                    Some("approve") => reply(&tx, approve(relay.clone(), frame)),
                    Some("grants") => grants(relay, &frame),
                    Some("inbox") => drain(relay),
                    _ => {}
                }
            }
            Some(text) = rx.recv() => {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
            _ = ping.tick() => {
                if awaiting_pong {
                    break;
                }
                awaiting_pong = true;
                if sink.send(Message::text(json!({"v": 1, "type": "ping"}).to_string())).await.is_err() {
                    break;
                }
            }
        }
    }
    relay.inner().outgoing = None;
}

fn drain(relay: &Arc<Relay>) {
    let relay = relay.clone();
    tokio::spawn(async move { relay.drain_inbox().await });
}

/// Answers on its own task: an MCP call or an approval dialog must not hold up other frames.
fn reply(
    tx: &mpsc::UnboundedSender<String>,
    answer: impl std::future::Future<Output = Option<Value>> + Send + 'static,
) {
    let tx = tx.clone();
    tokio::spawn(async move {
        if let Some(answer) = answer.await {
            let _ = tx.send(answer.to_string());
        }
    });
}

#[derive(Deserialize)]
struct McpFrame {
    grant: String,
    #[serde(default)]
    client_name: String,
    method: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: String,
}

struct HttpReply {
    status: u16,
    headers: Map<String, Value>,
    body: String,
}

/// A JSON-RPC error answering the request in `body`.
fn rpc_error(status: u16, body: &str, message: &str) -> HttpReply {
    let id = serde_json::from_str::<Value>(body).ok().and_then(|v| v.get("id").cloned()).unwrap_or(Value::Null);
    let error = json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}});
    let headers = Map::from_iter([("content-type".to_string(), json!("application/json"))]);
    HttpReply { status, headers, body: error.to_string() }
}

async fn mcp(relay: Arc<Relay>, frame: Value) -> Option<Value> {
    let id = frame["id"].as_str()?.to_string();
    let reply = match serde_json::from_value::<McpFrame>(frame) {
        Ok(frame) => forward(&relay, frame).await,
        Err(e) => rpc_error(400, "", &format!("Bad mcp frame: {e}")),
    };
    Some(json!({"v": 1, "type": "mcp_result", "id": id, "status": reply.status, "headers": reply.headers,
                "body": reply.body}))
}

/// Runs the request against the local MCP router in process, as if it came over HTTP without an Origin.
async fn forward(relay: &Relay, frame: McpFrame) -> HttpReply {
    let allowed = relay.app.vault().relay_grant_allowed(&frame.grant, &relay.url()).unwrap_or(false);
    if !allowed {
        let message = "This agent is not allowed on this computer. Connect it to Blankey again to ask for access.";
        return rpc_error(403, &frame.body, message);
    }
    let method = match frame.method.as_str() {
        "POST" => Method::POST,
        "GET" => Method::GET,
        "DELETE" => Method::DELETE,
        _ => return rpc_error(405, &frame.body, "Method not allowed"),
    };
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::HOST, format!("127.0.0.1:{}", relay.app.config.port));
    for (name, value) in &frame.headers {
        let name = name.to_ascii_lowercase();
        if FORWARDED_HEADERS.contains(&name.as_str()) || name.starts_with("mcp-") {
            builder = builder.header(name, value);
        }
    }
    let mut request = match builder.body(Body::from(frame.body.clone())) {
        Ok(request) => request,
        Err(e) => return rpc_error(400, &frame.body, &e.to_string()),
    };
    request.extensions_mut().insert(Remote { grant: frame.grant, client_name: frame.client_name });
    let call = async {
        let response = match relay.mcp.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
        let status = response.status().as_u16();
        let headers = RETURNED_HEADERS
            .iter()
            .filter_map(|name| Some((name.to_string(), json!(response.headers().get(*name)?.to_str().ok()?))))
            .collect();
        let body = axum::body::to_bytes(response.into_body(), MAX_BODY).await;
        match body {
            Ok(bytes) => HttpReply { status, headers, body: String::from_utf8_lossy(&bytes).into_owned() },
            Err(e) => rpc_error(502, &frame.body, &e.to_string()),
        }
    };
    tokio::time::timeout(MCP_TIMEOUT, call)
        .await
        .unwrap_or_else(|_| rpc_error(504, &frame.body, "Blankey took too long to answer"))
}

#[derive(Deserialize)]
struct ApproveFrame {
    id: String,
    grant: String,
    client_name: String,
    #[serde(default)]
    redirect_host: String,
}

async fn approve(relay: Arc<Relay>, frame: Value) -> Option<Value> {
    let frame: ApproveFrame = serde_json::from_value(frame).ok()?;
    let relay_url = relay.url();
    let approval = Approval {
        grant: frame.grant.clone(),
        client_name: frame.client_name.clone(),
        redirect_host: frame.redirect_host.clone(),
        relay_host: Url::parse(&relay_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default(),
    };
    let started = Instant::now();
    let approver = relay.approver.clone();
    let mut accepted = blocking(move || approver.approve(&approval)).await && started.elapsed() < APPROVAL_WINDOW;
    if accepted {
        let grant = RelayGrant {
            grant_id: frame.grant.clone(),
            relay_url,
            client_name: frame.client_name.clone(),
            redirect_host: frame.redirect_host.clone(),
            accepted_at: now(),
        };
        accepted = relay.app.vault().add_relay_grant(&grant).is_ok();
    }
    let action = if accepted { "relay_accept" } else { "relay_deny" };
    relay.app.audit("user", action, &format!("{} ({}) grant={}", frame.client_name, frame.redirect_host, frame.grant));
    Some(json!({"v": 1, "type": "approval", "id": frame.id, "accepted": accepted}))
}

fn grants(relay: &Relay, frame: &Value) {
    let Some(id) = frame["id"].as_str() else { return };
    let Some(tx) = relay.inner().grant_queries.remove(id) else { return };
    let grants: Vec<RemoteGrant> = serde_json::from_value(frame["grants"].clone()).unwrap_or_default();
    let _ = tx.send(grants);
}
