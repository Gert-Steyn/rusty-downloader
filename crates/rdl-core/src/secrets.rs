//! Passwords kept out of the database: Windows Credential Manager in the
//! app, memory in tests. Keys are short names like `proxy/3`; the app's own
//! entries all carry the service name [`SERVICE`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::{Error, Result};

/// Service name of every entry in Credential Manager ("rusty-downloader/proxy/3").
pub const SERVICE: &str = "rusty-downloader";

pub trait Secrets: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, secret: &str) -> Result<()>;
    /// Deleting what isn't there is fine.
    fn delete(&self, key: &str) -> Result<()>;
}

/// Secrets that live only as long as the process; for tests and for a
/// manager started without the app.
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.map().get(key).cloned())
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        self.map().insert(key.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.map().remove(key);
        Ok(())
    }
}

/// Windows Credential Manager (generic credentials, per user).
#[cfg(windows)]
pub struct CredentialManager(());

#[cfg(windows)]
impl CredentialManager {
    pub fn new() -> Result<Arc<Self>> {
        let store = windows_native_keyring_store::Store::new().map_err(secret_error)?;
        keyring_core::set_default_store(store);
        Ok(Arc::new(Self(())))
    }

    fn entry(key: &str) -> Result<keyring_core::Entry> {
        keyring_core::Entry::new(SERVICE, key).map_err(secret_error)
    }
}

#[cfg(windows)]
impl Secrets for CredentialManager {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(secret_error(e)),
        }
    }

    fn set(&self, key: &str, secret: &str) -> Result<()> {
        Self::entry(key)?.set_password(secret).map_err(secret_error)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(secret_error(e)),
        }
    }
}

#[cfg(windows)]
fn secret_error(e: keyring_core::Error) -> Error {
    Error::Secrets(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(secrets: &dyn Secrets, key: &str) {
        assert_eq!(secrets.get(key).unwrap(), None);
        secrets.set(key, "pä$$ wörd").unwrap();
        assert_eq!(secrets.get(key).unwrap().as_deref(), Some("pä$$ wörd"));
        secrets.set(key, "second").unwrap();
        assert_eq!(secrets.get(key).unwrap().as_deref(), Some("second"));
        secrets.delete(key).unwrap();
        secrets.delete(key).unwrap();
        assert_eq!(secrets.get(key).unwrap(), None);
    }

    #[test]
    fn memory_secrets_round_trip() {
        round_trip(&*MemorySecrets::new(), "proxy/1");
    }

    /// Writes to the real Credential Manager under a throwaway name, then
    /// removes it. Opt-in: CI's service account may have no credential store.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn credential_manager_round_trip() {
        let key = format!("test/{}-{}", std::process::id(), fastrand::u32(..));
        round_trip(&*CredentialManager::new().unwrap(), &key);
    }
}
