use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::workspaces::Workspaces;

pub const APP_NAME: &str = "Blankey";
pub const HEALTH_PATH: &str = "/health";
pub const HEALTH_TEXT: &str = "blankey";
const DEFAULT_PORT: u16 = 8765;
const FILE: &str = "config.toml";
pub const OFFICIAL_RELAY: &str = "https://relay.blankey.io";
/// ports tried after the configured one before asking the OS for any free port
const PORT_SEARCH: u16 = 50;

/// config.toml; missing keys take their defaults and unknown keys are ignored. The settings are the app's, kept in
/// the root dir, except the Blankey Online server (`relay_enabled`, `relay_url`), which each workspace has its own
/// of: the first workspace's are in the root file too, any other workspace's in a config.toml in its dir.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where the app keeps its settings and the list of workspaces; also the first workspace's data dir.
    #[serde(skip)]
    pub root_dir: PathBuf,
    /// The open workspace's data.
    #[serde(skip)]
    pub data_dir: PathBuf,
    pub port: u16,
    pub auto_lock_minutes: u32,
    pub relay_enabled: bool,
    pub relay_url: String,
    /// Anonymous usage statistics to the official relay (src/telemetry.rs); off by the user on the Connections page.
    pub telemetry: bool,
    /// A random id this install reports under, made on first load; unrelated to any account or device.
    pub telemetry_id: String,
    /// Where the statistics go instead of the official relay; for development and tests.
    pub telemetry_url: Option<String>,
    /// Feature usage to Mixpanel during the beta (src/mixpanel.rs); off by "Disable BETA data collection" in Settings.
    pub beta_data: bool,
    /// The host the beta usage events go to instead of Mixpanel's; for development and tests.
    pub mixpanel_url: Option<String>,
    /// Look for new versions and download them (src/app/updates.rs); off by the user on the Connections page.
    pub auto_update: bool,
    /// Values that look personal reach the assistant as placeholders (src/pii); a user setting only.
    pub pii_redaction: bool,
    /// The UI language picked in Settings; while set, the Blankey Online account's language never changes it.
    pub language: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::new(),
            data_dir: PathBuf::new(),
            port: DEFAULT_PORT,
            auto_lock_minutes: 15,
            relay_enabled: false,
            relay_url: OFFICIAL_RELAY.into(),
            telemetry: true,
            telemetry_id: String::new(),
            telemetry_url: None,
            beta_data: true,
            mixpanel_url: None,
            auto_update: true,
            pii_redaction: true,
            language: None,
        }
    }
}

impl Config {
    /// BLANKEY_DATA_DIR, else the per-user data dir (~/Library/Application Support/Blankey on macOS), the same
    /// place the Python app keeps its data. Debug builds use "Blankey Preview" so development never touches it.
    /// The open workspace of that root dir.
    pub fn load() -> std::io::Result<Self> {
        let folder = if cfg!(debug_assertions) { "Blankey Preview" } else { APP_NAME };
        let root_dir = match std::env::var_os("BLANKEY_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => base_dir().join(folder),
        };
        std::fs::create_dir_all(&root_dir)?;
        let workspaces = Workspaces::load(&root_dir);
        let data_dir = workspaces.dir(&workspaces.active);
        Self::load_dirs(root_dir, data_dir)
    }

    /// A root dir with its first workspace.
    pub fn load_from(data_dir: PathBuf) -> std::io::Result<Self> {
        Self::load_dirs(data_dir.clone(), data_dir)
    }

    pub fn load_dirs(root_dir: PathBuf, data_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root_dir)?;
        std::fs::create_dir_all(&data_dir)?;
        let mut config: Self = read(&root_dir.join(FILE))?;
        if data_dir != root_dir {
            let own: Online = read(&data_dir.join(FILE))?;
            (config.relay_enabled, config.relay_url) = (own.relay_enabled, own.relay_url);
        }
        (config.root_dir, config.data_dir) = (root_dir, data_dir);
        if config.telemetry_id.is_empty() {
            config.telemetry_id = format!("i-{}", uuid::Uuid::new_v4().simple());
            config.save()?;
        }
        std::fs::create_dir_all(config.templates_dir())?;
        Ok(config)
    }

    /// The data dir the Python app uses; only this one shares its keychain entry.
    pub fn default_data_dir() -> PathBuf {
        base_dir().join(APP_NAME)
    }

    /// As it is on disk now, so settings changed since launch are there.
    pub fn reload(&self) -> std::io::Result<Self> {
        Self::load_dirs(self.root_dir.clone(), self.data_dir.clone())
    }

    pub fn save(&self) -> std::io::Result<()> {
        let mut root = self.clone();
        if self.data_dir != self.root_dir {
            // the root file keeps the first workspace's server
            let first: Online = read(&self.config_path())?;
            (root.relay_enabled, root.relay_url) = (first.relay_enabled, first.relay_url);
            let own = Online { relay_enabled: self.relay_enabled, relay_url: self.relay_url.clone() };
            std::fs::write(self.data_dir.join(FILE), toml::to_string(&own).map_err(std::io::Error::other)?)?;
        }
        std::fs::write(self.config_path(), toml::to_string(&root).map_err(std::io::Error::other)?)
    }

    pub fn config_path(&self) -> PathBuf {
        self.root_dir.join(FILE)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("vault.db")
    }

    pub fn templates_dir(&self) -> PathBuf {
        self.data_dir.join("templates")
    }

    pub fn library_db_path(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }

    /// The document config set from the relay, as last served.
    pub fn doc_configs_path(&self) -> PathBuf {
        self.data_dir.join("doc-configs.json")
    }

    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}{HEALTH_PATH}", self.port)
    }
}

/// The Blankey Online server of a workspace: what its own config.toml holds, and what is read of the root file for
/// the first one.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Online {
    relay_enabled: bool,
    relay_url: String,
}

impl Default for Online {
    fn default() -> Self {
        Self { relay_enabled: false, relay_url: OFFICIAL_RELAY.into() }
    }
}

/// A missing file reads as the defaults.
fn read<T: serde::de::DeserializeOwned + Default>(path: &Path) -> std::io::Result<T> {
    let Ok(text) = std::fs::read_to_string(path) else { return Ok(T::default()) };
    toml::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display())))
}

/// %LOCALAPPDATA% on Windows, as the Python app's platformdirs chose; the usual data dir elsewhere.
fn base_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn port_available(port: u16) -> bool {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// The preferred port when it is free, else the next free one above it, else any port the OS hands out.
pub fn free_port(preferred: u16) -> u16 {
    let last = preferred.saturating_add(PORT_SEARCH);
    (preferred..last).find(|&port| port_available(port)).unwrap_or_else(|| {
        TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .unwrap_or(preferred)
    })
}

/// True only if Blankey itself answers; another program on the same port does not count.
pub fn server_running(health_url: &str) -> bool {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(1)).build();
    agent.get(health_url).call().ok().and_then(|r| r.into_string().ok()).is_some_and(|text| text.trim() == HEALTH_TEXT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_port_skips_a_port_in_use() {
        let busy = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = busy.local_addr().unwrap().port();
        assert_ne!(free_port(port), port);
    }

    #[test]
    fn config_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::load_from(dir.path().join("data")).unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        config.port = 9999;
        config.save().unwrap();
        let loaded = Config::load_from(dir.path().join("data")).unwrap();
        assert_eq!((loaded.port, loaded.auto_lock_minutes), (9999, 15));
        assert!(loaded.templates_dir().is_dir());
        assert_eq!((loaded.relay_enabled, loaded.relay_url.as_str()), (false, OFFICIAL_RELAY));

        let mut config = loaded;
        config.relay_enabled = true;
        config.relay_url = "http://localhost:9000".into();
        config.save().unwrap();
        let loaded = Config::load_from(dir.path().join("data")).unwrap();
        assert_eq!((loaded.relay_enabled, loaded.relay_url.as_str()), (true, "http://localhost:9000"));
    }

    #[test]
    fn telemetry_id_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(dir.path().join("data")).unwrap();
        assert!(config.telemetry && config.telemetry_url.is_none() && config.auto_update);
        assert!(config.beta_data && config.mixpanel_url.is_none());
        assert_eq!(config.telemetry_id.len(), 34, "{}", config.telemetry_id);
        assert!(config.telemetry_id.starts_with("i-"));
        let again = Config::load_from(dir.path().join("data")).unwrap();
        assert_eq!(again.telemetry_id, config.telemetry_id);

        let mut config = again;
        config.telemetry = false;
        config.telemetry_url = Some("http://127.0.0.1:1/telemetry".into());
        config.save().unwrap();
        let loaded = Config::load_from(dir.path().join("data")).unwrap();
        assert_eq!((loaded.telemetry, loaded.telemetry_url.as_deref()), (false, Some("http://127.0.0.1:1/telemetry")));
    }

    #[test]
    fn another_workspace_has_its_own_online_server_and_the_apps_settings() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let mut first = Config::load_from(root.clone()).unwrap();
        first.port = 9999;
        first.relay_enabled = true;
        first.relay_url = "http://localhost:9000".into();
        first.save().unwrap();

        let other = root.join("workspaces").join("w");
        let mut config = Config::load_dirs(root.clone(), other.clone()).unwrap();
        assert_eq!((config.port, config.telemetry_id.as_str()), (9999, first.telemetry_id.as_str()));
        assert_eq!((config.relay_enabled, config.relay_url.as_str()), (false, OFFICIAL_RELAY));
        assert!(other.join("templates").is_dir() && config.db_path().starts_with(&other));

        config.relay_url = "http://localhost:9001".into();
        config.auto_lock_minutes = 3;
        config.save().unwrap();
        let again = config.reload().unwrap();
        assert_eq!((again.relay_url.as_str(), again.auto_lock_minutes), ("http://localhost:9001", 3));
        let first = Config::load_from(root).unwrap();
        assert_eq!((first.relay_enabled, first.relay_url.as_str()), (true, "http://localhost:9000"));
        assert_eq!(first.auto_lock_minutes, 3, "the app's settings are shared");
    }

    #[test]
    fn legacy_config_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "port = 9001\nauto_lock_minutes = 5\nlegacy_key = \"x\"\n")
            .unwrap();
        let config = Config::load_from(dir.path().to_path_buf()).unwrap();
        assert_eq!((config.port, config.auto_lock_minutes, config.relay_enabled), (9001, 5, false));
        assert_eq!(config.relay_url, OFFICIAL_RELAY);
    }
}
