//! Connection passwords in the OS credential store (macOS Keychain).
//!
//! Items are stored under [`SERVICE`] with the connection id as the account, so
//! renaming a connection or changing its URL keeps the password attached.

use keyring::{Entry, Error};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const SERVICE: &str = "Ludo database connection";

/// Process-wide cache so queries don't hit the Keychain (and possibly a macOS
/// access prompt) every time.
fn cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn entry(conn_id: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, conn_id).map_err(|e| e.to_string())
}

pub fn set(conn_id: &str, password: &str) -> Result<(), String> {
    entry(conn_id)?.set_password(password).map_err(|e| e.to_string())?;
    cache().lock().unwrap().insert(conn_id.to_owned(), password.to_owned());
    Ok(())
}

/// `Ok(None)` when nothing is stored for this connection.
pub fn get(conn_id: &str) -> Result<Option<String>, String> {
    if let Some(p) = cache().lock().unwrap().get(conn_id) {
        return Ok(Some(p.clone()));
    }
    match entry(conn_id)?.get_password() {
        Ok(p) => {
            cache().lock().unwrap().insert(conn_id.to_owned(), p.clone());
            Ok(Some(p))
        }
        Err(Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn delete(conn_id: &str) -> Result<(), String> {
    cache().lock().unwrap().remove(conn_id);
    match entry(conn_id)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        let id = format!("test-{}", crate::db::new_id());
        if let Err(e) = super::set(&id, "s3cr3t") {
            eprintln!("credential store unavailable, skipping: {e}");
            return;
        }
        // Read from the store itself, not the in-memory cache.
        super::cache().lock().unwrap().clear();
        assert_eq!(super::get(&id).unwrap().as_deref(), Some("s3cr3t"));
        let cfg = crate::db::ConnectionConfig { id: id.clone(), keychain_password: true, ..Default::default() };
        assert_eq!(cfg.with_secret().unwrap().password, "s3cr3t");
        super::delete(&id).unwrap();
        assert_eq!(super::get(&id).unwrap(), None);
        super::delete(&id).unwrap(); // deleting twice is fine
    }
}
