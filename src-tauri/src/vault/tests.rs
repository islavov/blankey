use std::path::Path;

use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

use super::repo::schema::MIGRATIONS;
use super::*;
use crate::keychain::MemoryStore;

pub const PASSWORD: &str = "correct horse battery staple";

fn vault(dir: &Path) -> Vault {
    let mut vault = Vault::open(&dir.join("vault.db"), Box::new(MemoryStore::default())).unwrap();
    vault.initialize(PASSWORD, false).unwrap();
    vault
}

fn input(key: &str, value: &str) -> FieldInput {
    FieldInput::new(key, value, "", "text")
}

#[test]
fn values_are_encrypted_at_rest() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    let pid = vault.create_profile("Иван", "person").unwrap();
    vault.set_values(pid, &[input("name", "Sentinel Hiddenwell")]).unwrap();
    let raw: Vec<u8> = vault.repo.raw().query_row("SELECT ciphertext FROM fields", [], |r| r.get(0)).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("Sentinel"));
    let db = std::fs::read(dir.path().join("vault.db")).unwrap();
    assert!(!String::from_utf8_lossy(&db).contains("Sentinel"));
}

#[test]
fn lock_unlock_metadata_and_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
    let recovery = vault.initialize("pw", false).unwrap();
    let pid = vault.create_profile("Иван", "person").unwrap();
    vault.set_values(pid, &[input("name", "Иван Иванов"), input("empty", "")]).unwrap();
    vault.lock();
    drop(vault);

    let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
    let locked = vault.fields(pid).unwrap();
    assert!(locked.iter().all(|f| f.value.is_none()));
    assert_eq!(locked.iter().find(|f| f.key == "name").unwrap().length, 11);
    assert!(matches!(vault.get_values(pid), Err(VaultError::Locked)));
    assert!(matches!(vault.unlock("nope"), Err(VaultError::WrongSecret)));
    vault.unlock_with_recovery(&recovery.to_lowercase(), "pw2").unwrap();
    vault.lock();
    assert!(vault.unlock("pw").is_err());
    vault.unlock("pw2").unwrap();
    assert_eq!(vault.get_values(pid).unwrap()["name"].1, "Иван Иванов");
}

#[test]
fn length_counts_nfc_characters() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    let pid = vault.create_profile("x", "").unwrap();
    vault.set_values(pid, &[input("name", "Йордан")]).unwrap();
    vault.set_values(pid, &[input("decomposed", "Йордан".nfd().collect::<String>().as_str())]).unwrap();
    let lengths: Vec<i64> = vault.describe(pid).unwrap().iter().map(|f| f.length).collect();
    assert_eq!(lengths, [6, 6]);
}

#[test]
fn keyring_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    assert!(!vault.unlock_with_keyring());
    vault.enable_keyring().unwrap();
    vault.lock();
    assert!(vault.unlock_with_keyring());
    vault.disable_keyring().unwrap();
    vault.lock();
    assert!(!vault.unlock_with_keyring());
}

#[test]
fn ciphertext_cannot_be_moved_between_fields() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    let pid = vault.create_profile("x", "").unwrap();
    vault.set_values(pid, &[input("a", "secret a"), input("b", "secret b")]).unwrap();
    vault
        .repo
        .raw()
        .execute_batch(
            "UPDATE fields SET nonce = (SELECT nonce FROM fields WHERE key = 'a'),
             ciphertext = (SELECT ciphertext FROM fields WHERE key = 'a') WHERE key = 'b'",
        )
        .unwrap();
    assert!(vault.get_values(pid).is_err());
}

#[test]
fn documents_and_fill_sets_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    let profiles: Profiles = [("applicant".to_string(), 1)].into_iter().collect();
    let doc = vault.store_document("t", "Title", &profiles, 1, "t.pdf", b"%PDF-1.7").unwrap();
    assert_eq!(vault.load_document(doc).unwrap().1, b"%PDF-1.7");
    assert_eq!(vault.list_documents().unwrap()[0].profiles["applicant"], json!(1));

    let bindings: Bindings = serde_json::from_value(json!({"amount": {"value": "34 000"}})).unwrap();
    let id = vault.save_fill_set("acme", &["loan".into()], &profiles, &bindings, None).unwrap();
    assert_eq!(vault.find_fill_set("acme").unwrap(), Some(id));
    let (info, loaded) = vault.get_fill_set(id).unwrap();
    assert_eq!((info.templates, loaded), (vec!["loan".to_string()], bindings));
    vault.lock();
    assert!(vault.get_fill_set(id).is_err());
    assert_eq!(vault.list_fill_sets().unwrap().len(), 1);
}

#[test]
fn backup_rotation() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    let pid = vault.create_profile("x", "").unwrap();
    for i in 0..5 {
        vault.set_values(pid, &[input("n", &i.to_string())]).unwrap();
    }
    for i in 1..=3 {
        assert!(dir.path().join(format!("vault.db.{i}")).exists());
    }
    assert!(!dir.path().join("vault.db.4").exists());
}

#[test]
fn requests_are_sealed_and_need_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    vault.lock();
    let id = vault.create_request("fill", &json!({"reason": "Needed"}), None).unwrap();
    let raw: Vec<u8> = vault.repo.raw().query_row("SELECT sealed_payload FROM requests", [], |r| r.get(0)).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("Needed"));
    let request = vault.get_request(id).unwrap();
    assert!(request.hidden && request.payload.is_empty() && request.status == "pending");
    vault.resolve_request(id, "done", json!({"outcome": "saved"})).unwrap();
    assert_eq!(vault.get_request(id).unwrap().result, Some(json!({"outcome": "saved"})));
    vault.unlock(PASSWORD).unwrap();
    assert_eq!(vault.get_request(id).unwrap().payload["reason"], json!("Needed"));
    assert_eq!(vault.count_requests("done").unwrap(), 1);
}

#[test]
fn requests_need_a_key_pair() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(&dir.path().join("vault.db"), Box::new(MemoryStore::default())).unwrap();
    assert!(matches!(vault.create_request("fill", &json!({}), None), Err(VaultError::NoRequestKey)));
}

#[test]
fn plaintext_requests_from_before_sealing_are_sealed_on_unlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    vault.lock();
    vault
        .repo
        .raw()
        .execute(
            "INSERT INTO requests (kind, payload, status, created_at) VALUES ('fill', '{\"reason\": \"Old\"}', 'pending', 'x')",
            [],
        )
        .unwrap();
    vault.unlock(PASSWORD).unwrap();
    let payload: String = vault.repo.raw().query_row("SELECT payload FROM requests", [], |r| r.get(0)).unwrap();
    assert_eq!(payload, "{}");
    assert_eq!(vault.list_requests(None).unwrap()[0].payload["reason"], json!("Old"));
}

#[test]
fn migrations_rerun_safely() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
    vault.initialize(PASSWORD, false).unwrap();
    vault.create_profile("John", "person").unwrap();
    vault.repo.raw().pragma_update(None, "user_version", 0).unwrap();
    drop(vault);

    let vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
    let version: i64 = vault.repo.raw().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(version, MIGRATIONS.len() as i64);
    assert_eq!(vault.list_profiles().unwrap()[0].name, "John");
}

#[test]
fn relay_grants_work_while_locked() {
    let dir = tempfile::tempdir().unwrap();
    let mut vault = vault(dir.path());
    vault.lock();
    let grant = |id: &str, relay: &str| RelayGrant {
        grant_id: id.into(),
        relay_url: relay.into(),
        client_name: "Claude".into(),
        redirect_host: "claude.ai".into(),
        accepted_at: now(),
    };
    vault.add_relay_grant(&grant("g-1", "https://a")).unwrap();
    vault.add_relay_grant(&grant("g-2", "https://a")).unwrap();
    vault.add_relay_grant(&grant("g-3", "https://b")).unwrap();
    assert!(vault.relay_grant_allowed("g-1", "https://a").unwrap());
    assert!(!vault.relay_grant_allowed("g-1", "https://b").unwrap());

    vault.revoke_relay_grant("g-1").unwrap();
    assert!(!vault.relay_grant_allowed("g-1", "https://a").unwrap());
    let ids: Vec<String> = vault.relay_grants("https://a").unwrap().into_iter().map(|g| g.grant_id).collect();
    assert_eq!(ids, ["g-2"]);

    vault.clear_relay_grants("https://a").unwrap();
    assert!(vault.relay_grants("https://a").unwrap().is_empty());
    assert_eq!(vault.relay_grants("https://b").unwrap().len(), 1);

    assert_eq!(vault.relay_device_id().unwrap(), None);
    vault.set_relay_device_id(Some("d-1")).unwrap();
    assert_eq!(vault.relay_device_id().unwrap().as_deref(), Some("d-1"));
    vault.set_relay_device_id(None).unwrap();
    assert_eq!(vault.relay_device_id().unwrap(), None);
}

/// A vault written by the Python app opens and decrypts unchanged.
#[test]
fn reads_python_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    std::fs::copy(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/vault_v3.db"), &path).unwrap();
    let mut vault = Vault::open(&path, Box::new(MemoryStore::default())).unwrap();
    let profiles = vault.list_profiles().unwrap();
    assert_eq!((profiles[0].name.as_str(), profiles[0].kind.as_str()), ("John", "person"));
    let lengths: Vec<(String, i64)> = vault.describe(1).unwrap().into_iter().map(|f| (f.key, f.length)).collect();
    assert_eq!(lengths, [("egn".to_string(), 0), ("name".to_string(), 11)]);
    let requests = vault.list_requests(None).unwrap();
    assert!(requests[0].hidden && requests[0].status == "done");

    vault.unlock("legacy-pw").unwrap();
    assert_eq!(vault.get_values(1).unwrap()["name"].1, "John Watson");
    let id = vault.find_fill_set("acme").unwrap().unwrap();
    let (info, bindings) = vault.get_fill_set(id).unwrap();
    assert_eq!(info.templates, ["loan"]);
    assert_eq!(info.profiles["manager"], 1);
    assert_eq!(Value::Object(bindings), json!({"amount": {"value": "34 000"}}));
    let doc = &vault.list_documents().unwrap()[0];
    assert_eq!(doc.title, "Contract");
    assert_eq!(vault.load_document(doc.id).unwrap().1, b"%PDF-legacy");
    let request = &vault.list_requests(None).unwrap()[0];
    assert_eq!(Value::Object(request.payload.clone()), json!({"reason": "Fill in"}));
    assert_eq!(request.result, Some(json!({"outcome": "saved"})));

    // the Python schema has no default for requests.payload
    let id = vault.create_request("fill", &json!({"reason": "New"}), None).unwrap();
    assert_eq!(vault.get_request(id).unwrap().payload["reason"], json!("New"));
}
