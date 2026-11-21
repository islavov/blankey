//! Feature usage to Mixpanel during the beta: one event when the app opens and when its window gains or loses
//! focus (what Mixpanel builds sessions from), one per audit entry (its action and actor, never its target) and one
//! per page the user opens. The UI records session replays with Mixpanel's own library, all text and inputs masked
//! (ui/js/replay.js). Each event carries the version and platform, under the random id from `config.toml` that the
//! anonymous statistics use, and the account's email while the install is signed in to Blankey Online, which also
//! becomes the email of the install's Mixpanel user profile. Mixpanel adds
//! an approximate location from the sender's address. On by default, off with "Disable BETA data collection" in
//! Settings. Builds without `BLANKEY_MIXPANEL_TOKEN` send nothing.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::config::Config;
use crate::core::Blankey;
use crate::relay::{blocking, oauth::platform};

/// The project token release builds carry; it only lets a sender add events to the project.
pub const TOKEN: Option<&str> = match option_env!("BLANKEY_MIXPANEL_TOKEN") {
    Some(token) if !token.is_empty() => Some(token),
    _ => None,
};
/// The project keeps its data in the EU.
const HOST: &str = "https://api-eu.mixpanel.com";
/// `ip=1` has Mixpanel place the event or profile by the sender's address.
const TRACK: &str = "/track?ip=1&verbose=1";
const ENGAGE: &str = "/engage?ip=1&verbose=1";
const INTERVAL: Duration = Duration::from_secs(10);
const TIMEOUT: Duration = Duration::from_secs(20);
/// events Mixpanel takes in one request
const BATCH: usize = 50;
/// events kept while they cannot be sent; older ones are dropped
const KEPT: usize = 500;

struct Event {
    name: String,
    properties: Value,
    time: i64,
}

pub struct Mixpanel {
    app: Arc<Blankey>,
    /// whether the install is signed in to Blankey Online right now
    online: Box<dyn Fn() -> bool + Send + Sync>,
    http: ureq::Agent,
    host: String,
    token: Option<String>,
    /// the email the install's user profile was last given
    profile_email: Mutex<Option<String>>,
    /// the session replay the UI is recording, which its events are shown beside
    replay_id: Mutex<Option<String>>,
    enabled: Mutex<bool>,
    queue: Mutex<Vec<Event>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// The lock, also after a thread panicked while holding it.
fn held<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Mixpanel {
    /// `token` is the Mixpanel project token; None sends nothing.
    pub fn new(app: Arc<Blankey>, token: Option<&str>, online: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        let config = &app.config;
        Self {
            enabled: Mutex::new(config.beta_data),
            host: config.mixpanel_url.clone().unwrap_or_else(|| HOST.into()),
            token: token.map(str::to_string),
            profile_email: Mutex::new(None),
            replay_id: Mutex::new(None),
            app,
            online: Box::new(online),
            http: ureq::AgentBuilder::new().timeout(TIMEOUT).max_idle_connections(0).build(),
            queue: Mutex::new(Vec::new()),
            task: Mutex::new(None),
        }
    }

    /// Whether the user left the beta data collection on.
    pub fn enabled(&self) -> bool {
        *held(&self.enabled)
    }

    /// Sends what was tracked every few seconds.
    pub fn start(self: &Arc<Self>, runtime: &Handle) {
        let this = self.clone();
        let task = runtime.spawn(async move {
            loop {
                tokio::time::sleep(INTERVAL).await;
                if let Err(e) = this.flush().await {
                    eprintln!("mixpanel: {e}");
                }
            }
        });
        if let Some(old) = held(&self.task).replace(task) {
            old.abort();
        }
    }

    /// Saves the choice; turning off drops what was not sent yet.
    pub fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        *held(&self.enabled) = enabled;
        if !enabled {
            held(&self.queue).clear();
        }
        // as it is on disk now, so other settings changed since launch stay
        let mut config = Config::load_from(self.app.config.data_dir.clone()).map_err(|e| e.to_string())?;
        config.beta_data = enabled;
        config.save().map_err(|e| e.to_string())?;
        self.app.audit("user", if enabled { "beta_data_on" } else { "beta_data_off" }, "");
        Ok(())
    }

    /// What the UI needs to record session replays under the same install; None while nothing is collected.
    pub fn replay_config(&self) -> Option<Value> {
        let token = self.token.as_ref().filter(|_| self.enabled())?;
        Some(json!({ "token": token, "api_host": self.host, "distinct_id": self.app.config.telemetry_id }))
    }

    /// The replay the UI is recording now; later events carry its id.
    pub fn set_replay(&self, id: Option<String>) {
        *held(&self.replay_id) = id;
    }

    /// Keeps the event for the next send; nothing is kept while the collection is off or the build has no token.
    /// Only queues, so it is safe inside the audit hook, where the vault is held.
    pub fn track(&self, name: &str, properties: Value) {
        if self.token.is_none() || !self.enabled() {
            return;
        }
        let mut queue = held(&self.queue);
        if queue.len() >= KEPT {
            queue.remove(0);
        }
        queue.push(Event { name: name.to_string(), properties, time: chrono::Utc::now().timestamp_millis() });
    }

    /// Posts the tracked events, and the user profile when the account's email is new; the events of a failed post
    /// are dropped.
    pub async fn flush(&self) -> Result<(), String> {
        let Some(token) = self.token.clone() else { return Ok(()) };
        let events: Vec<Event> = std::mem::take(&mut *held(&self.queue));
        if !self.enabled() {
            return Ok(());
        }
        let signed_in = (self.online)();
        let email = if signed_in { self.app.vault().relay_account().ok().flatten() } else { None };
        if let Some(email) = email.clone().filter(|email| held(&self.profile_email).as_ref() != Some(email)) {
            let body = json!([{
                "$token": token,
                "$distinct_id": self.app.config.telemetry_id,
                "$set": { "$email": email, "app_version": env!("CARGO_PKG_VERSION"), "platform": platform() },
            }]);
            self.post(ENGAGE, vec![body.to_string()]).await?;
            *held(&self.profile_email) = Some(email);
        }
        if events.is_empty() {
            return Ok(());
        }
        let bodies: Vec<String> = events
            .chunks(BATCH)
            .map(|chunk| {
                Value::from_iter(chunk.iter().map(|event| self.payload(event, &token, signed_in, email.as_deref())))
                    .to_string()
            })
            .collect();
        self.post(TRACK, bodies).await
    }

    /// Posts each body to the Mixpanel endpoint at `path`, stopping at the first that fails.
    async fn post(&self, path: &str, bodies: Vec<String>) -> Result<(), String> {
        let (http, url) = (self.http.clone(), format!("{}{path}", self.host));
        blocking(move || {
            for body in bodies {
                let post = http.post(&url).set("Content-Type", "application/json").set("Accept", "text/plain");
                post.send_string(&body).map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .await
    }

    /// The event as Mixpanel's `/track` takes it: the install's properties, then the event's own.
    fn payload(&self, event: &Event, token: &str, signed_in: bool, email: Option<&str>) -> Value {
        let mut properties = json!({
            "token": token,
            "distinct_id": self.app.config.telemetry_id,
            "time": event.time,
            "$insert_id": uuid::Uuid::new_v4().simple().to_string(),
            "app_version": env!("CARGO_PKG_VERSION"),
            "platform": platform(),
            "signed_in": signed_in,
        });
        if let Some(email) = email {
            properties["email"] = json!(email);
        }
        if let Some(replay) = held(&self.replay_id).as_ref() {
            properties["$mp_replay_id"] = json!(replay);
        }
        if let (Some(all), Some(own)) = (properties.as_object_mut(), event.properties.as_object()) {
            all.extend(own.clone());
        }
        json!({ "event": event.name, "properties": properties })
    }
}

#[cfg(test)]
mod tests {
    use axum::extract::State;
    use axum::routing::post;
    use axum::{Json, Router};
    use tokio::net::TcpListener;

    use super::*;

    type Posts = Arc<Mutex<Vec<Value>>>;
    /// (event posts, profile posts)
    type Stub = (Posts, Posts);

    async fn track(State(stub): State<Stub>, Json(body): Json<Value>) -> &'static str {
        stub.0.lock().unwrap().push(body);
        "1"
    }

    async fn engage(State(stub): State<Stub>, Json(body): Json<Value>) -> &'static str {
        stub.1.lock().unwrap().push(body);
        "1"
    }

    async fn start(dir: &std::path::Path, token: Option<&str>) -> (Posts, Arc<Mixpanel>, Arc<Blankey>) {
        let (posts, _, mixpanel, app) = start_as(dir, token, true).await;
        (posts, mixpanel, app)
    }

    async fn start_as(
        dir: &std::path::Path,
        token: Option<&str>,
        signed_in: bool,
    ) -> (Posts, Posts, Arc<Mixpanel>, Arc<Blankey>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stub = Stub::default();
        let posts = stub.0.clone();
        let profiles = stub.1.clone();
        let router = Router::new().route("/track", post(track)).route("/engage", post(engage)).with_state(stub);
        tokio::spawn(async move { axum::serve(listener, router).await });
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/config.toml"), format!("mixpanel_url = \"{url}\"\n")).unwrap();
        let app = crate::mcp::tools::tests::app(dir);
        let mixpanel = Arc::new(Mixpanel::new(app.clone(), token, move || signed_in));
        let hook = mixpanel.clone();
        app.set_audit_hook(move |actor, action| hook.track(action, json!({ "actor": actor })));
        (posts, profiles, mixpanel, app)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sends_what_the_app_did_without_targets_until_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let (posts, mixpanel, app) = start(dir.path(), Some("tok")).await;
        assert!(mixpanel.enabled());

        mixpanel.track("Page Viewed", json!({ "page": "cases" }));
        app.audit("user", "generate", "template=loan document=7");
        app.vault().audit("mcp", "request:fill", "request-12").unwrap();
        mixpanel.flush().await.unwrap();
        let sent = posts.lock().unwrap()[0].clone();
        let names: Vec<&str> = sent.as_array().unwrap().iter().map(|e| e["event"].as_str().unwrap()).collect();
        assert_eq!(names, ["Page Viewed", "generate", "request:fill"]);
        let page = &sent[0]["properties"];
        assert_eq!((page["token"].as_str(), page["page"].as_str()), (Some("tok"), Some("cases")));
        assert_eq!(page["distinct_id"], app.config.telemetry_id);
        assert_eq!(
            (page["app_version"].as_str(), page["signed_in"].as_bool()),
            (Some(env!("CARGO_PKG_VERSION")), Some(true))
        );
        assert_eq!(page["platform"], platform());
        assert!(page.get("email").is_none(), "the relay named no account: {page}");
        assert!(page["time"].as_i64().unwrap() > 1_700_000_000_000);
        assert_ne!(page["$insert_id"], sent[1]["properties"]["$insert_id"]);
        assert_eq!(sent[1]["properties"]["actor"], "user");
        assert_eq!(sent[2]["properties"]["actor"], "mcp");
        assert!(!sent.to_string().contains("loan") && !sent.to_string().contains("request-12"), "{sent}");

        mixpanel.flush().await.unwrap();
        assert_eq!(posts.lock().unwrap().len(), 1, "nothing new, nothing sent");

        for _ in 0..BATCH + 1 {
            mixpanel.track("Page Viewed", json!({ "page": "templates" }));
        }
        mixpanel.flush().await.unwrap();
        let sizes: Vec<usize> = posts.lock().unwrap()[1..].iter().map(|p| p.as_array().unwrap().len()).collect();
        assert_eq!(sizes, [BATCH, 1]);

        mixpanel.track("Page Viewed", json!({ "page": "profiles" }));
        mixpanel.set_enabled(false).unwrap();
        assert!(!mixpanel.enabled());
        assert!(!Config::load_from(app.config.data_dir.clone()).unwrap().beta_data);
        app.audit("user", "generate", "template=loan document=8");
        mixpanel.flush().await.unwrap();
        assert_eq!(posts.lock().unwrap().len(), 3, "nothing while off");

        mixpanel.set_enabled(true).unwrap();
        assert!(Config::load_from(app.config.data_dir.clone()).unwrap().beta_data);
        mixpanel.flush().await.unwrap();
        let on = posts.lock().unwrap()[3].clone();
        assert_eq!((on.as_array().unwrap().len(), on[0]["event"].as_str()), (1, Some("beta_data_on")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_account_email_goes_along_and_names_the_profile_only_while_signed_in() {
        let dir = tempfile::tempdir().unwrap();
        let (posts, profiles, mixpanel, app) = start_as(dir.path(), Some("tok"), true).await;
        app.vault().set_relay_account(Some("someone@example.com")).unwrap();
        mixpanel.set_replay(Some("replay-1".into()));
        mixpanel.track("App Opened", json!({}));
        mixpanel.flush().await.unwrap();
        let properties = posts.lock().unwrap()[0][0]["properties"].clone();
        assert_eq!(properties["email"], "someone@example.com");
        assert_eq!(properties["$mp_replay_id"], "replay-1");
        let profile = profiles.lock().unwrap()[0][0].clone();
        assert_eq!(
            (profile["$token"].as_str(), &profile["$distinct_id"]),
            (Some("tok"), &json!(app.config.telemetry_id))
        );
        assert_eq!(profile["$set"]["$email"], "someone@example.com");
        assert_eq!(profile["$set"]["platform"], platform());

        mixpanel.track("App Opened", json!({}));
        mixpanel.flush().await.unwrap();
        assert_eq!(profiles.lock().unwrap().len(), 1, "the same email is sent once");
        app.vault().set_relay_account(Some("other@example.com")).unwrap();
        mixpanel.flush().await.unwrap();
        assert_eq!(profiles.lock().unwrap()[1][0]["$set"]["$email"], "other@example.com");
        let config = mixpanel.replay_config().unwrap();
        assert_eq!((config["token"].as_str(), &config["distinct_id"]), (Some("tok"), &json!(app.config.telemetry_id)));
        mixpanel.set_enabled(false).unwrap();
        assert!(mixpanel.replay_config().is_none());

        let other = tempfile::tempdir().unwrap();
        let (posts, profiles, mixpanel, app) = start_as(other.path(), Some("tok"), false).await;
        app.vault().set_relay_account(Some("someone@example.com")).unwrap();
        mixpanel.track("App Opened", json!({}));
        mixpanel.flush().await.unwrap();
        let properties = posts.lock().unwrap()[0][0]["properties"].clone();
        assert_eq!(properties["signed_in"], false);
        assert!(properties.get("email").is_none() && properties.get("$mp_replay_id").is_none(), "{properties}");
        assert!(profiles.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_build_without_a_token_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (posts, mixpanel, app) = start(dir.path(), None).await;
        mixpanel.track("App Opened", json!({}));
        app.audit("user", "generate", "template=loan document=7");
        mixpanel.flush().await.unwrap();
        assert!(posts.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_mixpanel_is_only_an_error_and_drops_the_events() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/config.toml"), "mixpanel_url = \"http://127.0.0.1:9\"\n").unwrap();
        let app = crate::mcp::tools::tests::app(dir.path());
        let mixpanel = Mixpanel::new(app, Some("tok"), || false);
        mixpanel.track("App Opened", json!({}));
        assert!(mixpanel.flush().await.is_err());
        mixpanel.flush().await.unwrap();
    }
}
