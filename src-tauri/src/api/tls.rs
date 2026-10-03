//! The desktop identity is trusted through the link/fingerprint copied from its
//! local UI, never downloaded over an unauthenticated LAN bootstrap connection.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Serialize, Deserialize)]
pub struct LanIdentity {
    pub certificate_pem: String,
    private_key_pem: String,
}

impl LanIdentity {
    pub fn fingerprint(&self) -> Result<String, String> {
        let content: String = self
            .certificate_pem
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        let der = STANDARD
            .decode(content)
            .map_err(|_| "Invalid desktop certificate.".to_string())?;
        Ok(hex::encode(Sha256::digest(der)))
    }

    pub async fn config(&self) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        axum_server::tls_rustls::RustlsConfig::from_pem(self.certificate_pem.as_bytes().to_vec(), self.private_key_pem.as_bytes().to_vec()).await.map_err(|_| "Desktop TLS identity could not be loaded. Preserve the identity file and repair phone pairing.".into())
    }
}

pub fn load_or_create(path: &Path) -> Result<LanIdentity, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let identity: LanIdentity = serde_json::from_slice(&bytes).map_err(|_| "Desktop identity is unreadable. Its file was preserved; repair it before enabling phone access.".to_string())?;
            identity.fingerprint()?;
            return Ok(identity);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(
                "Could not read desktop identity. Check access to the app configuration directory."
                    .into(),
            )
        }
    }
    let mut names = super::net::lan_ipv4_addrs();
    names.extend([
        "localhost".to_string(),
        "127.0.0.1".to_string(),
        "::1".to_string(),
    ]);
    let rcgen::CertifiedKey { cert, key_pair } = rcgen::generate_simple_self_signed(names)
        .map_err(|_| "Could not create desktop TLS identity.".to_string())?;
    let identity = LanIdentity {
        certificate_pem: cert.pem(),
        private_key_pem: key_pair.serialize_pem(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = PathBuf::from(format!(
        "{}.{}.partial",
        path.display(),
        uuid::Uuid::new_v4()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write;
    let mut file = options
        .open(&temporary)
        .map_err(|_| "Could not save desktop identity.".to_string())?;
    file.write_all(&serde_json::to_vec(&identity).map_err(|e| e.to_string())?)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not save desktop identity.".to_string())?;
    drop(file);
    std::fs::rename(&temporary, path)
        .map_err(|_| "Could not activate desktop identity.".to_string())?;
    Ok(identity)
}

pub fn identity(ctx: &crate::context::AppContext) -> Result<LanIdentity, String> {
    ctx.api_identity
        .get_or_init(|| load_or_create(&ctx.api_identity_path))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn identity_is_persistent_and_unreadable_identity_is_never_replaced() {
        let dir = std::env::temp_dir().join(format!("reflow_tls_{}", uuid::Uuid::new_v4()));
        let path = dir.join("identity.json");
        let identity = load_or_create(&path).unwrap();
        assert_eq!(identity.fingerprint().unwrap().len(), 64);
        assert_eq!(
            identity.fingerprint().unwrap(),
            load_or_create(&path).unwrap().fingerprint().unwrap()
        );
        identity.config().await.unwrap();
        std::fs::write(&path, b"preserve corrupt identity").unwrap();
        assert!(load_or_create(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"preserve corrupt identity");
    }
}
