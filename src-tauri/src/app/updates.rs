//! New versions from the GitHub releases (`latest.json`, written by `scripts/updater-manifest.sh`). On macOS and
//! Windows the update downloads in the background and installs when the user restarts from the menu or the
//! Connections page; deb and rpm installs cannot replace themselves, so Linux only says a version is out and links to
//! its release. Checked shortly after launch and then every few hours while automatic updates are on.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use super::{notify, state, tray};
use crate::config::Config;
use crate::i18n::{mark, tf};

const RELEASES: &str = "https://github.com/islavov/blankey/releases";
const FIRST_DELAY: Duration = Duration::from_secs(30);
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
#[cfg(target_os = "linux")]
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Idle,
    Checking,
    UpToDate,
    /// out, but this install cannot update itself (Linux)
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Available {
        version: String,
        url: String,
    },
    Downloading {
        version: String,
    },
    /// downloaded and verified, installs on restart
    Ready {
        version: String,
    },
    Failed {
        message: String,
    },
}

#[derive(Clone, Serialize)]
pub struct Info {
    current: &'static str,
    auto: bool,
    #[serde(flatten)]
    status: Status,
}

pub struct Updates {
    handle: AppHandle,
    status: Mutex<Status>,
    auto: Mutex<bool>,
    /// the version last announced in a notification, so each one is announced once
    notified: Mutex<Option<String>>,
    #[cfg(not(target_os = "linux"))]
    ready: Mutex<Option<(tauri_plugin_updater::Update, Vec<u8>)>>,
}

/// BLANKEY_UPDATE_URL points the check at another `latest.json`, for testing; debug builds check only with it.
fn override_url() -> Option<String> {
    std::env::var("BLANKEY_UPDATE_URL").ok().filter(|url| !url.is_empty())
}

fn manifest_url() -> String {
    override_url().unwrap_or_else(|| format!("{RELEASES}/latest/download/latest.json"))
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn release_url(version: &str) -> String {
    format!("{RELEASES}/tag/v{version}")
}

/// The manifest's version when it is newer than `current`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn newer(current: &str, manifest: &str) -> Result<Option<String>, String> {
    let manifest: serde_json::Value = serde_json::from_str(manifest).map_err(|e| format!("bad latest.json: {e}"))?;
    let text = manifest["version"].as_str().ok_or("latest.json has no version")?;
    let text = text.trim_start_matches('v');
    let version = semver::Version::parse(text).map_err(|e| format!("bad version {text}: {e}"))?;
    let current = semver::Version::parse(current).map_err(|e| e.to_string())?;
    Ok((version > current).then(|| text.to_string()))
}

impl Updates {
    pub fn new(handle: AppHandle, auto: bool) -> Self {
        Self {
            handle,
            status: Mutex::new(Status::Idle),
            auto: Mutex::new(auto),
            notified: Mutex::new(None),
            #[cfg(not(target_os = "linux"))]
            ready: Mutex::new(None),
        }
    }

    pub fn info(&self) -> Info {
        Info { current: env!("CARGO_PKG_VERSION"), auto: self.auto(), status: self.status() }
    }

    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn auto(&self) -> bool {
        *self.auto.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_status(&self, status: Status) {
        *self.status.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = status;
        let _ = self.handle.emit("update-status", self.info());
        tray::refresh(&self.handle);
    }

    /// Saved to config.toml as it is on disk now, so other settings changed since launch stay.
    pub fn set_auto(&self, enabled: bool) -> Result<(), String> {
        *self.auto.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = enabled;
        let app = &state(&self.handle).app;
        let mut config = Config::load_from(app.config.data_dir.clone()).map_err(|e| e.to_string())?;
        config.auto_update = enabled;
        config.save().map_err(|e| e.to_string())?;
        app.audit("user", if enabled { "auto_update_on" } else { "auto_update_off" }, "");
        Ok(())
    }

    pub fn start(self: &Arc<Self>) {
        if cfg!(debug_assertions) && override_url().is_none() {
            return;
        }
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_DELAY).await;
            loop {
                if this.auto() {
                    this.check().await;
                }
                tokio::time::sleep(INTERVAL).await;
            }
        });
    }

    /// Looks for a newer version and, where this install can update itself, downloads it.
    pub async fn check(&self) {
        if matches!(self.status(), Status::Checking | Status::Downloading { .. } | Status::Ready { .. }) {
            return;
        }
        self.set_status(Status::Checking);
        match self.fetch().await {
            Ok(status) => self.set_status(status),
            Err(message) => {
                eprintln!("updates: {message}");
                self.set_status(Status::Failed { message });
            }
        }
    }

    fn announce(&self, version: &str, text: &str) {
        let mut notified = self.notified.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if notified.as_deref() != Some(version) {
            let lang = state(&self.handle).app.language();
            notify(&self.handle, "Blankey", &tf(&lang, text, &[("version", &version)]));
            *notified = Some(version.to_string());
        }
    }

    #[cfg(target_os = "linux")]
    async fn fetch(&self) -> Result<Status, String> {
        let url = manifest_url();
        let text = crate::relay::blocking(move || {
            let http = ureq::AgentBuilder::new().timeout(TIMEOUT).build();
            http.get(&url).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())
        })
        .await?;
        Ok(match newer(env!("CARGO_PKG_VERSION"), &text)? {
            Some(version) => {
                self.announce(&version, mark("Blankey {version} is out. Download it from the Blankey menu."));
                Status::Available { url: release_url(&version), version }
            }
            None => Status::UpToDate,
        })
    }

    #[cfg(not(target_os = "linux"))]
    async fn fetch(&self) -> Result<Status, String> {
        use tauri_plugin_updater::UpdaterExt;

        let url = manifest_url().parse().map_err(|e| format!("bad update URL: {e}"))?;
        let builder = self.handle.updater_builder().endpoints(vec![url]).map_err(|e| e.to_string())?;
        // Windows exits right after starting the installer
        let handle = self.handle.clone();
        let builder = builder.on_before_exit(move || {
            state(&handle).app.vault().lock();
            super::platform::clear_opened_documents();
        });
        let Some(update) = builder.build().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())? else {
            return Ok(Status::UpToDate);
        };
        let version = update.version.clone();
        self.set_status(Status::Downloading { version: version.clone() });
        let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
        *self.ready.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((update, bytes));
        self.announce(&version, mark("Blankey {version} is ready. Restart Blankey from its menu to update."));
        Ok(Status::Ready { version })
    }

    /// Installs the downloaded update and starts the new version; the vault locks, as on quit.
    #[cfg(not(target_os = "linux"))]
    pub fn install(&self) -> Result<(), String> {
        if cfg!(debug_assertions) {
            return Err("Development builds do not install updates.".into());
        }
        let ready = self.ready.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
        let (update, bytes) = ready.ok_or("No update is ready.")?;
        state(&self.handle).app.vault().lock();
        super::platform::clear_opened_documents();
        if let Err(e) = update.install(bytes) {
            self.set_status(Status::Failed { message: e.to_string() });
            return Err(e.to_string());
        }
        self.handle.restart()
    }

    /// Opens the release page of the version that is out.
    #[cfg(target_os = "linux")]
    pub fn install(&self) -> Result<(), String> {
        match self.status() {
            Status::Available { url, .. } => open::that(url).map_err(|e| e.to_string()),
            _ => Err("No update is available.".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"{"version": "1.4.0", "notes": "Faster fills", "pub_date": "2026-10-01T09:00:00Z"}"#;

    #[test]
    fn newer_versions_only() {
        assert_eq!(newer("1.3.9", MANIFEST).unwrap().as_deref(), Some("1.4.0"));
        assert_eq!(newer("1.4.0", MANIFEST).unwrap(), None);
        assert_eq!(newer("1.10.0", MANIFEST).unwrap(), None, "compared as versions, not text");
        assert_eq!(newer("1.4.0-rc.1", MANIFEST).unwrap().as_deref(), Some("1.4.0"));
        assert_eq!(newer("1.3.0", r#"{"version": "v1.4.0"}"#).unwrap().as_deref(), Some("1.4.0"));
    }

    #[test]
    fn malformed_manifests_are_errors() {
        assert!(newer("1.3.0", "not json").is_err());
        assert!(newer("1.3.0", r#"{"notes": "no version"}"#).is_err());
        assert!(newer("1.3.0", r#"{"version": "soon"}"#).is_err());
    }

    #[test]
    fn release_page_per_version() {
        assert_eq!(release_url("1.4.0"), "https://github.com/islavov/blankey/releases/tag/v1.4.0");
    }
}
