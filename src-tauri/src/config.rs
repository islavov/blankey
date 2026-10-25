use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "Blankey";
pub const HEALTH_PATH: &str = "/health";
pub const HEALTH_TEXT: &str = "blankey";
const DEFAULT_PORT: u16 = 8765;
pub const OFFICIAL_RELAY: &str = "https://relay.blankey.io";
/// ports tried after the configured one before asking the OS for any free port
const PORT_SEARCH: u16 = 50;

/// config.toml in the data dir; missing keys take their defaults and unknown keys are ignored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
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
            data_dir: PathBuf::new(),
            port: DEFAULT_PORT,
            auto_lock_minutes: 15,
            relay_enabled: false,
            relay_url: OFFICIAL_RELAY.into(),
            telemetry: true,
            telemetry_id: String::new(),
            telemetry_url: None,
            auto_update: true,
            pii_redaction: true,
            language: None,
        }
    }
}

impl Config {
    /// BLANKEY_DATA_DIR, else the per-user data dir (~/Library/Application Support/Blankey on macOS), the same
    /// place the Python app keeps its data. Debug builds use "Blankey Preview" so development never touches it.
    pub fn load() -> std::io::Result<Self> {
        let folder = if cfg!(debug_assertions) { "Blankey Preview" } else { APP_NAME };
        let data_dir = match std::env::var_os("BLANKEY_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => base_dir().join(folder),
        };
        Self::load_from(data_dir)
    }

    pub fn load_from(data_dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let path = data_dir.join("config.toml");
        let mut config = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let invalid =
                    |e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", path.display()));
                Self { data_dir, ..toml::from_str(&text).map_err(invalid)? }
            }
            Err(_) => Self { data_dir, ..Self::default() },
        };
        if config.telemetry_id.is_empty() {
            config.telemetry_id = format!("i-{}", uuid::Uuid::new_v4().simple());
            config.save()?;
        }
        std::fs::create_dir_all(config.templates_dir())?;
        std::fs::create_dir_all(config.previews_dir())?;
        Ok(config)
    }

    /// The data dir the Python app uses; only this one shares its keychain entry.
    pub fn default_data_dir() -> PathBuf {
        base_dir().join(APP_NAME)
    }

    pub fn save(&self) -> std::io::Result<()> {
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(self.config_path(), text)
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir.join("config.toml")
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

    pub fn previews_dir(&self) -> PathBuf {
        self.data_dir.join("previews")
    }

    pub fn mcp_url(&self) -> String {
        format!("http://127.0.0.1:{}/mcp", self.port)
    }

    pub fn health_url(&self) -> String {
        format!("http://127.0.0.1:{}{HEALTH_PATH}", self.port)
    }
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
        assert!(loaded.templates_dir().is_dir() && loaded.previews_dir().is_dir());
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
    fn legacy_config_loads() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), "port = 9001\nauto_lock_minutes = 5\nlegacy_key = \"x\"\n")
            .unwrap();
        let config = Config::load_from(dir.path().to_path_buf()).unwrap();
        assert_eq!((config.port, config.auto_lock_minutes, config.relay_enabled), (9001, 5, false));
        assert_eq!(config.relay_url, OFFICIAL_RELAY);
    }
}
