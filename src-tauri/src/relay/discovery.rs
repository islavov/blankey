//! The relay's discovery document, `GET {relay}/.well-known/blankey-relay`.

use serde::{Deserialize, Serialize};
use url::Url;

pub const PROTOCOL: u32 = 1;
pub const PATH: &str = "/.well-known/blankey-relay";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Discovery {
    pub protocol: u32,
    pub issuer: String,
    pub device_register: String,
    pub device_connect: String,
    pub device_revoke: String,
    pub mcp_url: String,
    pub account_url: String,
}

impl Discovery {
    pub fn validate(self) -> Result<Self, String> {
        if self.protocol != PROTOCOL {
            return Err(format!(
                "The relay speaks protocol {}; this Blankey needs protocol {PROTOCOL}.",
                self.protocol
            ));
        }
        for url in [&self.issuer, &self.device_register, &self.device_revoke, &self.mcp_url, &self.account_url] {
            check_url(url, "https", "http")?;
        }
        check_url(&self.device_connect, "wss", "ws")?;
        Ok(self)
    }
}

/// The relay URL as the app stores it: trimmed, without a trailing slash, https unless local.
pub fn normalize(url: &str) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/');
    check_url(url, "https", "http")?;
    Ok(url.to_string())
}

/// `secure` everywhere; `plain` only on localhost / 127.0.0.1, for development.
pub fn check_url(raw: &str, secure: &str, plain: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| format!("Not a valid URL: {raw}"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1"));
    if url.scheme() == secure || (url.scheme() == plain && local) {
        Ok(url)
    } else {
        Err(format!("{raw} must use {secure}://"))
    }
}

pub fn fetch(agent: &ureq::Agent, relay_url: &str) -> Result<Discovery, String> {
    let base = normalize(relay_url)?;
    let not_relay = |e: &dyn std::fmt::Display| format!("{base} is not a Blankey relay: {e}");
    let text = agent
        .get(&format!("{base}{PATH}"))
        .call()
        .map_err(|e| not_relay(&e))?
        .into_string()
        .map_err(|e| not_relay(&e))?;
    serde_json::from_str::<Discovery>(&text).map_err(|e| not_relay(&e))?.validate()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn document(base: &str, connect: &str) -> Discovery {
        Discovery {
            protocol: PROTOCOL,
            issuer: base.into(),
            device_register: format!("{base}/device/register"),
            device_connect: connect.into(),
            device_revoke: format!("{base}/device/grants/revoke"),
            mcp_url: format!("{base}/mcp"),
            account_url: format!("{base}/account"),
        }
    }

    #[test]
    fn validation() {
        let official = document("https://relay.example.com", "wss://relay.example.com/device/connect");
        assert!(official.clone().validate().is_ok());
        assert!(document("http://localhost:9000", "ws://127.0.0.1:9001/connect").validate().is_ok());

        let newer = Discovery { protocol: 2, ..official.clone() };
        assert!(newer.validate().unwrap_err().contains("protocol 2"));
        let plain = document("http://relay.example.com", "wss://relay.example.com/device/connect");
        assert!(plain.validate().unwrap_err().contains("https://"));
        let plain_socket = Discovery { device_connect: "ws://relay.example.com/connect".into(), ..official };
        assert!(plain_socket.validate().unwrap_err().contains("wss://"));

        assert_eq!(normalize(" https://relay.example.com/ ").unwrap(), "https://relay.example.com");
        assert!(normalize("http://relay.example.com").is_err());
        assert!(normalize("relay.example.com").is_err());
    }
}
