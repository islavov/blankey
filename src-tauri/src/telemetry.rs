//! Anonymous usage statistics to the official relay (`relay/README.md`, Anonymous metrics): how often this install
//! generated documents, saved templates and fill sets and created profiles, by month, with its version and platform,
//! under a random id from `config.toml`. Signed in or not. On by default, off on the Connections page, and the relay
//! is told once when it goes off.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::runtime::Handle;
use tokio::task::JoinHandle;

use crate::config::{Config, OFFICIAL_RELAY};
use crate::core::Blankey;
use crate::relay::{blocking, oauth::platform};
use crate::vault::kept_months;

/// What development builds send; only a relay with the same key in `TELEMETRY_KEY` takes it. Release builds get the
/// real key from `BLANKEY_TELEMETRY_KEY` at build time.
pub const DEFAULT_KEY: &str = "blankey-dev";
const KEY: &str = match option_env!("BLANKEY_TELEMETRY_KEY") {
    Some(key) if !key.is_empty() => key,
    _ => DEFAULT_KEY,
};
pub const HEADER: &str = "X-Blankey-Telemetry";
const FIRST_DELAY: Duration = Duration::from_secs(60);
const INTERVAL: Duration = Duration::from_secs(60 * 60);
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct Telemetry {
    app: Arc<Blankey>,
    /// whether the install is signed in to Blankey Online right now
    online: Box<dyn Fn() -> bool + Send + Sync>,
    http: ureq::Agent,
    url: String,
    enabled: Mutex<bool>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Telemetry {
    pub fn new(app: Arc<Blankey>, online: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        let config = &app.config;
        let url = config.telemetry_url.clone().unwrap_or_else(|| format!("{OFFICIAL_RELAY}/telemetry"));
        Self {
            enabled: Mutex::new(config.telemetry),
            app,
            online: Box::new(online),
            http: ureq::AgentBuilder::new().timeout(TIMEOUT).max_idle_connections(0).build(),
            url,
            task: Mutex::new(None),
        }
    }

    pub fn enabled(&self) -> bool {
        *self.enabled.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A minute after launch, then hourly, while the statistics are on.
    pub fn start(self: &Arc<Self>, runtime: &Handle) {
        let this = self.clone();
        let task = runtime.spawn(async move {
            tokio::time::sleep(FIRST_DELAY).await;
            loop {
                if let Err(e) = this.send(false).await {
                    eprintln!("telemetry: {e}");
                }
                tokio::time::sleep(INTERVAL).await;
            }
        });
        if let Some(old) = self.task.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).replace(task) {
            old.abort();
        }
    }

    /// Saves the choice; turning off tells the relay once, turning on reports right away. The relay's answer never
    /// matters here: a wrong key or no network is logged, the choice stands.
    pub async fn set_enabled(&self, enabled: bool) -> Result<(), String> {
        *self.enabled.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = enabled;
        let config = Config { telemetry: enabled, ..self.app.config.clone() };
        config.save().map_err(|e| e.to_string())?;
        self.app.audit("user", if enabled { "telemetry_on" } else { "telemetry_off" }, "");
        if let Err(e) = self.send(!enabled).await {
            eprintln!("telemetry: {e}");
        }
        Ok(())
    }

    /// Posts this install's numbers; nothing leaves while the statistics are off, except the opt-out itself.
    pub async fn send(&self, opt_out: bool) -> Result<(), String> {
        if !opt_out && !self.enabled() {
            return Ok(());
        }
        let body = self.payload(opt_out).to_string();
        let (http, url) = (self.http.clone(), self.url.clone());
        blocking(move || {
            let post = http.post(&url).set(HEADER, KEY).set("Content-Type", "application/json");
            post.send_string(&body).map(|_| ()).map_err(|e| e.to_string())
        })
        .await
    }

    fn payload(&self, opt_out: bool) -> Value {
        let months = kept_months();
        let (counts, profiles) = {
            let vault = self.app.vault();
            (vault.usage_months(&months).unwrap_or_default(), vault.list_profiles().map(|p| p.len()).unwrap_or(0))
        };
        json!({
            "v": 1,
            "id": self.app.config.telemetry_id,
            "app": env!("CARGO_PKG_VERSION"),
            "platform": platform(),
            "online": (self.online)(),
            "profiles": profiles,
            "templates": self.app.templates.all().len(),
            "months": counts,
            "opt_out": opt_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::post;
    use axum::{Json, Router};
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Default)]
    struct Stub {
        posts: Mutex<Vec<(Option<String>, Value)>>,
        reject: AtomicBool,
    }

    async fn telemetry(State(stub): State<Arc<Stub>>, headers: HeaderMap, Json(body): Json<Value>) -> StatusCode {
        let key = headers.get(HEADER).and_then(|v| v.to_str().ok()).map(str::to_string);
        stub.posts.lock().unwrap().push((key.clone(), body));
        if stub.reject.load(Ordering::SeqCst) || key.as_deref() != Some(KEY) {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::NO_CONTENT
        }
    }

    async fn start(dir: &std::path::Path) -> (Arc<Stub>, Telemetry, Arc<Blankey>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/telemetry", listener.local_addr().unwrap());
        let stub = Arc::new(Stub::default());
        let router = Router::new().route("/telemetry", post(telemetry)).with_state(stub.clone());
        tokio::spawn(async move { axum::serve(listener, router).await });
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("data/config.toml"), format!("telemetry_url = \"{url}\"\n")).unwrap();
        let app = crate::mcp::tools::tests::app(dir);
        let telemetry = Telemetry::new(app.clone(), || false);
        (stub, telemetry, app)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_this_install_by_month_and_stops_when_turned_off() {
        let dir = tempfile::tempdir().unwrap();
        let (stub, telemetry, app) = start(dir.path()).await;
        assert!(telemetry.enabled());

        telemetry.send(false).await.unwrap();
        let (key, first) = stub.posts.lock().unwrap()[0].clone();
        assert_eq!(key.as_deref(), Some(KEY));
        assert_eq!(first["id"], app.config.telemetry_id);
        assert!(app.config.telemetry_id.starts_with("i-"));
        assert_eq!(first["app"], env!("CARGO_PKG_VERSION"));
        assert_eq!(first["platform"], platform());
        assert_eq!((first["online"].as_bool(), first["opt_out"].as_bool()), (Some(false), Some(false)));
        assert_eq!((first["profiles"].as_u64(), first["templates"].as_u64()), (Some(0), Some(0)));
        let this_month = kept_months()[0].clone();
        assert_eq!(first["months"].as_object().unwrap().len(), 4);
        assert_eq!(first["months"][&this_month]["doc_generated"], 0);
        assert!(!first.to_string().contains("name"), "{first}");

        app.vault().create_profile("Me", "person").unwrap();
        app.audit("user", "create_profile", "1");
        app.audit("user", "generate", "template=t document=1");
        telemetry.send(false).await.unwrap();
        let (_, second) = stub.posts.lock().unwrap()[1].clone();
        assert_eq!(second["profiles"], 1);
        assert_eq!(second["months"][&this_month]["doc_generated"], 1);
        assert_eq!(second["months"][&this_month]["profile_created"], 1);

        telemetry.set_enabled(false).await.unwrap();
        assert!(!telemetry.enabled());
        assert!(!Config::load_from(app.config.data_dir.clone()).unwrap().telemetry);
        let (_, off) = stub.posts.lock().unwrap()[2].clone();
        assert_eq!(off["opt_out"], true);
        telemetry.send(false).await.unwrap();
        assert_eq!(stub.posts.lock().unwrap().len(), 3, "nothing while off");

        telemetry.set_enabled(true).await.unwrap();
        let (_, on) = stub.posts.lock().unwrap()[3].clone();
        assert_eq!(on["opt_out"], false);
        assert!(Config::load_from(app.config.data_dir.clone()).unwrap().telemetry);

        // a relay that does not take the key: logged, nothing else
        stub.reject.store(true, Ordering::SeqCst);
        let refused = telemetry.send(false).await.unwrap_err();
        assert!(refused.contains("401"), "{refused}");
        telemetry.set_enabled(false).await.unwrap();
        assert!(!telemetry.enabled());
        assert!(!Config::load_from(app.config.data_dir.clone()).unwrap().telemetry);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unreachable_relay_is_only_logged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        std::fs::write(dir.path().join("data/config.toml"), "telemetry_url = \"http://127.0.0.1:9/telemetry\"\n")
            .unwrap();
        let app = crate::mcp::tools::tests::app(dir.path());
        let telemetry = Telemetry::new(app.clone(), || false);
        assert!(telemetry.send(false).await.is_err());
        telemetry.set_enabled(false).await.unwrap();
        telemetry.set_enabled(true).await.unwrap();
        assert!(Config::load_from(app.config.data_dir.clone()).unwrap().telemetry);
    }
}
