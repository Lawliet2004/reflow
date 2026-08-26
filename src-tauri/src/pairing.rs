use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevicePublic {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct PairingOffer {
    pub code: String,
    pub expires_at: Instant,
}

pub struct PairingState {
    offer: RwLock<Option<PairingOffer>>,
    devices: RwLock<Vec<PairedDevice>>,
    path: PathBuf,
}

impl PairingState {
    pub fn new(path: PathBuf) -> Self {
        let devices = load_devices(&path);
        Self {
            offer: RwLock::new(None),
            devices: RwLock::new(devices),
            path,
        }
    }

    pub fn rotate_code(&self) -> PairingOffer {
        let code = generate_code();
        let offer = PairingOffer {
            code,
            expires_at: Instant::now() + PAIRING_TTL,
        };
        *self.offer.write() = Some(offer.clone());
        offer
    }

    pub fn current_offer(&self) -> Option<PairingOffer> {
        let offer = self.offer.read().clone()?;
        if Instant::now() >= offer.expires_at {
            *self.offer.write() = None;
            return None;
        }
        Some(offer)
    }

    pub fn ensure_offer(&self) -> PairingOffer {
        if let Some(offer) = self.current_offer() {
            return offer;
        }
        self.rotate_code()
    }

    pub fn pair(&self, code: &str, device_name: &str) -> Result<(String, PairedDevicePublic), String> {
        let offer = self.current_offer().ok_or_else(|| "Pairing code expired. Generate a new one on the desktop.".to_string())?;
        if offer.code != code.trim() {
            return Err("Invalid pairing code".into());
        }

        let token = uuid::Uuid::new_v4().to_string();
        let device = PairedDevice {
            id: uuid::Uuid::new_v4().to_string(),
            name: if device_name.trim().is_empty() {
                "Android".into()
            } else {
                device_name.trim().to_string()
            },
            token_hash: hash_token(&token),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        let public = PairedDevicePublic {
            id: device.id.clone(),
            name: device.name.clone(),
            created_at: device.created_at.clone(),
        };
        {
            let mut devices = self.devices.write();
            devices.push(device);
            persist_devices(&self.path, &devices)?;
        }
        *self.offer.write() = None;
        Ok((token, public))
    }

    pub fn authorize(&self, token: &str) -> bool {
        let hash = hash_token(token);
        self.devices.read().iter().any(|d| d.token_hash == hash)
    }

    pub fn list_public(&self) -> Vec<PairedDevicePublic> {
        self.devices
            .read()
            .iter()
            .map(|d| PairedDevicePublic {
                id: d.id.clone(),
                name: d.name.clone(),
                created_at: d.created_at.clone(),
            })
            .collect()
    }

    pub fn revoke(&self, id: &str) -> Result<bool, String> {
        let mut devices = self.devices.write();
        let before = devices.len();
        devices.retain(|d| d.id != id);
        let removed = devices.len() != before;
        persist_devices(&self.path, &devices)?;
        Ok(removed)
    }

    pub fn reset(&self) -> Result<(), String> {
        self.devices.write().clear();
        persist_devices(&self.path, &[])?;
        *self.offer.write() = None;
        Ok(())
    }
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

fn generate_code() -> String {
    let n: u32 = rand::random::<u32>() % 1_000_000;
    format!("{n:06}")
}

fn load_devices(path: &PathBuf) -> Vec<PairedDevice> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn persist_devices(path: &PathBuf, devices: &[PairedDevice]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(devices).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_accepts_valid_code_and_rejects_wrong() {
        let dir = std::env::temp_dir().join(format!("reflow_pair_{}", uuid::Uuid::new_v4()));
        let state = PairingState::new(dir.join("devices.json"));
        let offer = state.rotate_code();
        assert!(state.pair("000000", "phone").is_err());
        let (token, device) = state.pair(&offer.code, "Pixel").expect("pair");
        assert!(state.authorize(&token));
        assert!(!state.authorize("nope"));
        assert_eq!(device.name, "Pixel");
        assert!(state.pair(&offer.code, "again").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn token_hash_is_stable() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }
}
