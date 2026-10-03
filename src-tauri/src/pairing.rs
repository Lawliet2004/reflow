use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PAIRING_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct DevicePermissions {
    pub stream: bool,
    pub history: bool,
    pub injection: bool,
}
impl Default for DevicePermissions {
    fn default() -> Self {
        Self {
            stream: true,
            history: false,
            injection: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
    #[serde(default)]
    pub permissions: DevicePermissions,
    #[serde(default)]
    pub automation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevicePublic {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub permissions: DevicePermissions,
}

#[derive(Debug, Clone)]
pub struct PairingOffer {
    pub code: String,
    pub expires_at: Instant,
    failed_attempts: u8,
}

pub struct PairingState {
    offer: RwLock<Option<PairingOffer>>,
    devices: RwLock<Vec<PairedDevice>>,
    path: PathBuf,
    pub(crate) automation_sessions: tokio::sync::Mutex<std::collections::HashMap<String, u64>>,
}

impl PairingState {
    pub fn new(path: PathBuf) -> Self {
        let devices = load_devices(&path);
        Self {
            offer: RwLock::new(None),
            devices: RwLock::new(devices),
            path,
            automation_sessions: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn rotate_code(&self) -> PairingOffer {
        let code = generate_code();
        let offer = PairingOffer {
            code,
            expires_at: Instant::now() + PAIRING_TTL,
            failed_attempts: 0,
        };
        *self.offer.write() = Some(offer.clone());
        offer
    }

    pub fn current_offer(&self) -> Option<PairingOffer> {
        let mut offer = self.offer.write();
        if offer
            .as_ref()
            .is_some_and(|o| Instant::now() >= o.expires_at)
        {
            *offer = None;
        }
        offer.clone()
    }

    pub fn ensure_offer(&self) -> PairingOffer {
        if let Some(offer) = self.current_offer() {
            return offer;
        }
        self.rotate_code()
    }

    pub fn pair(
        &self,
        code: &str,
        device_name: &str,
    ) -> Result<(String, PairedDevicePublic), String> {
        // Keep the offer locked through persistence and consumption: one code
        // authorizes exactly one device even when requests arrive together.
        let mut offer_guard = self.offer.write();
        let offer = offer_guard.as_mut().ok_or_else(|| {
            "Pairing code expired. Generate a new one on the desktop.".to_string()
        })?;
        if Instant::now() >= offer.expires_at {
            return Err("Pairing code expired. Generate a new one on the desktop.".into());
        }
        if offer.failed_attempts >= 5 {
            return Err("Too many pairing attempts. Generate a new code on the desktop.".into());
        }
        if offer.code != code.trim() {
            offer.failed_attempts += 1;
            return Err("Invalid pairing code".into());
        }
        if device_name.len() > 128 {
            return Err("Device name must be at most 128 bytes".into());
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
            permissions: DevicePermissions::default(),
            automation: false,
        };
        let public = PairedDevicePublic {
            id: device.id.clone(),
            name: device.name.clone(),
            created_at: device.created_at.clone(),
            permissions: device.permissions,
        };
        {
            let mut devices = self.devices.write();
            let mut next = devices.clone();
            next.push(device);
            persist_devices(&self.path, &next)?;
            *devices = next;
        }
        *offer_guard = None;
        Ok((token, public))
    }

    /// Creation is exposed only through desktop IPC. The raw token is returned
    /// once; only its SHA-256 digest is persisted alongside paired devices.
    pub fn create_automation_token(&self) -> Result<(String, PairedDevicePublic), String> {
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let device = PairedDevice {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Local automation".into(),
            token_hash: hash_token(&token),
            created_at: chrono::Utc::now().to_rfc3339(),
            permissions: DevicePermissions {
                stream: true,
                history: true,
                injection: true,
            },
            automation: true,
        };
        let public = PairedDevicePublic {
            id: device.id.clone(),
            name: device.name.clone(),
            created_at: device.created_at.clone(),
            permissions: device.permissions,
        };
        let mut devices = self.devices.write();
        let mut next = devices.clone();
        // Rotation invalidates the previous automation token without affecting phones.
        next.retain(|device| !device.automation);
        next.push(device);
        persist_devices(&self.path, &next)?;
        *devices = next;
        Ok((token, public))
    }

    pub fn is_automation_token(&self, token: &str) -> bool {
        let hash = hash_token(token);
        self.devices
            .read()
            .iter()
            .any(|device| device.automation && device.token_hash == hash)
    }

    pub fn automation_device(&self) -> Option<PairedDevicePublic> {
        self.devices
            .read()
            .iter()
            .find(|device| device.automation)
            .map(|device| PairedDevicePublic {
                id: device.id.clone(),
                name: device.name.clone(),
                created_at: device.created_at.clone(),
                permissions: device.permissions,
            })
    }

    pub fn authorize(&self, token: &str) -> bool {
        let hash = hash_token(token);
        self.devices.read().iter().any(|d| d.token_hash == hash)
    }

    pub fn permissions(&self, token: &str) -> Option<DevicePermissions> {
        let hash = hash_token(token);
        self.devices
            .read()
            .iter()
            .find(|d| d.token_hash == hash)
            .map(|d| d.permissions)
    }

    pub fn set_permissions(
        &self,
        id: &str,
        permissions: DevicePermissions,
    ) -> Result<bool, String> {
        let mut devices = self.devices.write();
        let mut next = devices.clone();
        let Some(device) = next.iter_mut().find(|d| d.id == id) else {
            return Ok(false);
        };
        if device.automation {
            return Err("Local automation permissions are fixed; revoke the token instead".into());
        }
        device.permissions = permissions;
        persist_devices(&self.path, &next)?;
        *devices = next;
        Ok(true)
    }

    pub fn list_public(&self) -> Vec<PairedDevicePublic> {
        self.devices
            .read()
            .iter()
            .filter(|device| !device.automation)
            .map(|d| PairedDevicePublic {
                id: d.id.clone(),
                name: d.name.clone(),
                created_at: d.created_at.clone(),
                permissions: d.permissions,
            })
            .collect()
    }

    pub fn revoke(&self, id: &str) -> Result<bool, String> {
        let mut devices = self.devices.write();
        let before = devices.len();
        let mut next = devices.clone();
        next.retain(|d| d.id != id);
        let removed = next.len() != before;
        persist_devices(&self.path, &next)?;
        *devices = next;
        Ok(removed)
    }

    pub fn reset(&self) -> Result<(), String> {
        let mut offer = self.offer.write();
        let mut devices = self.devices.write();
        persist_devices(&self.path, &[])?;
        devices.clear();
        *offer = None;
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
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_accepts_valid_code_and_rejects_wrong() {
        let dir = std::env::temp_dir().join(format!("reflow_pair_{}", uuid::Uuid::new_v4()));
        let state = PairingState::new(dir.join("devices.json"));
        let offer = state.rotate_code();
        assert!(state.pair("invalid", "phone").is_err());
        let (token, device) = state.pair(&offer.code, "Pixel").expect("pair");
        assert!(state.authorize(&token));
        assert!(!state.authorize("nope"));
        assert_eq!(device.name, "Pixel");
        assert!(state.pair(&offer.code, "again").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pairing_locks_after_too_many_guesses() {
        let dir = std::env::temp_dir().join(format!("reflow_pair_limit_{}", uuid::Uuid::new_v4()));
        let state = PairingState::new(dir.join("devices.json"));
        let offer = state.rotate_code();
        for _ in 0..5 {
            assert!(state.pair("invalid", "phone").is_err());
        }
        assert!(state.pair(&offer.code, "phone").is_err());
        let offer = state.rotate_code();
        assert!(state.pair(&offer.code, "phone").is_ok());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_pairing_consumes_code_once() {
        let dir = std::env::temp_dir().join(format!("reflow_pair_race_{}", uuid::Uuid::new_v4()));
        let state = std::sync::Arc::new(PairingState::new(dir.join("devices.json")));
        let offer = state.rotate_code();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let state = state.clone();
                let code = offer.code.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    state.pair(&code, "phone").is_ok()
                })
            })
            .collect();
        let successes = threads
            .into_iter()
            .map(|t| usize::from(t.join().unwrap()))
            .sum::<usize>();
        assert_eq!(successes, 1);
        assert_eq!(state.list_public().len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn token_hash_is_stable() {
        assert_eq!(hash_token("abc"), hash_token("abc"));
        assert_ne!(hash_token("abc"), hash_token("abd"));
    }
}
