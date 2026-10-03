//! Model manifests and the pure hardware-to-configuration resolver.
//!
//! `manifest` is declarative data; `resolver` is a pure function over that data
//! plus a [`crate::capability::Capabilities`] snapshot. Neither performs I/O, so
//! every hardware class in the validation matrix is reproducible from a fixture.

pub mod manifest;
pub mod measurements;
pub mod resolver;

pub use manifest::{
    all_manifests, asr_manifest, refinement_manifest, Device, MeasuredPeaks, ModelManifest,
    Precision, RuntimeKind, ASR_MODELS, REFINEMENT_MODELS,
};
pub use resolver::{
    build_attempts, no_measurements, resolve_profile, select_asr_load, select_asr_load_for_runtime,
    vram_reserve_mb, AsrSelection, LoadAttempt, Preset, ProfileOverrides, ProfileReason,
    ResolvedProfile, STREAMING_RTF_CEILING,
};
