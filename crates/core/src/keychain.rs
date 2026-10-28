//! The OS keychain (macOS Keychain, Windows Credential Manager, the Secret Service on
//! Linux), for profiles that keep their passwords there instead of in the file.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use keyring_core::{CredentialStore, Error as KeyError};

use crate::error::{CoreError, Result};

const SERVICE: &str = "DataDrop";

/// A handle on the keychain, opened the first time it's needed.
#[derive(Clone)]
pub struct Keychain {
    store: Arc<OnceLock<std::result::Result<Arc<CredentialStore>, String>>>,
    open: fn() -> std::result::Result<Arc<CredentialStore>, String>,
    /// Secrets already read, so the keychain is asked (and may prompt) once each.
    cache: Arc<Mutex<HashMap<String, Option<String>>>>,
}

impl std::fmt::Debug for Keychain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keychain")
    }
}

impl Keychain {
    /// The system's keychain.
    pub fn system() -> Self {
        Self {
            store: Arc::default(),
            open: open_system,
            cache: Arc::default(),
        }
    }

    /// A keychain that only lasts as long as this value (and its clones), for tests.
    pub fn memory() -> Self {
        let store: Arc<CredentialStore> = keyring_core::mock::Store::new().expect("mock store");
        let cell = OnceLock::new();
        let _ = cell.set(Ok(store));
        Self {
            store: Arc::new(cell),
            open: open_system,
            cache: Arc::default(),
        }
    }

    /// What the keychain is called on this system.
    pub fn name() -> &'static str {
        if cfg!(target_os = "macos") {
            "the macOS Keychain"
        } else if cfg!(windows) {
            "Windows Credential Manager"
        } else {
            "the system keyring (Secret Service)"
        }
    }

    fn store(&self) -> Result<&Arc<CredentialStore>> {
        self.store
            .get_or_init(self.open)
            .as_ref()
            .map_err(|e| CoreError::Keychain(format!("{} isn't available: {e}", Self::name())))
    }

    /// Whether the keychain can be used (and why not).
    pub fn check(&self) -> std::result::Result<(), String> {
        match self.store() {
            Ok(_) => Ok(()),
            Err(CoreError::Keychain(why)) => Err(why),
            Err(e) => Err(e.to_string()),
        }
    }

    fn entry(&self, account: &str) -> Result<keyring_core::Entry> {
        self.store()?
            .build(SERVICE, account, None)
            .map_err(|e| failed("opening", account, e))
    }

    pub fn get(&self, account: &str) -> Result<Option<String>> {
        if let Some(s) = self.cache().get(account) {
            return Ok(s.clone());
        }
        let secret = match self.entry(account)?.get_password() {
            Ok(p) => Some(p),
            Err(KeyError::NoEntry) => None,
            Err(e) => return Err(failed("reading", account, e)),
        };
        self.cache().insert(account.to_owned(), secret.clone());
        Ok(secret)
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<String>>> {
        self.cache.lock().expect("keychain cache")
    }

    /// Store a secret, or (with `None` or an empty one) remove it.
    pub fn set(&self, account: &str, secret: Option<&str>) -> Result<()> {
        match secret.filter(|s| !s.is_empty()) {
            Some(s) => {
                self.cache().remove(account);
                self.entry(account)?
                    .set_password(s)
                    .map_err(|e| failed("saving", account, e))?;
                self.cache().insert(account.to_owned(), Some(s.to_owned()));
                Ok(())
            }
            None => self.delete(account),
        }
    }

    pub fn delete(&self, account: &str) -> Result<()> {
        self.cache().remove(account);
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(KeyError::NoEntry) => Ok(()),
            Err(e) => Err(failed("removing", account, e)),
        }
    }
}

fn failed(doing: &str, account: &str, e: KeyError) -> CoreError {
    let why = match e {
        KeyError::NoStorageAccess(e) => format!("it's locked or access was refused ({e})"),
        e => e.to_string(),
    };
    CoreError::Keychain(format!(
        "{doing} the password for '{account}' in {} failed: {why}",
        Keychain::name()
    ))
}

fn open_system() -> std::result::Result<Arc<CredentialStore>, String> {
    #[cfg(target_os = "macos")]
    {
        apple_native_keyring_store::keychain::Store::new()
            .map(|s| s as Arc<CredentialStore>)
            .map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        windows_native_keyring_store::Store::new()
            .map(|s| s as Arc<CredentialStore>)
            .map_err(|e| e.to_string())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        zbus_secret_service_keyring_store::Store::new()
            .map(|s| s as Arc<CredentialStore>)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err("this system has no supported keychain".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_reads_and_removes() {
        let k = Keychain::memory();
        assert_eq!(k.get("shop").unwrap(), None);
        k.set("shop", Some("s3cret")).unwrap();
        assert_eq!(k.get("shop").unwrap().as_deref(), Some("s3cret"));
        k.set("shop", Some("")).unwrap();
        assert_eq!(k.get("shop").unwrap(), None);
        k.delete("shop").unwrap();
    }
}
