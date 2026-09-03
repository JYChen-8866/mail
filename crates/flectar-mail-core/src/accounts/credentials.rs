//! Secrets live in the OS keyring (Secret Service on Linux), never in SQLite.
//! Keyring entries are keyed "flectar-mail:<account_id>:<slot>".

use crate::error::{CoreError, Result};
use std::sync::Arc;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
const SERVICE: &str = "flectar-mail";
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_INSECURE_CREDENTIAL_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    Password,
    RefreshToken,
    AccessToken,
    /// OAuth application registration that issued this account's refresh
    /// token. Keeping it per-account prevents a later custom-app override from
    /// invalidating accounts connected with the bundled registration.
    OAuthClientId,
    OAuthClientSecret,
    /// App-level AI API key (stored under account id 0).
    AiApiKey,
    /// Generic-CalDAV app password (Google CalDAV reuses the OAuth tokens).
    CaldavPassword,
}

impl Slot {
    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::Password => "password",
            Slot::RefreshToken => "refresh_token",
            Slot::AccessToken => "access_token",
            Slot::OAuthClientId => "oauth_client_id",
            Slot::OAuthClientSecret => "oauth_client_secret",
            Slot::AiApiKey => "ai_api_key",
            Slot::CaldavPassword => "caldav_password",
        }
    }
}

/// Platform-owned secret persistence. Implementations must be durable across
/// process death and must never put secret material in SQLite or logs.
pub trait CredentialStore: Send + Sync {
    fn store(&self, account_id: i64, slot: Slot, secret: &str) -> Result<()>;
    fn load(&self, account_id: i64, slot: Slot) -> Result<String>;
    fn delete_all(&self, account_id: i64) -> Result<()>;
}

pub type CredentialStoreHandle = Arc<dyn CredentialStore>;

#[derive(Debug, Default)]
pub struct SystemCredentialStore;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn entry(account_id: i64, slot: &Slot) -> Result<keyring::Entry> {
    let user = format!("{account_id}:{}", slot.as_str());
    keyring::Entry::new(SERVICE, &user).map_err(Into::into)
}

// Windows Credential Manager caps a credential blob at 2560 bytes
// (CRED_MAX_CREDENTIAL_BLOB_SIZE) and the native store encodes secrets as
// UTF-16, so ~1280 code units. Microsoft AAD refresh tokens routinely exceed
// that, and the store then fails with a "secure storage error" that leaves the
// account uncredentialed. Split oversized secrets across sibling entries on
// Windows; other platforms have no such limit and store secrets whole.
#[cfg(target_os = "windows")]
const MAX_SECRET_UTF16: usize = 1024;
#[cfg(all(
    not(target_os = "windows"),
    not(any(target_os = "android", target_os = "ios"))
))]
const MAX_SECRET_UTF16: usize = usize::MAX;

// A NUL-prefixed header no real token or password starts with. When present in
// the primary entry it means the value is split across `<slot>:c0..cN` entries.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const CHUNK_MARKER: &str = "\u{0}flectar-mail-chunks:";

// Chunk sanity cap; MAX_SECRET_UTF16 * this is far beyond any real secret.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const MAX_CHUNKS: usize = 256;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn chunk_entry(account_id: i64, slot: &Slot, index: usize) -> Result<keyring::Entry> {
    let user = format!("{account_id}:{}:c{index}", slot.as_str());
    keyring::Entry::new(SERVICE, &user).map_err(Into::into)
}

/// Split on char boundaries so no chunk exceeds `max_units` UTF-16 code units
/// (what the Windows blob limit counts). Returns a single chunk when it fits.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn split_utf16_chunks(secret: &str, max_units: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut cur = String::new();
    let mut units = 0usize;
    for ch in secret.chars() {
        let w = ch.len_utf16();
        if units.saturating_add(w) > max_units && !cur.is_empty() {
            chunks.push(std::mem::take(&mut cur));
            units = 0;
        }
        cur.push(ch);
        units += w;
    }
    chunks.push(cur);
    chunks
}

/// Delete `<slot>:c{index}` entries starting at `start`. Chunks are written
/// contiguously, so stop at the first missing (or unreadable) index.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn delete_chunks_from(account_id: i64, slot: &Slot, start: usize) {
    for index in start..start + MAX_CHUNKS {
        match chunk_entry(account_id, slot, index) {
            Ok(e) => match e.delete_credential() {
                Ok(()) => {}
                _ => break,
            },
            Err(_) => break,
        }
    }
}

/// Fallback for machines without a Secret Service (headless boxes, tests):
/// FLECTAR_MAIL_CREDENTIALS_INSECURE_FILE=<path> stores secrets as plaintext
/// JSON.
/// The OS keyring is always preferred; never set this on a desktop.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn insecure_file() -> Option<std::path::PathBuf> {
    std::env::var("FLECTAR_MAIL_CREDENTIALS_INSECURE_FILE")
        .ok()
        .filter(|p| !p.is_empty())
        .map(std::path::PathBuf::from)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
static INSECURE_FILE_LOCK: once_cell::sync::Lazy<std::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn file_map(path: &std::path::Path) -> Result<std::collections::HashMap<String, String>> {
    use std::io::Read;

    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_INSECURE_CREDENTIAL_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_INSECURE_CREDENTIAL_FILE_BYTES {
        return Err(CoreError::Other(
            "insecure credential file exceeds its 1 MiB safety limit".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| CoreError::Other(format!("invalid insecure credential file: {error}")))
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn file_save(
    path: &std::path::Path,
    map: &std::collections::HashMap<String, String>,
) -> Result<()> {
    use std::io::Write;

    let bytes = serde_json::to_vec(map)?;
    if bytes.len() as u64 > MAX_INSECURE_CREDENTIAL_FILE_BYTES {
        return Err(CoreError::Other(
            "insecure credential file exceeds its 1 MiB safety limit".into(),
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file_mut().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| CoreError::Io(error.error))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn slot_key(account_id: i64, slot: &Slot) -> String {
    format!("{account_id}:{}", slot.as_str())
}

/// Keyring calls are blocking (D-Bus on Linux); call from spawn_blocking in async paths.
fn system_store(account_id: i64, slot: Slot, secret: &str) -> Result<()> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (account_id, slot, secret);
        return Err(CoreError::Keyring(
            "the mobile host did not inject secure credential storage".into(),
        ));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        if let Some(path) = insecure_file() {
            let _guard = INSECURE_FILE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut map = file_map(&path)?;
            map.insert(slot_key(account_id, &slot), secret.to_string());
            return file_save(&path, &map);
        }
        let chunks = split_utf16_chunks(secret, MAX_SECRET_UTF16);
        if chunks.len() <= 1 {
            // Fits in one entry (always the case off Windows). On Windows, clear any
            // stale chunks left by a previous oversized value, then store whole.
            #[cfg(target_os = "windows")]
            delete_chunks_from(account_id, &slot, 0);
            return entry(account_id, &slot)?
                .set_password(secret)
                .map_err(Into::into);
        }
        // Write data chunks first, header last: a partial write never reassembles.
        for (index, part) in chunks.iter().enumerate() {
            chunk_entry(account_id, &slot, index)?
                .set_password(part)
                .map_err(CoreError::from)?;
        }
        delete_chunks_from(account_id, &slot, chunks.len());
        entry(account_id, &slot)?
            .set_password(&format!("{CHUNK_MARKER}{}", chunks.len()))
            .map_err(Into::into)
    }
}

fn system_load(account_id: i64, slot: Slot) -> Result<String> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (account_id, slot);
        return Err(CoreError::Keyring(
            "the mobile host did not inject secure credential storage".into(),
        ));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        if let Some(path) = insecure_file() {
            let _guard = INSECURE_FILE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            return file_map(&path)?
                .remove(&slot_key(account_id, &slot))
                .ok_or_else(|| CoreError::Auth("no stored credential".into()));
        }
        load_current_secret(account_id, &slot)?
            .ok_or_else(|| CoreError::Auth("no stored credential".into()))
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn load_current_secret(account_id: i64, slot: &Slot) -> Result<Option<String>> {
    let head = match entry(account_id, slot)?.get_password() {
        Ok(secret) => secret,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let count = head.strip_prefix(CHUNK_MARKER);
    let Some(count) = count else {
        return Ok(Some(head));
    };
    let count: usize = count
        .parse()
        .map_err(|_| CoreError::Other("corrupt keyring chunk header".into()))?;
    let mut out = String::new();
    for index in 0..count {
        let chunk = chunk_entry(account_id, slot, index)?;
        match chunk.get_password() {
            Ok(s) => out.push_str(&s),
            Err(keyring::Error::NoEntry) => {
                return Err(CoreError::Auth("no stored credential".into()));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(Some(out))
}

fn system_delete_all(account_id: i64) -> Result<()> {
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = account_id;
        return Err(CoreError::Keyring(
            "the mobile host did not inject secure credential storage".into(),
        ));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        if let Some(path) = insecure_file() {
            let _guard = INSECURE_FILE_LOCK
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut map = file_map(&path)?;
            map.retain(|k, _| !k.starts_with(&format!("{account_id}:")));
            return file_save(&path, &map);
        }
        for slot in [
            Slot::Password,
            Slot::RefreshToken,
            Slot::AccessToken,
            Slot::OAuthClientId,
            Slot::OAuthClientSecret,
            Slot::CaldavPassword,
        ] {
            if let Ok(e) = entry(account_id, &slot) {
                let _ = e.delete_credential();
            }
            delete_chunks_from(account_id, &slot, 0);
        }
        Ok(())
    }
}

impl CredentialStore for SystemCredentialStore {
    fn store(&self, account_id: i64, slot: Slot, secret: &str) -> Result<()> {
        system_store(account_id, slot, secret)
    }

    fn load(&self, account_id: i64, slot: Slot) -> Result<String> {
        system_load(account_id, slot)
    }

    fn delete_all(&self, account_id: i64) -> Result<()> {
        system_delete_all(account_id)
    }
}

pub async fn store_async(
    store: CredentialStoreHandle,
    account_id: i64,
    slot: Slot,
    secret: String,
) -> Result<()> {
    tokio::task::spawn_blocking(move || store.store(account_id, slot, &secret))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

pub async fn load_async(
    store: CredentialStoreHandle,
    account_id: i64,
    slot: Slot,
) -> Result<String> {
    tokio::task::spawn_blocking(move || store.load(account_id, slot))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

pub async fn delete_all_async(store: CredentialStoreHandle, account_id: i64) -> Result<()> {
    tokio::task::spawn_blocking(move || store.delete_all(account_id))
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_secret_is_one_chunk() {
        assert_eq!(split_utf16_chunks("abc", 1024), vec!["abc"]);
        assert_eq!(split_utf16_chunks("", 1024), vec![""]);
    }

    #[test]
    fn oversized_secret_splits_and_rejoins() {
        let secret: String = "a".repeat(2500);
        let chunks = split_utf16_chunks(&secret, 1024);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.encode_utf16().count() <= 1024));
        assert_eq!(chunks.concat(), secret);
    }

    #[test]
    fn split_never_breaks_a_surrogate_pair() {
        // '😀' is two UTF-16 code units; a boundary must not land mid-char.
        let secret: String = "😀".repeat(600); // 1200 UTF-16 units
        let chunks = split_utf16_chunks(&secret, 1023); // odd cap, forces a squeeze
        assert!(chunks.iter().all(|c| c.encode_utf16().count() <= 1023));
        assert_eq!(chunks.concat(), secret);
    }

    #[test]
    fn insecure_file_is_bounded_strict_and_atomically_replaceable() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credentials.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(file_map(&path).is_err());

        let first = std::collections::HashMap::from([("1:password".into(), "first".into())]);
        file_save(&path, &first).unwrap();
        assert_eq!(file_map(&path).unwrap(), first);

        let second = std::collections::HashMap::from([("2:password".into(), "second".into())]);
        file_save(&path, &second).unwrap();
        assert_eq!(file_map(&path).unwrap(), second);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[derive(Default)]
    struct MemoryStore(std::sync::Mutex<std::collections::HashMap<(i64, Slot), String>>);

    impl CredentialStore for MemoryStore {
        fn store(&self, account_id: i64, slot: Slot, secret: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert((account_id, slot), secret.into());
            Ok(())
        }

        fn load(&self, account_id: i64, slot: Slot) -> Result<String> {
            self.0
                .lock()
                .unwrap()
                .get(&(account_id, slot))
                .cloned()
                .ok_or_else(|| CoreError::Auth("no stored credential".into()))
        }

        fn delete_all(&self, account_id: i64) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .retain(|(id, _), _| *id != account_id);
            Ok(())
        }
    }

    #[tokio::test]
    async fn injected_store_contract_round_trips_and_deletes_an_account() {
        let store: CredentialStoreHandle = Arc::new(MemoryStore::default());
        store_async(store.clone(), 9, Slot::Password, "secret".into())
            .await
            .unwrap();
        store_async(store.clone(), 10, Slot::Password, "other".into())
            .await
            .unwrap();
        assert_eq!(
            load_async(store.clone(), 9, Slot::Password).await.unwrap(),
            "secret"
        );
        delete_all_async(store.clone(), 9).await.unwrap();
        assert!(load_async(store.clone(), 9, Slot::Password).await.is_err());
        assert_eq!(
            load_async(store, 10, Slot::Password).await.unwrap(),
            "other"
        );
    }
}
