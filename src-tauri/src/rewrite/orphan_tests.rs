use super::{command_identifies_managed_server, is_managed_orphan, managed_launcher_generation};
use crate::rewrite::runtime_inventory::{self, RuntimeFile, RuntimeManifest};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

struct Fixture {
    root: PathBuf,
    bin: PathBuf,
    models: PathBuf,
    legacy: PathBuf,
    generation: PathBuf,
    model: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("reflow-orphans-{}", uuid::Uuid::new_v4()));
        let bin = root.join("bin");
        let models = root.join("models");
        std::fs::create_dir_all(&bin).unwrap();
        let legacy = bin.join(runtime_inventory::binary_name());
        std::fs::write(&legacy, b"legacy fixture").unwrap();
        let generation = runtime_inventory::generation_dir(&bin, "b11146-CPU-fixture")
            .unwrap()
            .join(runtime_inventory::binary_name());
        std::fs::create_dir_all(generation.parent().unwrap()).unwrap();
        let bytes = b"generation fixture";
        std::fs::write(&generation, bytes).unwrap();
        runtime_inventory::write_manifest(
            &bin,
            &RuntimeManifest {
                id: "b11146-CPU-fixture".into(),
                version: "b11146".into(),
                kind: "CPU".into(),
                asset: "fixture.zip".into(),
                archive_sha256: "0".repeat(64),
                binary: runtime_inventory::binary_name().into(),
                files: vec![RuntimeFile {
                    filename: runtime_inventory::binary_name().into(),
                    bytes: bytes.len() as u64,
                    sha256: format!("{:x}", Sha256::digest(bytes)),
                }],
            },
        )
        .unwrap();
        let model = models.join("flow").join(super::FLOW_MODELS[1].filename);
        std::fs::create_dir_all(model.parent().unwrap()).unwrap();
        std::fs::write(&model, b"model fixture").unwrap();
        Self {
            root,
            bin,
            models,
            legacy,
            generation,
            model,
        }
    }

    fn command(&self, model: &Path) -> Vec<OsString> {
        vec![
            self.generation.as_os_str().into(),
            "-m".into(),
            model.as_os_str().into(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            "45678".into(),
        ]
    }

    fn selected(&self, executable: Option<&Path>, parent: u32, alive: bool) -> bool {
        is_managed_orphan(
            executable,
            &self.bin,
            &self.models,
            &self.command(&self.model),
            parent,
            100,
            alive,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn orphan_selection_accepts_verified_generations_and_preserves_other_servers() {
    let fixture = Fixture::new();
    assert_eq!(
        managed_launcher_generation(&fixture.generation, &fixture.bin),
        Some(true)
    );
    for launcher in [&fixture.legacy, &fixture.generation] {
        assert!(fixture.selected(Some(launcher), 42, false));
        assert!(!fixture.selected(Some(launcher), 42, true));
        assert!(!fixture.selected(Some(launcher), 100, false));
    }
    assert!(!fixture.selected(None, 42, false));
    let foreign = fixture
        .root
        .join("another-app")
        .join(runtime_inventory::binary_name());
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(&foreign, b"other app").unwrap();
    assert!(!fixture.selected(Some(&foreign), 42, false));
    let unverified = fixture
        .bin
        .join("runtimes/unknown")
        .join(runtime_inventory::binary_name());
    std::fs::create_dir_all(unverified.parent().unwrap()).unwrap();
    std::fs::write(&unverified, b"unverified").unwrap();
    assert!(!fixture.selected(Some(&unverified), 42, false));
    let nested = fixture
        .generation
        .parent()
        .unwrap()
        .join("nested")
        .join(runtime_inventory::binary_name());
    std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
    std::fs::write(&nested, b"ambiguous").unwrap();
    assert!(!fixture.selected(Some(&nested), 42, false));
    std::fs::write(&fixture.generation, b"corrupt generation").unwrap();
    assert!(!fixture.selected(Some(&fixture.generation), 42, false));
}

#[test]
fn reparented_server_requires_a_generation_and_unambiguous_managed_loopback_arguments() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.selected(Some(&fixture.generation), 1, true),
        cfg!(target_os = "linux")
    );
    assert!(!fixture.selected(Some(&fixture.legacy), 1, true));
    assert!(command_identifies_managed_server(
        &fixture.command(&fixture.model),
        &fixture.models
    ));
    let native = &crate::profile::manifest::NATIVE_ASR_MODELS[0];
    let native_model = fixture.models.join(native.dir_name).join(native.filename);
    std::fs::create_dir_all(native_model.parent().unwrap()).unwrap();
    std::fs::write(&native_model, b"native fixture").unwrap();
    assert!(command_identifies_managed_server(
        &fixture.command(&native_model),
        &fixture.models
    ));
    let foreign_model = fixture.root.join(super::FLOW_MODELS[1].filename);
    std::fs::write(&foreign_model, b"foreign model").unwrap();
    assert!(!command_identifies_managed_server(
        &fixture.command(&foreign_model),
        &fixture.models
    ));
    for extra in [
        vec!["--host", "0.0.0.0"],
        vec!["--host=0.0.0.0"],
        vec!["--model", fixture.model.to_str().unwrap()],
        vec!["--port", "1234"],
    ] {
        let mut command = fixture.command(&fixture.model);
        command.extend(extra.iter().map(|argument| OsString::from(*argument)));
        assert!(!command_identifies_managed_server(
            &command,
            &fixture.models
        ));
        assert!(!is_managed_orphan(
            Some(&fixture.generation),
            &fixture.bin,
            &fixture.models,
            &command,
            1,
            100,
            true
        ));
    }
    for (index, replacement) in [(4, "0.0.0.0"), (6, "0"), (6, "65536")] {
        let mut command = fixture.command(&fixture.model);
        command[index] = replacement.into();
        assert!(!command_identifies_managed_server(
            &command,
            &fixture.models
        ));
    }
    let mut command = fixture.command(&fixture.model);
    command.truncate(3);
    assert!(!command_identifies_managed_server(
        &command,
        &fixture.models
    ));
}

#[cfg(unix)]
#[test]
fn generation_symlink_escape_is_never_selected() {
    let fixture = Fixture::new();
    let foreign = fixture.root.join("foreign-runtime");
    std::fs::create_dir_all(&foreign).unwrap();
    std::fs::write(
        foreign.join(runtime_inventory::binary_name()),
        b"foreign launcher",
    )
    .unwrap();
    let alias = fixture.bin.join("runtimes/symlink-generation");
    std::os::unix::fs::symlink(&foreign, &alias).unwrap();
    assert_eq!(
        managed_launcher_generation(&alias.join(runtime_inventory::binary_name()), &fixture.bin),
        None
    );
}
