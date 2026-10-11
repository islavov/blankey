//! `blankey mcp`: stdio MCP server for Claude Desktop that forwards to the running app.
//!
//! Each JSON-RPC line from stdin is POSTed to the app's local HTTP endpoint and the JSON reply is written back to
//! stdout. Requests run on their own threads so a blocking wait_request does not hold up other calls. If the app
//! is not running it is started.

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::config::{server_running, Config};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(20);
/// longer than wait_request's maximum
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1900);

struct Bridge {
    url: String,
    health: String,
    session: Mutex<Option<String>>,
    protocol: Mutex<Option<String>>,
    stdout: Mutex<std::io::Stdout>,
    start: Mutex<()>,
    agent: ureq::Agent,
}

impl Bridge {
    fn write(&self, message: &Value) {
        let mut out = self.stdout.lock().unwrap();
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    }

    fn post(&self, body: &str) -> Result<(u16, Option<String>, String), String> {
        let mut request = self
            .agent
            .post(&self.url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json, text/event-stream");
        if let Some(session) = self.session.lock().unwrap().clone() {
            request = request.set("mcp-session-id", &session);
        }
        if let Some(protocol) = self.protocol.lock().unwrap().clone() {
            request = request.set("mcp-protocol-version", &protocol);
        }
        let response = match request.send_string(body) {
            Ok(response) => response,
            Err(ureq::Error::Status(_, response)) => response,
            Err(e) => return Err(e.to_string()),
        };
        let status = response.status();
        let session = response.header("mcp-session-id").map(str::to_string);
        Ok((status, session, response.into_string().map_err(|e| e.to_string())?))
    }

    fn ensure_app(&self) -> bool {
        let _guard = self.start.lock().unwrap();
        if server_running(&self.health) {
            return true;
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut command = std::process::Command::new(exe);
            command
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            if command.spawn().is_err() {
                return false;
            }
        }
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            if server_running(&self.health) {
                return true;
            }
        }
        false
    }

    fn handle(&self, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else { return };
        let id = message.get("id").cloned();
        let fail = |text: &str| {
            if let Some(id) = &id {
                self.write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": text}}));
            }
        };
        if !self.ensure_app() {
            return fail("Blankey is not running and could not be started");
        }
        let mut reply = self.post(line);
        if matches!(reply, Ok((404, _, _))) && self.session.lock().unwrap().is_some() {
            // the app restarted: its sessions are gone
            *self.session.lock().unwrap() = None;
            reply = self.post(line);
        }
        let (_, session, body) = match reply {
            Ok(reply) => reply,
            Err(e) => return fail(&e),
        };
        if let Some(session) = session {
            *self.session.lock().unwrap() = Some(session);
        }
        if message.get("method").and_then(Value::as_str) == Some("initialize") {
            *self.protocol.lock().unwrap() =
                message.pointer("/params/protocolVersion").and_then(Value::as_str).map(str::to_string);
        }
        if body.trim().is_empty() {
            return;
        }
        match serde_json::from_str::<Value>(&body) {
            Ok(Value::Array(items)) => items.iter().for_each(|item| self.write(item)),
            Ok(item) => self.write(&item),
            Err(_) => fail(&body.chars().take(500).collect::<String>()),
        }
    }
}

pub fn main() {
    let config = match Config::load() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("blankey mcp: {e}");
            std::process::exit(1);
        }
    };
    let bridge = Arc::new(Bridge {
        url: config.mcp_url(),
        health: config.health_url(),
        session: Mutex::new(None),
        protocol: Mutex::new(None),
        stdout: Mutex::new(std::io::stdout()),
        start: Mutex::new(()),
        agent: ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build(),
    });
    let mut workers = Vec::new();
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let bridge = bridge.clone();
        workers.push(std::thread::spawn(move || bridge.handle(&line)));
        workers.retain(|w| !w.is_finished());
    }
    // stdin closed: let the requests in flight answer
    for worker in workers {
        let _ = worker.join();
    }
}
