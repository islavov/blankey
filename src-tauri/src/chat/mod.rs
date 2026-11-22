//! The case chat: each case's thread runs on the relay's agent, one turn per case at a time, several cases at once.
//! The app streams each turn's events to the window, keeps the thread in the vault, and answers the agent's tool
//! calls under the case's `case:<uuid>` grant while the turn runs (`relay/README.md`, "Case chat").

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

use crate::core::Blankey;
use crate::relay::{ChatRefusal, Relay};
use crate::vault::StoredMessage;

const UNAVAILABLE: &str = "The assistant is not available: sign in to Blankey Online with an account that has it.";
const LOST: &str = "The connection to the assistant was lost before it finished.";

/// One event of a running turn, as the window receives it (`chat-event`).
#[derive(Clone, Debug, Serialize)]
pub struct Update {
    pub case_id: i64,
    pub turn_id: String,
    pub event: String,
    pub data: Value,
}

/// A case's stored thread, for the chat tab.
#[derive(Debug, Serialize)]
pub struct Thread {
    pub messages: Vec<StoredMessage>,
    pub summary: Option<String>,
    pub running: bool,
}

struct Running {
    turn_id: String,
}

/// A turn on its way to the relay: what `run` streams and stores.
struct Turn {
    case_id: i64,
    turn_id: String,
    uuid: String,
    body: Value,
    seqs: Vec<i64>,
    user: Value,
}

/// How long a turn and each of its tool calls took, from the app's side: model, relay and network time together.
struct Timing {
    sent_at: String,
    started: Instant,
    calls: HashMap<String, Instant>,
    tools: Map<String, Value>,
}

impl Timing {
    fn start() -> Self {
        let sent_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        Self { sent_at, started: Instant::now(), calls: HashMap::new(), tools: Map::new() }
    }

    fn ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Notes when a `tool_call` arrived, and on its `tool_done` adds how long it ran to the event.
    fn track(&mut self, event: &str, data: &mut Value) {
        let Some(id) = data["id"].as_str().map(str::to_owned) else { return };
        match event {
            "tool_call" => {
                self.calls.insert(id, Instant::now());
            }
            "tool_done" => {
                let Some(called) = self.calls.remove(&id) else { return };
                let ms = called.elapsed().as_millis() as u64;
                self.tools.insert(id, json!(ms));
                if let Some(data) = data.as_object_mut() {
                    data.insert("ms".into(), json!(ms));
                }
            }
            _ => {}
        }
    }

    /// What the turn's user message keeps: when it was sent, the turn's length and each tool call's, in ms.
    fn record(&self) -> Value {
        json!({"sent_at": self.sent_at, "ms": self.ms(), "tools": self.tools})
    }
}

/// A stored message as the relay takes it: without the app's own `timing`, and with the `files` sent with it as a
/// text part that names them, since the model gets what a file is, never the file.
fn for_relay(message: &Value) -> Value {
    let mut message = message.clone();
    let Some(fields) = message.as_object_mut() else { return message };
    fields.remove("timing");
    let files = fields.remove("files");
    let files = files.as_ref().and_then(Value::as_array).filter(|files| !files.is_empty());
    if let (Some(files), Some(content)) = (files, fields.get_mut("content").and_then(Value::as_array_mut)) {
        let mut text = String::from("The user added these files to the case with this message:");
        for file in files {
            let (id, name, kind) =
                (&file["id"], file["name"].as_str().unwrap_or(""), file["type"].as_str().unwrap_or(""));
            text.push_str(&format!("\n- file {id}: {name} ({kind})"));
            match file["description"].as_str() {
                Some(description) if !description.is_empty() && description != name => {
                    text.push_str(&format!(", described as \"{description}\""));
                }
                _ => {}
            }
        }
        content.push(json!({"type": "text", "text": text}));
    }
    message
}

type Emit = Box<dyn Fn(Update) + Send + Sync>;

pub struct Chat {
    app: Arc<Blankey>,
    relay: Arc<Relay>,
    runtime: Handle,
    turns: Mutex<HashMap<i64, Running>>,
    emit: Emit,
}

/// The grant the relay's agent works under for one case.
pub fn grant(case_uuid: &str) -> String {
    format!("case:{case_uuid}")
}

/// The case block sent with every turn: the case as the agent's `get_case` sees it, never a profile value.
pub fn case_block(app: &Blankey, case_id: i64, case_uuid: &str) -> Result<String, String> {
    let overview = app.case_overview(case_id, Some(&grant(case_uuid)))?;
    let text = serde_json::to_string_pretty(&overview).map_err(|e| e.to_string())?;
    Ok(format!(
        "This conversation is about case {case_id}. Pass case_id {case_id} to every tool that takes one.\n{text}"
    ))
}

impl Chat {
    /// `emit` sends each update to the window; `runtime` runs the turns.
    pub fn new(
        app: Arc<Blankey>,
        relay: Arc<Relay>,
        runtime: Handle,
        emit: impl Fn(Update) + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self { app, relay, runtime, turns: Mutex::new(HashMap::new()), emit: Box::new(emit) })
    }

    fn turns(&self) -> MutexGuard<'_, HashMap<i64, Running>> {
        self.turns.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The cases with a turn in progress.
    pub fn running(&self) -> Vec<i64> {
        let mut cases: Vec<i64> = self.turns().keys().copied().collect();
        cases.sort_unstable();
        cases
    }

    /// The case's stored messages, its summary, and whether a turn is running.
    pub fn thread(&self, case_id: i64) -> Result<Thread, String> {
        let vault = self.app.vault();
        vault.get_case(case_id).map_err(|e| e.to_string())?;
        let messages = vault.case_messages(case_id).map_err(|e| e.to_string())?;
        let summary = vault.case_summary(case_id).map_err(|e| e.to_string())?.map(|(text, _)| text);
        Ok(Thread { messages, summary, running: self.turns().contains_key(&case_id) })
    }

    /// Starts a turn with the user's message and returns its id; the turn goes on in the background.
    /// `files`: files of the case sent with the message; the model is told which they are.
    pub fn send(self: &Arc<Self>, case_id: i64, text: &str, files: &[i64]) -> Result<String, String> {
        let text = text.trim();
        if text.is_empty() && files.is_empty() {
            return Err("Write a message first".into());
        }
        if !self.relay.chat_available() {
            return Err(UNAVAILABLE.into());
        }
        let (uuid, history, summary) = {
            let vault = self.app.vault();
            let info = vault.get_case(case_id).map_err(|e| e.to_string())?;
            let summary = vault.case_summary(case_id).map_err(|e| e.to_string())?;
            let upto = summary.as_ref().map_or(0, |(_, upto)| *upto);
            let history: Vec<StoredMessage> =
                vault.case_messages(case_id).map_err(|e| e.to_string())?.into_iter().filter(|m| m.seq > upto).collect();
            (info.uuid, history, summary.map(|(text, _)| text))
        };
        let files = self.sent_files(case_id, files)?;
        let context = case_block(&self.app, case_id, &uuid)?;
        let turn_id = uuid::Uuid::new_v4().to_string();
        {
            let mut turns = self.turns();
            if turns.contains_key(&case_id) {
                return Err("The assistant is still working on this case".into());
            }
            turns.insert(case_id, Running { turn_id: turn_id.clone() });
        }
        self.relay.begin_case_turn(&uuid);
        let content = if text.is_empty() { json!([]) } else { json!([{"type": "text", "text": text}]) };
        let mut user = json!({"role": "user", "content": content});
        if !files.is_empty() {
            user["files"] = json!(files);
        }
        let body = json!({
            "turn_id": turn_id, "case": uuid, "context": context, "summary": summary,
            "messages": history.iter().map(|m| for_relay(&m.message)).collect::<Vec<_>>(),
            "user": for_relay(&user)["content"],
        });
        let seqs: Vec<i64> = history.iter().map(|m| m.seq).collect();
        self.app.audit("user", "chat_turn", &case_id.to_string());
        let timing = Timing::start();
        self.update(case_id, &turn_id, "started", json!({"text": text, "files": files, "sent_at": timing.sent_at}));
        let chat = self.clone();
        let turn = Turn { case_id, turn_id: turn_id.clone(), uuid, body, seqs, user };
        self.runtime.spawn(async move { chat.run(turn, timing).await });
        Ok(turn_id)
    }

    /// The files sent with a message as the thread keeps them: what each is, never its content. Refuses a file
    /// that is not the case's.
    fn sent_files(&self, case_id: i64, ids: &[i64]) -> Result<Vec<Value>, String> {
        let vault = self.app.vault();
        let mut files = Vec::new();
        for id in ids {
            let file = vault.get_collected_file(*id).map_err(|e| e.to_string())?;
            if file.case_id != Some(case_id) {
                return Err(format!("File {id} is not in this case"));
            }
            files.push(json!({
                "id": file.id, "name": file.name, "description": file.description, "type": file.file_type,
                "size": file.size,
            }));
        }
        Ok(files)
    }

    /// Streams the turn to the window and stores what it added once the relay says `done`, with its timing.
    async fn run(self: Arc<Self>, turn: Turn, mut timing: Timing) {
        let Turn { case_id, turn_id, uuid, body, seqs, mut user } = turn;
        let (events, mut incoming) = mpsc::unbounded_channel();
        let relay = self.relay.clone();
        let post = tokio::spawn(async move { relay.chat_turn(body, events).await });
        let mut done = None;
        while let Some((event, mut data)) = incoming.recv().await {
            if event == "done" {
                done = Some(data);
            } else {
                timing.track(&event, &mut data);
                self.update(case_id, &turn_id, &event, data);
            }
        }
        let result = post.await.unwrap_or_else(|e| Err(ChatRefusal::new("lost", e.to_string())));
        let record = timing.record();
        let outcome = match done {
            Some(done) => {
                user["timing"] = record.clone();
                match self.store(case_id, &seqs, user, &done) {
                    Ok(()) => {
                        ("done", json!({"stopped": done["stopped"], "error": done.get("error"), "timing": record}))
                    }
                    Err(e) => (
                        "error",
                        json!({"code": "internal", "message": format!("The reply could not be saved: {e}"),
                               "ms": record["ms"]}),
                    ),
                }
            }
            None => {
                let refusal = result.err().unwrap_or_else(|| ChatRefusal::new("lost", LOST));
                ("error", json!({"code": refusal.code, "message": refusal.message, "ms": record["ms"]}))
            }
        };
        self.turns().remove(&case_id);
        self.relay.end_case_turn(&uuid);
        self.update(case_id, &turn_id, outcome.0, outcome.1);
    }

    /// Appends the user's message and the turn's messages, and the new summary when the relay compacted the thread.
    fn store(&self, case_id: i64, seqs: &[i64], user: Value, done: &Value) -> Result<(), String> {
        let mut messages = vec![user];
        messages.extend(done["messages"].as_array().cloned().unwrap_or_default());
        let mut vault = self.app.vault();
        vault.append_case_messages(case_id, &messages).map_err(|e| e.to_string())?;
        let compaction = &done["compaction"];
        if let (Some(summary), Some(folded)) = (compaction["summary"].as_str(), compaction["folded"].as_u64()) {
            if let Some(upto) = (folded as usize).checked_sub(1).and_then(|last| seqs.get(last)) {
                vault.set_case_summary(case_id, summary, *upto).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Stops the case's turn on the relay; the stream still ends with `done` and keeps what already ran.
    pub async fn stop(&self, case_id: i64) -> Result<(), String> {
        let Some(turn_id) = self.turns().get(&case_id).map(|r| r.turn_id.clone()) else { return Ok(()) };
        self.relay.cancel_chat(&turn_id).await
    }

    /// Locking the vault stops every turn: the agent may not act on a locked vault.
    pub fn stop_all(self: &Arc<Self>) {
        for case_id in self.running() {
            let chat = self.clone();
            self.runtime.spawn(async move {
                if let Err(e) = chat.stop(case_id).await {
                    eprintln!("chat: cannot stop the turn of case {case_id}: {e}");
                }
            });
        }
    }

    fn update(&self, case_id: i64, turn_id: &str, event: &str, data: Value) {
        (self.emit)(Update { case_id, turn_id: turn_id.to_string(), event: event.to_string(), data });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_sent_with_a_message_reach_the_relay_as_text() {
        let stored = json!({
            "role": "user",
            "content": [{"type": "text", "text": "Make a template from this"}],
            "timing": {"ms": 5},
            "files": [
                {"id": 7, "name": "lease.docx", "description": "Lease draft", "type": "docx", "size": 100},
                {"id": 8, "name": "scan.pdf", "description": "scan.pdf", "type": "pdf", "size": 200},
            ],
        });
        let sent = for_relay(&stored);
        assert!(sent.get("timing").is_none() && sent.get("files").is_none());
        assert_eq!(sent["content"][0]["text"], "Make a template from this");
        assert_eq!(
            sent["content"][1]["text"],
            "The user added these files to the case with this message:\n\
             - file 7: lease.docx (docx), described as \"Lease draft\"\n- file 8: scan.pdf (pdf)"
        );
        let plain = json!({"role": "user", "content": [{"type": "text", "text": "Hello"}], "timing": {}});
        assert_eq!(for_relay(&plain), json!({"role": "user", "content": [{"type": "text", "text": "Hello"}]}));
    }
}
