//! Build-time pinned, model-free speech packs. The archive contains binary
//! wheels, never a relocated environment. Activation stays in python_setup.
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize)]
pub(super) struct RuntimePackPin {
    pub target: String,
    pub profile: String,
    pub variant: String,
    pub asset: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub unpacked_bytes: u64,
    requirements_sha256: String,
    pip_lock_sha256: String,
    pbs_sha256: String,
    source_commit: String,
}

#[derive(Deserialize)]
struct Catalog {
    schema: u32,
    release: Option<String>,
    source_commit: Option<String>,
    packs: Vec<RuntimePackPin>,
}

#[derive(Deserialize)]
struct FilePin {
    path: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize)]
struct Manifest {
    schema: u32,
    target: String,
    profile: String,
    variant: String,
    requirements_sha256: String,
    pip_lock_sha256: String,
    pbs_sha256: String,
    files: Vec<FilePin>,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_catalog(text: &str) -> Result<Catalog, String> {
    let catalog: Catalog =
        serde_json::from_str(text).map_err(|e| format!("Invalid runtime pack catalog: {e}"))?;
    if catalog.schema != 1 {
        return Err("Unsupported runtime pack catalog; update Reflow.".into());
    }
    if catalog.packs.is_empty() && cfg!(debug_assertions) {
        return Ok(catalog);
    }
    let release = format!("v{}", env!("CARGO_PKG_VERSION"));
    let commit = catalog.source_commit.as_deref().unwrap_or_default();
    if catalog.release.as_deref() != Some(&release)
        || commit.len() != 40
        || !commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("This build has no matching speech runtime pack catalog. Install a current Reflow release.".into());
    }
    let mut keys = HashSet::new();
    for pin in &catalog.packs {
        let python =
            super::python_setup::pin_for(&pin.target).ok_or("Unknown runtime pack target")?;
        let requirements =
            super::dependency_locks::locked_requirements(&pin.target, &pin.profile, &pin.variant)
                .ok_or("Unknown runtime pack profile")?;
        let asset = format!(
            "speech-runtime-{}-{}-{}.tar.gz",
            pin.target, pin.profile, pin.variant
        );
        let url = reqwest::Url::parse(&pin.url).map_err(|e| e.to_string())?;
        crate::network_policy::validate_download_url(&url)?;
        if pin.asset != asset
            || pin.source_commit != commit
            || url.host_str() != Some("github.com")
            || !url
                .path()
                .ends_with(&format!("/releases/download/{release}/{asset}"))
            || url.query().is_some()
            || url.fragment().is_some()
            || !valid_digest(&pin.sha256)
            || pin.bytes == 0
            || pin.bytes > 16 * 1024 * 1024 * 1024
            || pin.unpacked_bytes == 0
            || pin.unpacked_bytes > 24 * 1024 * 1024 * 1024
            || pin.requirements_sha256 != digest(requirements.as_bytes())
            || pin.pip_lock_sha256 != digest(include_bytes!("../../../model-runtime/locks/pip.txt"))
            || pin.pbs_sha256 != python.sha256
            || !keys.insert((&pin.target, &pin.profile, &pin.variant))
        {
            return Err(format!(
                "Runtime pack differs from this build's reviewed pins: {asset}"
            ));
        }
    }
    Ok(catalog)
}

pub(super) fn selected(
    target: &str,
    profile: &str,
    variant: &str,
) -> Result<Option<RuntimePackPin>, String> {
    selected_from(
        include_str!("../../../model-runtime/runtime-packs.json"),
        target,
        profile,
        variant,
    )
}

pub(super) fn selected_from(
    text: &str,
    target: &str,
    profile: &str,
    variant: &str,
) -> Result<Option<RuntimePackPin>, String> {
    let catalog = parse_catalog(text)?;
    if catalog.packs.is_empty() && cfg!(debug_assertions) {
        return Ok(None);
    }
    catalog.packs.into_iter().find(|pin| pin.target == target && pin.profile == profile && pin.variant == variant)
        .map(Some).ok_or_else(|| "This speech runtime pack is unavailable in this build. Choose CPU or Zipformer, or install a current Reflow release.".into())
}

struct Inputs<'a> {
    bootstrap_asset: &'a str,
    requirements: &'a [u8],
    pip: &'a [u8],
}

fn safe_payload_path(value: &str) -> bool {
    let parts: Vec<_> = value.split('/').collect();
    if parts.iter().any(|part| {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        part.is_empty()
            || *part == "."
            || *part == ".."
            || part.len() > 240
            || [
                "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
                "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8",
                "lpt9",
            ]
            .contains(&stem.as_str())
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    }) {
        return false;
    }
    match parts.as_slice() {
        ["requirements.txt" | "pip-requirements.txt" | "licenses.json" | "build-check.json"] => {
            true
        }
        ["bootstrap", name] => name.ends_with(".tar.gz"),
        ["wheelhouse", name] => name.ends_with(".whl"),
        _ => false,
    }
}

/// The caller owns the pinned, hash-verified archive and a fresh generation.
/// Every extracted payload has its own bound, digest and cancellation checks.
pub(super) fn unpack(
    pin: &RuntimePackPin,
    archive: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let python = super::python_setup::pin_for(&pin.target).ok_or("Unknown runtime pack target")?;
    let requirements =
        super::dependency_locks::locked_requirements(&pin.target, &pin.profile, &pin.variant)
            .ok_or("Unknown runtime pack profile")?;
    unpack_inner(
        pin,
        archive,
        destination,
        cancel,
        Inputs {
            bootstrap_asset: python.asset,
            requirements: requirements.as_bytes(),
            pip: include_bytes!("../../../model-runtime/locks/pip.txt"),
        },
    )
}

fn unpack_inner(
    pin: &RuntimePackPin,
    archive: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    inputs: Inputs<'_>,
) -> Result<(), String> {
    let check = || {
        if cancel.load(Ordering::Acquire) {
            Err("python runtime setup cancelled".to_string())
        } else {
            Ok(())
        }
    };
    check()?;
    let source = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(source));
    let mut entries = tar.entries().map_err(|e| e.to_string())?;
    let mut entry = entries
        .next()
        .ok_or("Runtime pack is empty")?
        .map_err(|e| e.to_string())?;
    if entry.path().map_err(|e| e.to_string())?.as_ref() != Path::new("manifest.json")
        || !entry.header().entry_type().is_file()
        || entry.size() > 2 * 1024 * 1024
    {
        return Err("Runtime pack must begin with a bounded manifest".into());
    }
    let mut manifest_bytes = Vec::new();
    entry
        .read_to_end(&mut manifest_bytes)
        .map_err(|e| e.to_string())?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
    drop(entry);
    if manifest.schema != 1
        || manifest.target != pin.target
        || manifest.profile != pin.profile
        || manifest.variant != pin.variant
        || manifest.requirements_sha256 != pin.requirements_sha256
        || manifest.pip_lock_sha256 != pin.pip_lock_sha256
        || manifest.pbs_sha256 != pin.pbs_sha256
        || manifest.files.is_empty()
        || manifest.files.len() > 512
    {
        return Err("Runtime pack manifest does not match the selected dependency set".into());
    }
    let mut files = HashMap::new();
    let mut declared = manifest_bytes.len() as u64;
    for file in manifest.files {
        declared = declared
            .checked_add(file.bytes)
            .ok_or("Runtime pack size overflow")?;
        if !safe_payload_path(&file.path)
            || !valid_digest(&file.sha256)
            || file.bytes == 0
            || declared > pin.unpacked_bytes
            || files.insert(file.path.clone(), file).is_some()
        {
            return Err("Runtime pack has invalid, duplicate or oversized payloads".into());
        }
    }
    let bootstrap = format!("bootstrap/{}", inputs.bootstrap_asset);
    for (path, expected) in [
        ("requirements.txt", &pin.requirements_sha256),
        ("pip-requirements.txt", &pin.pip_lock_sha256),
        (bootstrap.as_str(), &pin.pbs_sha256),
    ] {
        if !files.get(path).is_some_and(|file| &file.sha256 == expected) {
            return Err(format!(
                "Runtime pack is missing a reviewed payload: {path}"
            ));
        }
    }
    if declared != pin.unpacked_bytes
        || !files.contains_key("licenses.json")
        || !files.contains_key("build-check.json")
        || !files.keys().any(|path| path.starts_with("wheelhouse/"))
    {
        return Err("Runtime pack has incomplete size, license or build evidence".into());
    }
    for entry in entries {
        check()?;
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry
            .path()
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        let file = files
            .remove(&path)
            .ok_or("Runtime pack contains an undeclared or duplicate payload")?;
        if !entry.header().entry_type().is_file() || entry.size() != file.bytes {
            return Err("Runtime pack contains a link or mismatched payload size".into());
        }
        let path = destination.join(&file.path);
        std::fs::create_dir_all(path.parent().ok_or("Missing payload directory")?)
            .map_err(|e| e.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = options.open(path).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            check()?;
            let count = entry.read(&mut buffer).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
        }
        if hex::encode(hash.finalize()) != file.sha256 {
            return Err(format!("Runtime pack checksum failed: {}", file.path));
        }
        output.sync_all().map_err(|e| e.to_string())?;
    }
    if !files.is_empty() {
        return Err("Runtime pack ended before all payloads were extracted".into());
    }
    for (path, expected) in [
        ("requirements.txt", inputs.requirements),
        ("pip-requirements.txt", inputs.pip),
    ] {
        if std::fs::read(destination.join(path)).map_err(|e| e.to_string())? != expected {
            return Err("Runtime pack requirements do not match this application build".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    struct Fixture {
        root: std::path::PathBuf,
        pin: RuntimePackPin,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    fn fixture(change: impl FnOnce(&mut Value), fault: &str) -> Fixture {
        let root = std::env::temp_dir().join(format!("reflow-pack-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let payloads: Vec<(&str, &[u8])> = vec![
            ("requirements.txt", b"package==1\n"),
            ("pip-requirements.txt", b"pip==1\n"),
            ("bootstrap/python.tar.gz", b"python"),
            ("wheelhouse/package.whl", b"wheel"),
            ("licenses.json", b"{}"),
            ("build-check.json", b"{}"),
        ];
        let mut manifest = json!({"schema":1,"target":"fixture","profile":"qwen","variant":"cpu",
            "requirements_sha256":digest(payloads[0].1), "pip_lock_sha256":digest(payloads[1].1),
            "pbs_sha256":digest(payloads[2].1), "files":payloads.iter().map(|(path,bytes)|
                json!({"path":path,"bytes":bytes.len(),"sha256":digest(bytes)})).collect::<Vec<_>>()});
        change(&mut manifest);
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();
        let mut pin = RuntimePackPin {
            target: "fixture".into(),
            profile: "qwen".into(),
            variant: "cpu".into(),
            asset: "pack.tar.gz".into(),
            url: "https://github.com/example/pack.tar.gz".into(),
            sha256: String::new(),
            bytes: 0,
            unpacked_bytes: manifest_bytes.len() as u64
                + payloads.iter().map(|(_, b)| b.len() as u64).sum::<u64>(),
            requirements_sha256: digest(payloads[0].1),
            pip_lock_sha256: digest(payloads[1].1),
            pbs_sha256: digest(payloads[2].1),
            source_commit: "a".repeat(40),
        };
        let archive = std::fs::File::create(root.join("pack.tar.gz")).unwrap();
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            archive,
            flate2::Compression::fast(),
        ));
        let mut append = |path: &str, bytes: &[u8], link: bool| {
            let mut header = tar::Header::new_gnu();
            header.set_size(if link { 0 } else { bytes.len() as u64 });
            header.set_mode(0o600);
            if link {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_link_name("/outside").unwrap();
            }
            header.set_cksum();
            tar.append_data(&mut header, path, if link { &[][..] } else { bytes })
                .unwrap();
        };
        append("manifest.json", &manifest_bytes, false);
        for (path, bytes) in &payloads {
            if fault == "missing" && *path == "licenses.json" {
                continue;
            }
            append(
                path,
                if fault == "corrupt" && *path == "wheelhouse/package.whl" {
                    b"wrong"
                } else {
                    bytes
                },
                fault == "link" && *path == "wheelhouse/package.whl",
            );
        }
        if fault == "duplicate" {
            append(payloads[0].0, payloads[0].1, false);
        }
        if fault == "extra" {
            append("undeclared", b"extra", false);
        }
        tar.into_inner().unwrap().finish().unwrap();
        let archive = root.join("pack.tar.gz");
        pin.bytes = std::fs::metadata(&archive).unwrap().len();
        pin.sha256 = digest(&std::fs::read(archive).unwrap());
        Fixture { root, pin }
    }
    fn extract(f: &Fixture, cancel: bool) -> Result<(), String> {
        unpack_inner(
            &f.pin,
            &f.root.join("pack.tar.gz"),
            &f.root.join("payload"),
            &AtomicBool::new(cancel),
            Inputs {
                bootstrap_asset: "python.tar.gz",
                requirements: b"package==1\n",
                pip: b"pip==1\n",
            },
        )
    }

    #[test]
    fn valid_payloads_extract_and_cancel_leaves_no_partial_generation() {
        let f = fixture(|_| {}, "");
        assert!(extract(&f, true).unwrap_err().contains("cancelled"));
        assert!(!f.root.join("payload").exists());
        extract(&f, false).unwrap();
        assert_eq!(
            std::fs::read(f.root.join("payload/wheelhouse/package.whl")).unwrap(),
            b"wheel"
        );
        // A caller must use a fresh generation; incumbent bytes are never replaced.
        assert!(extract(&f, false).is_err());
        assert_eq!(
            std::fs::read(f.root.join("payload/wheelhouse/package.whl")).unwrap(),
            b"wheel"
        );
    }

    #[test]
    fn corrupt_missing_extra_duplicate_and_link_payloads_are_rejected() {
        for fault in ["corrupt", "missing", "extra", "duplicate", "link"] {
            let f = fixture(|_| {}, fault);
            assert!(extract(&f, false).is_err(), "{fault}");
        }
    }

    #[test]
    fn manifest_identity_paths_sizes_and_pin_mismatches_are_rejected() {
        for path in [
            "../escape",
            "/escape",
            "C:/escape",
            "wheelhouse/../escape.whl",
            "wheelhouse\\escape.whl",
            "wheelhouse/NUL:escape.whl",
            "wheelhouse/NUL.whl",
            "bootstrap/CON.tar.gz",
        ] {
            let f = fixture(|m| m["files"][0]["path"] = json!(path), "");
            assert!(extract(&f, false).is_err(), "{path}");
        }
        for field in [
            "target",
            "profile",
            "variant",
            "requirements_sha256",
            "pip_lock_sha256",
            "pbs_sha256",
        ] {
            let f = fixture(|m| m[field] = json!("wrong"), "");
            assert!(extract(&f, false).is_err(), "{field}");
        }
        let f = fixture(|m| m["files"][0]["bytes"] = json!(u64::MAX), "");
        assert!(extract(&f, false).is_err());
        let f = fixture(
            |m| {
                let copy = m["files"][0].clone();
                m["files"].as_array_mut().unwrap().push(copy);
            },
            "",
        );
        assert!(extract(&f, false).is_err());
    }

    #[test]
    fn catalog_requires_the_current_version_source_and_compiled_dependency_hashes() {
        let target = "x86_64-unknown-linux-gnu";
        let python = super::super::python_setup::pin_for(target).unwrap();
        let asset = format!("speech-runtime-{target}-zipformer-none.tar.gz");
        let release = format!("v{}", env!("CARGO_PKG_VERSION"));
        let mut catalog = json!({"schema":1,"release":release,"source_commit":"a".repeat(40),"packs":[{
            "target":target,"profile":"zipformer","variant":"none","asset":asset,
            "url":format!("https://github.com/example/reflow/releases/download/{release}/{asset}"),
            "sha256":"b".repeat(64),"bytes":100,"unpacked_bytes":100,
            "requirements_sha256":digest(super::super::dependency_locks::locked_requirements(target,"zipformer","none").unwrap().as_bytes()),
            "pip_lock_sha256":digest(include_bytes!("../../../model-runtime/locks/pip.txt")),
            "pbs_sha256":python.sha256,"source_commit":"a".repeat(40)
        }]});
        parse_catalog(&catalog.to_string()).unwrap();
        for field in [
            "source_commit",
            "requirements_sha256",
            "pip_lock_sha256",
            "pbs_sha256",
            "url",
            "asset",
        ] {
            let original = catalog["packs"][0][field].clone();
            catalog["packs"][0][field] = json!("wrong");
            assert!(parse_catalog(&catalog.to_string()).is_err(), "{field}");
            catalog["packs"][0][field] = original;
        }
        catalog["release"] = json!("v0.0.0");
        assert!(parse_catalog(&catalog.to_string()).is_err());
    }
}
