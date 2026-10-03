//! AES-GCM envelopes bind each value to its row and column with associated data.
//! The key remains in the OS credential store; it is never saved beside SQLite.

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng, Payload},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::STANDARD, Engine};

const PREFIX: &str = "reflow:encrypted:v1:";

pub trait HistoryKeyProvider: Send + Sync {
    /// `create` is true only on the first explicit enable. Missing keys for an
    /// already encrypted database must never be silently replaced.
    fn load_key(&self, create: bool) -> Result<[u8; 32], String>;
}

pub struct OsHistoryKeyProvider;

impl HistoryKeyProvider for OsHistoryKeyProvider {
    fn load_key(&self, create: bool) -> Result<[u8; 32], String> {
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        return Err("History encryption needs a supported OS credential store".into());
        #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux"))]
        {
            let entry = keyring::Entry::new("reflow", "history-key")
                .map_err(|_| "The OS credential store is unavailable".to_string())?;
            match entry.get_secret() {
                Ok(secret) => secret.try_into()
                    .map_err(|_| "The stored history encryption key is invalid".to_string()),
                Err(keyring::Error::NoEntry) if create => {
                    let key = Aes256Gcm::generate_key(&mut OsRng);
                    entry.set_secret(&key)
                        .map_err(|_| "Could not save the history key in the OS credential store".to_string())?;
                    // Verify persistence before any database contents are changed.
                    let stored = entry.get_secret()
                        .map_err(|_| "Could not verify the saved history key".to_string())?;
                    if stored.as_slice() != key.as_slice() {
                        return Err("The OS credential store did not retain the history key".into());
                    }
                    Ok(key.into())
                }
                Err(keyring::Error::NoEntry) => Err("History encryption key is missing from the OS credential store. The original database was preserved.".into()),
                Err(_) => Err("Could not access the history key in the OS credential store".into()),
            }
        }
    }
}

pub(super) struct HistoryCipher(Aes256Gcm);

impl HistoryCipher {
    pub fn new(key: [u8; 32]) -> Self {
        Self(Aes256Gcm::new(&key.into()))
    }

    pub fn seal(&self, id: &str, field: &str, value: &[u8]) -> Result<Vec<u8>, String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let aad = format!("reflow/history/v1/{id}/{field}");
        let encrypted = self
            .0
            .encrypt(
                &nonce,
                Payload {
                    msg: value,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "Could not encrypt history value".to_string())?;
        let mut envelope = nonce.to_vec();
        envelope.extend(encrypted);
        Ok(envelope)
    }

    pub fn open(&self, id: &str, field: &str, value: &[u8]) -> Result<Vec<u8>, String> {
        if value.len() < 28 {
            return Err("Encrypted history value is damaged".into());
        }
        let nonce: [u8; 12] = value[..12].try_into().expect("checked nonce length");
        let aad = format!("reflow/history/v1/{id}/{field}");
        self.0
            .decrypt(
                &Nonce::from(nonce),
                Payload {
                    msg: &value[12..],
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "History authentication failed. The database was preserved.".to_string())
    }

    pub fn seal_text(&self, id: &str, field: &str, value: &str) -> Result<String, String> {
        Ok(format!(
            "{PREFIX}{}",
            STANDARD.encode(self.seal(id, field, value.as_bytes())?)
        ))
    }

    pub fn open_text(&self, id: &str, field: &str, value: &str) -> Result<String, String> {
        let encoded = value
            .strip_prefix(PREFIX)
            .ok_or("Encrypted history envelope is missing")?;
        let envelope = STANDARD
            .decode(encoded)
            .map_err(|_| "Encrypted history envelope is damaged")?;
        String::from_utf8(self.open(id, field, &envelope)?)
            .map_err(|_| "Decrypted history text is invalid".into())
    }
}
