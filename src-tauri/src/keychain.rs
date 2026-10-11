//! Where the keychain-wrapped vault key lives, and the Touch ID check before using it.

use std::sync::Mutex;

const SERVICE: &str = "blankey";
const USER: &str = "vault-kek";

pub trait SecretStore: Send {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, value: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// An OS keychain entry. The default data dir uses ("blankey", "vault-kek"), shared with the Python app; any
/// other data dir gets its own account, so a test vault never replaces the real vault's key.
pub struct OsKeyring {
    user: String,
}

impl OsKeyring {
    pub fn for_data_dir(data_dir: &std::path::Path, default_dir: &std::path::Path) -> Self {
        let canonical = |p: &std::path::Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if canonical(data_dir) == canonical(default_dir) {
            return Self { user: USER.into() };
        }
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(data_dir.to_string_lossy().as_bytes());
        let tag: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        Self { user: format!("{USER}-{tag}") }
    }

    fn entry(&self) -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, &self.user).map_err(|e| e.to_string())
    }
}

impl SecretStore for OsKeyring {
    fn get(&self) -> Result<Option<String>, String> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, value: &str) -> Result<(), String> {
        self.entry()?.set_password(value).map_err(|e| e.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore(Mutex<Option<String>>);

impl SecretStore for MemoryStore {
    fn get(&self) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().clone())
    }

    fn set(&self, value: &str) -> Result<(), String> {
        *self.0.lock().unwrap() = Some(value.to_string());
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        *self.0.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod biometric {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_local_authentication::{LAContext, LAPolicy};

    const PROMPT_TIMEOUT: Duration = Duration::from_secs(60);

    pub fn available() -> bool {
        unsafe {
            let context = LAContext::new();
            context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics).is_ok()
        }
    }

    /// Shows the system Touch ID prompt. False on failure, cancel or "use password".
    pub fn authenticate(reason: &str) -> bool {
        if !available() {
            return false;
        }
        let (tx, rx) = mpsc::channel();
        // the context must outlive the prompt: releasing it cancels the evaluation
        let context = unsafe { LAContext::new() };
        unsafe {
            context.setLocalizedFallbackTitle(Some(&NSString::from_str("Use password")));
            let reply = RcBlock::new(move |success: Bool, _error: *mut NSError| {
                let _ = tx.send(success.as_bool());
            });
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthenticationWithBiometrics,
                &NSString::from_str(reason),
                &reply,
            );
        }
        let ok = rx.recv_timeout(PROMPT_TIMEOUT).unwrap_or(false);
        drop(context);
        ok
    }
}

#[cfg(not(target_os = "macos"))]
mod biometric {
    pub fn available() -> bool {
        false
    }

    pub fn authenticate(_reason: &str) -> bool {
        false
    }
}

pub use biometric::{authenticate, available as biometric_available};
