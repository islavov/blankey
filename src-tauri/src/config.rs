use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::PathBuf;

pub const APP_NAME: &str = "Blankey";
pub const HEALTH_PATH: &str = "/health";
pub const HEALTH_TEXT: &str = "blankey";
const DEFAULT_PORT: u16 = 8765;
/// ports tried after the configured one before asking the OS for any free port
const PORT_SEARCH: u16 = 50;

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    pub auto_lock_minutes: u32,
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
        let mut config = Self { data_dir, port: DEFAULT_PORT, auto_lock_minutes: 15 };
        match std::fs::read_to_string(config.config_path()) {
            Ok(text) => {
                for line in text.lines() {
                    let Some((key, value)) = line.split_once('=') else { continue };
                    match (key.trim(), value.trim().parse::<u32>()) {
                        ("port", Ok(v)) => config.port = v as u16,
                        ("auto_lock_minutes", Ok(v)) => config.auto_lock_minutes = v,
                        _ => {}
                    }
                }
            }
            Err(_) => config.save()?,
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
        let text = format!("port = {}\nauto_lock_minutes = {}\n", self.port, self.auto_lock_minutes);
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
    }
}
