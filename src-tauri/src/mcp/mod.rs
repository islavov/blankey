//! MCP over streamable HTTP in JSON-response mode on 127.0.0.1, plus `/health`.
//!
//! Only local connections: the listener binds to loopback and foreign Host/Origin headers are refused (DNS
//! rebinding).

pub mod bridge;
pub mod tools;

use std::future::Future;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::config::{HEALTH_PATH, HEALTH_TEXT};
use crate::core::Blankey;

/// Request extension on MCP requests forwarded by the relay (`relay::client`): the agent's grant and name.
#[derive(Clone, Debug)]
pub struct Remote {
    pub grant: String,
    pub client_name: String,
}

pub fn router(app: Arc<Blankey>, port: u16) -> Router {
    let origins = ["http", "https"]
        .iter()
        .flat_map(|scheme| ["127.0.0.1", "localhost", "[::1]"].map(|host| format!("{scheme}://{host}:{port}")));
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        .with_allowed_origins(origins)
        .enforce_origin_validation();
    let service: StreamableHttpService<tools::Tools, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(tools::Tools::new(app.clone())), Default::default(), config);
    Router::new().nest_service("/mcp", service).route(HEALTH_PATH, get(|| async { HEALTH_TEXT }))
}

pub async fn serve(
    app: Arc<Blankey>,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    serve_with(app, listener, Router::new(), shutdown).await
}

/// `serve` plus extra routes (the relay sign-in callback).
pub async fn serve_with(
    app: Arc<Blankey>,
    listener: tokio::net::TcpListener,
    extra: Router,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let port = listener.local_addr()?.port();
    axum::serve(listener, router(app, port).merge(extra)).with_graceful_shutdown(shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn post(port: u16, body: Value, origin: Option<&str>) -> (u16, Value) {
        let mut request = ureq::post(&format!("http://127.0.0.1:{port}/mcp"))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        if let Some(origin) = origin {
            request = request.set("Origin", origin);
        }
        let response = match request.send_string(&body.to_string()) {
            Ok(r) | Err(ureq::Error::Status(_, r)) => r,
            Err(e) => panic!("{e}"),
        };
        (response.status(), serde_json::from_str(&response.into_string().unwrap()).unwrap_or(Value::Null))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_tools_over_http() {
        let dir = tempfile::tempdir().unwrap();
        let app = tools::tests::app(dir.path());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(serve(app, listener, std::future::pending()));
        tokio::task::spawn_blocking(move || {
            let health = ureq::get(&format!("http://127.0.0.1:{port}/health")).call().unwrap().into_string().unwrap();
            assert_eq!(health, "blankey");
            let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}});
            let (status, reply) = post(port, init, None);
            assert_eq!(status, 200, "{reply}");
            assert!(reply["result"]["instructions"].as_str().unwrap().contains("Blankey"));
            let (_, tools) = post(port, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}), None);
            assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 21);
            let call = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                              "params": {"name": "get_template", "arguments": {"template_id": "nope"}}});
            let (_, reply) = post(port, call.clone(), None);
            assert_eq!(reply["result"]["isError"], true, "{reply}");
            let local = format!("http://localhost:{port}");
            assert_eq!(post(port, call.clone(), Some(&local)).0, 200);
            assert_eq!(post(port, call, Some("http://evil.example")).0, 403);
        })
        .await
        .unwrap();
    }
}
