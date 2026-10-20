//! Requests from assistants. Payloads and results are sealed to the request public key, so a request can be stored
//! while the vault is locked and only an unlocked vault can read it. Kind, status and timestamps stay readable.

use serde_json::{Map, Value};

use super::encrypt;
use super::error::{Result, VaultError};
use super::repo::StoredRequest;
use super::{Request, Vault};

/// How many requests a listing without a status returns.
const LATEST: i64 = 100;

impl Vault {
    pub fn create_request(&self, kind: &str, payload: &Value, grant: Option<&str>) -> Result<i64> {
        let public = self.request_public_key()?;
        let id = self.repo.insert_request(kind, grant)?;
        let sealed = encrypt::seal_request(&public, id, "payload", &serde_json::to_vec(payload)?)?;
        self.repo.set_sealed_payload(id, &sealed)?;
        Ok(id)
    }

    pub fn get_request(&self, id: i64) -> Result<Request> {
        let stored = self.repo.request(id)?.ok_or_else(|| VaultError::NotFound(format!("Unknown request {id}")))?;
        self.open_request(stored)
    }

    /// With a status: those requests oldest first; without: the latest 100.
    pub fn list_requests(&self, status: Option<&str>) -> Result<Vec<Request>> {
        let stored = match status {
            Some(status) => self.repo.requests_with_status(status)?,
            None => self.repo.latest_requests(LATEST)?,
        };
        stored.into_iter().map(|r| self.open_request(r)).collect()
    }

    pub fn count_requests(&self, status: &str) -> Result<i64> {
        self.repo.count_requests(status)
    }

    pub fn resolve_request(&mut self, id: i64, status: &str, result: Value) -> Result<()> {
        let public = self.request_public_key()?;
        let sealed = encrypt::seal_request(&public, id, "result", &serde_json::to_vec(&result)?)?;
        self.repo.resolve_request(id, status, &sealed)?;
        self.results.insert(id, result);
        Ok(())
    }

    /// Unseals payload and result when unlocked; while locked the payload is hidden and only results resolved in
    /// this process are known.
    fn open_request(&self, r: StoredRequest) -> Result<Request> {
        let private = self.seal_private.as_ref();
        let open = |sealed: &[u8], part: &str| -> Result<Value> {
            let plain = encrypt::unseal_request(private.ok_or(VaultError::Locked)?, r.id, part, sealed)?;
            Ok(serde_json::from_slice(&plain)?)
        };
        let mut hidden = false;
        let payload = match &r.sealed_payload {
            Some(_) if private.is_none() => {
                hidden = true;
                Value::Object(Map::new())
            }
            Some(sealed) => open(sealed, "payload")?,
            None => serde_json::from_str(&r.payload).unwrap_or_default(),
        };
        let result = match (self.results.get(&r.id), &r.sealed_result) {
            (Some(cached), _) => Some(cached.clone()),
            (None, Some(sealed)) if private.is_some() => Some(open(sealed, "result")?),
            (None, Some(_)) => None,
            (None, None) => r.result.as_deref().and_then(|text| serde_json::from_str(text).ok()),
        };
        let payload = match payload {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        Ok(Request {
            id: r.id,
            kind: r.kind,
            payload,
            status: r.status,
            result,
            created_at: r.created_at,
            resolved_at: r.resolved_at,
            grant_id: r.grant_id,
            hidden,
        })
    }
}
