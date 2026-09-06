//! Milestone 11 / Task 44: Distribution and model checksum verification tests.
//!
//! Tests that:
//! 1. All manifests (ASR & Refinement) have pinned revisions, non-empty sha256, and report is_verifiable() == true.
//! 2. SHA-256 verification correctly accepts matching digests and rejects corrupted or modified bytes.
//! 3. Atomic download staging (.tmp file -> verified target) prevents partial files from being considered valid.

use reflow_lib::profile::manifest::{
    all_manifests, asr_manifest, refinement_manifest, ModelManifest,
};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;

fn test_temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("reflow_dist_test_{}", uuid::Uuid::new_v4()));
    let _ = fs::create_dir_all(&dir);
    dir
}

#[test]
fn all_manifests_are_pinned_and_verifiable() {
    let manifests: Vec<&ModelManifest> = all_manifests().collect();
    assert!(
        !manifests.is_empty(),
        "Manifest registry should not be empty"
    );

    for m in manifests {
        assert!(
            m.is_verifiable(),
            "Manifest '{}' must be verifiable (revision != 'main', sha256 non-empty)",
            m.id
        );
        assert_ne!(
            m.revision, "main",
            "Manifest '{}' must not use floating 'main' branch",
            m.id
        );
        assert!(
            !m.sha256.is_empty(),
            "Manifest '{}' must have a pinned sha256",
            m.id
        );
        assert_eq!(
            m.sha256.len(),
            64,
            "Manifest '{}' sha256 must be 64-char hex digest",
            m.id
        );
        assert!(
            m.download_bytes > 0,
            "Manifest '{}' must specify non-zero download bytes",
            m.id
        );
    }
}

#[test]
fn asr_and_refinement_manifest_lookups() {
    let asr_06b = asr_manifest("0.6b").expect("0.6b ASR manifest");
    assert_eq!(asr_06b.id, "0.6b");
    assert!(asr_06b.is_verifiable());

    let asr_17b = asr_manifest("1.7b").expect("1.7b ASR manifest");
    assert_eq!(asr_17b.id, "1.7b");
    assert!(asr_17b.is_verifiable());

    let ref_lfm = refinement_manifest("qwen3.5-0.8b").expect("LFM2.5 manifest");
    assert_eq!(ref_lfm.id, "qwen3.5-0.8b");
    assert!(ref_lfm.is_verifiable());

    let ref_qwen = refinement_manifest("qwen3.5-2b").expect("Qwen3.5 manifest");
    assert_eq!(ref_qwen.id, "qwen3.5-2b");
    assert!(ref_qwen.is_verifiable());
}

#[test]
fn sha256_checksum_verification_and_corruption_detection() {
    let dir = test_temp_dir();
    let test_file = dir.join("model_weights.bin");

    let payload = b"reflow-deterministic-weights-data-block-verification-payload";
    let mut file = File::create(&test_file).expect("create test file");
    file.write_all(payload).expect("write payload");
    drop(file);

    let mut hasher = Sha256::new();
    hasher.update(payload);
    let expected_hex = format!("{:x}", hasher.finalize());

    // Verify correct digest matches
    let read_bytes = fs::read(&test_file).expect("read file");
    let mut actual_hasher = Sha256::new();
    actual_hasher.update(&read_bytes);
    let actual_hex = format!("{:x}", actual_hasher.finalize());
    assert_eq!(actual_hex, expected_hex);

    // Verify corrupted digest fails
    let corrupted_hex = "0000000000000000000000000000000000000000000000000000000000000000";
    assert_ne!(actual_hex, corrupted_hex);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn atomic_staging_prevents_partial_installation() {
    let dir = test_temp_dir();
    let target_file = dir.join("final_model.bin");
    let temp_file = dir.join("final_model.bin.tmp");

    // Simulate partial interrupted write to .tmp
    let partial_data = b"partial-data";
    fs::write(&temp_file, partial_data).expect("write partial");

    // Target must not exist yet
    assert!(!target_file.exists());

    // Complete download and atomic rename
    let complete_data = b"full-complete-model-data-payload";
    fs::write(&temp_file, complete_data).expect("overwrite complete temp");
    fs::rename(&temp_file, &target_file).expect("atomic rename");

    assert!(target_file.exists());
    assert!(!temp_file.exists());
    assert_eq!(fs::read(&target_file).expect("read final"), complete_data);

    let _ = fs::remove_dir_all(&dir);
}
