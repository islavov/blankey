//! The case chat on the relay: whether the account has it, the turns this app is running, and one turn as a stream
//! of server-sent events (`relay/README.md`, "Case chat").

use std::io::{BufRead, BufReader};

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, watch};

use super::oauth::{self, HttpError};
use super::{blocking, Relay};

/// One server-sent event of a turn: its name and its JSON data.
pub type ChatEvent = (String, Value);

/// Why a turn did not run or did not finish, as the UI shows it: the relay's `error` code when it refused the turn,
/// `unavailable` when this relay or account has no chat, `lost` when the stream broke.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatRefusal {
    pub code: String,
    pub message: String,
}

impl ChatRefusal {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self { code: code.into(), message: message.into() }
    }

    /// The relay's JSON error body (`{error, error_description}`), or the status when it has none.
    fn from_status(status: u16, body: &str) -> Self {
        #[derive(Deserialize)]
        struct Body {
            error: String,
            #[serde(default)]
            error_description: Option<String>,
        }
        match serde_json::from_str::<Body>(body) {
            Ok(b) => Self::new(
                &b.error,
                b.error_description.unwrap_or_else(|| format!("The online server answered {status}")),
            ),
            Err(_) => Self::new("unavailable", format!("The online server answered {status}")),
        }
    }
}

const UNAVAILABLE: &str = "This online server or account does not offer the assistant.";

impl Relay {
    /// Signed in, the relay offers a chat and the account has it turned on.
    pub fn chat_available(&self) -> bool {
        *self.chat.borrow()
            && self.signed_in()
            && self.inner().endpoints.as_ref().is_some_and(|e| e.discovery.chat_url.is_some())
    }

    /// Changes of the account's chat flag from its device profile.
    pub fn subscribe_chat(&self) -> watch::Receiver<bool> {
        self.chat.subscribe()
    }

    pub(super) fn set_chat(&self, on: bool) {
        self.chat.send_if_modified(|current| std::mem::replace(current, on) != on);
    }

    /// Marks a case's turn as running: its `case:<uuid>` grant is answered until `end_case_turn`.
    pub fn begin_case_turn(&self, case_uuid: &str) {
        self.case_turns.lock().unwrap_or_else(|p| p.into_inner()).insert(case_uuid.to_string());
    }

    pub fn end_case_turn(&self, case_uuid: &str) {
        self.case_turns.lock().unwrap_or_else(|p| p.into_inner()).remove(case_uuid);
    }

    /// A `case:<uuid>` grant the relay's agent uses while a turn this app started for that case runs.
    pub(super) fn case_turn_running(&self, grant: &str) -> bool {
        grant
            .strip_prefix("case:")
            .is_some_and(|uuid| self.case_turns.lock().unwrap_or_else(|p| p.into_inner()).contains(uuid))
    }

    /// Posts one turn and sends each server-sent event to `events` as it arrives; returns when the stream ends.
    pub async fn chat_turn(&self, body: Value, events: mpsc::UnboundedSender<ChatEvent>) -> Result<(), ChatRefusal> {
        let device = self.device_id().map_err(|e| ChatRefusal::new("unavailable", e))?;
        let mut retried = false;
        loop {
            let token = self.access_token().await.map_err(|f| ChatRefusal::new("unavailable", f.message()))?;
            let endpoints = self.endpoints().await.map_err(|f| ChatRefusal::new("unavailable", f.message()))?;
            let Some(base) = endpoints.discovery.chat_url.clone() else {
                return Err(ChatRefusal::new("unavailable", UNAVAILABLE));
            };
            let url = super::device_url(Some(&base), &[], &device)
                .map_err(|e| ChatRefusal::new("unavailable", e.to_string()))?;
            // pings every 20 s keep the read under the download agent's 60 s timeout
            let (agent, body, events) = (self.download.clone(), body.clone(), events.clone());
            match blocking(move || stream(&agent, &url, &token, &body, &events)).await {
                Err(HttpError::Status(401, _)) if !retried => {
                    retried = true;
                    self.drop_access_token();
                }
                Ok(()) => return Ok(()),
                Err(HttpError::Status(status, body)) => return Err(ChatRefusal::from_status(status, &body)),
                Err(HttpError::Transport(e)) => return Err(ChatRefusal::new("lost", e)),
            }
        }
    }

    /// Stops a turn on the relay; its stream still ends with `done`.
    pub async fn cancel_chat(&self, turn_id: &str) -> Result<(), String> {
        let device = self.device_id()?;
        let turn_id = turn_id.to_string();
        self.authorized(move |agent, discovery, token| {
            let url = super::device_url(discovery.chat_url.as_deref(), &[&turn_id], &device)?;
            oauth::delete(agent, &url, token)
        })
        .await
        .map_err(|f| f.message())
    }
}

/// The blocking half of a turn: the POST, then the event stream line by line.
fn stream(
    agent: &ureq::Agent,
    url: &str,
    token: &str,
    body: &Value,
    events: &mpsc::UnboundedSender<ChatEvent>,
) -> Result<(), HttpError> {
    let response = agent
        .post(url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("Accept", "text/event-stream")
        .send_string(&body.to_string());
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(status, response)) => {
            return Err(HttpError::Status(status, response.into_string().unwrap_or_default()))
        }
        Err(e) => return Err(HttpError::Transport(e.to_string())),
    };
    read_events(BufReader::new(response.into_reader()), |event| events.send(event).is_ok())
}

/// Server-sent events: `event:` and `data:` lines up to a blank line; comments (`: ping`) are skipped. Stops early when
/// `deliver` returns false.
fn read_events(reader: impl BufRead, mut deliver: impl FnMut(ChatEvent) -> bool) -> Result<(), HttpError> {
    let (mut name, mut data) = (String::new(), String::new());
    for line in reader.lines() {
        let line = line.map_err(|e| HttpError::Transport(e.to_string()))?;
        if line.is_empty() {
            if !data.is_empty() {
                let value = serde_json::from_str(&data).unwrap_or(Value::Null);
                let event = if name.is_empty() { "message".to_string() } else { std::mem::take(&mut name) };
                if !deliver((event, value)) {
                    return Ok(());
                }
            }
            name.clear();
            data.clear();
        } else if let Some(value) = line.strip_prefix("event:") {
            name = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.strip_prefix(' ').unwrap_or(value));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_events_and_skips_comments() {
        let text = ": ping\n\nevent: text_delta\ndata: {\"text\":\"Hi\"}\n\n: ping\n\nevent: done\ndata: {\"stopped\":null}\n\n";
        let mut seen = Vec::new();
        read_events(text.as_bytes(), |event| {
            seen.push(event);
            true
        })
        .unwrap();
        assert_eq!(
            seen,
            vec![
                ("text_delta".to_string(), serde_json::json!({"text": "Hi"})),
                ("done".to_string(), serde_json::json!({"stopped": null}))
            ]
        );
    }

    #[test]
    fn refusals_carry_the_relays_code() {
        let refusal = ChatRefusal::from_status(429, r#"{"error":"chat_budget","error_description":"Used up"}"#);
        assert_eq!(refusal, ChatRefusal::new("chat_budget", "Used up"));
        assert_eq!(ChatRefusal::from_status(502, "Bad gateway").code, "unavailable");
    }
}
