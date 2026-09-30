//! Milestone 1 / Task 8: declarative manifests for everything installable.
//!
//! Every model and runtime is described in one place, so the profile resolver
//! can be a pure function over data rather than a pile of conditionals that
//! each know a different subset of the facts.
//!
//! Two kinds of resource figure are recorded and deliberately kept apart:
//!
//! * **Estimated** peaks, derived from weight size and architecture. These seed
//!   the resolver before anything has been measured on this machine.
//! * **Measured** peaks, written back by the benchmark harness. Once present
//!   they override the estimates, because an estimate that disagrees with a
//!   measurement on the actual hardware is simply wrong.
//!
//! Nothing here performs I/O.

use serde::{Deserialize, Serialize};

/// Numeric precision a model can be loaded at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// Full precision on CPU (fp32) — the CPU path never quantizes, because
    /// quantized weights on CPU cost RAM without buying speed.
    Fp32,
    Bf16,
    Int8,
    Int4,
}

impl Precision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Precision::Fp32 => "fp32",
            Precision::Bf16 => "bf16",
            Precision::Int8 => "int8",
            Precision::Int4 => "int4",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "fp32" | "float32" => Some(Precision::Fp32),
            "bf16" | "bfloat16" => Some(Precision::Bf16),
            "int8" => Some(Precision::Int8),
            "int4" => Some(Precision::Int4),
            _ => None,
        }
    }

    /// Bytes per weight, used for the VRAM estimate.
    pub fn bytes_per_weight(&self) -> f32 {
        match self {
            Precision::Fp32 => 4.0,
            Precision::Bf16 => 2.0,
            // Weight-only quantization keeps some tensors at higher precision,
            // so the effective ratio is above the nominal 1 and 0.5 bytes.
            Precision::Int8 => 1.1,
            Precision::Int4 => 0.65,
        }
    }
}

/// Where a model can run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    Cpu,
    Cuda,
    Vulkan,
}

impl Device {
    pub fn as_str(&self) -> &'static str {
        match self {
            Device::Cpu => "cpu",
            Device::Cuda => "cuda",
            Device::Vulkan => "vulkan",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(Device::Cpu),
            "cuda" | "gpu" => Some(Device::Cuda),
            "vulkan" => Some(Device::Vulkan),
            _ => None,
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, Device::Cuda | Device::Vulkan)
    }
}

/// Which execution stack loads a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    /// The Python/Transformers ASR sidecar.
    PythonAsr,
    /// `llama-server` from llama.cpp, used for refinement.
    LlamaServer,
    /// A native, Python-free ASR backend. Added in Milestone 8.
    NativeAsr,
}

/// Measured resource peaks for one (device, precision) pair.
///
/// `None` means "not measured on this machine yet" — which is not the same as
/// zero, and must never be treated as such.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct MeasuredPeaks {
    pub ram_mb: Option<f32>,
    pub vram_mb: Option<f32>,
    /// Seconds from load request to ready.
    pub load_seconds: Option<f32>,
    /// Warm inference real-time factor.
    pub rtf: Option<f32>,
    /// Word error rate against the reference corpus, 0.0-1.0.
    pub wer: Option<f32>,
}

impl MeasuredPeaks {
    pub fn is_empty(&self) -> bool {
        self.ram_mb.is_none()
            && self.vram_mb.is_none()
            && self.load_seconds.is_none()
            && self.rtf.is_none()
            && self.wer.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeightFile {
    pub filename: &'static str,
    pub sha256: &'static str,
}

/// A downloadable, loadable artifact.
///
/// `Serialize` only: these are compile-time constants sent to the UI, never
/// read back from anywhere.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelManifest {
    /// Stable id used in settings and caches.
    pub id: &'static str,
    pub label: &'static str,
    pub runtime: RuntimeKind,
    /// Hugging Face repo.
    pub repo: &'static str,
    /// Pinned revision. `main` is a moving target and would silently change the
    /// model under a cached benchmark, so a real release pins a commit.
    pub revision: &'static str,
    /// Single file to fetch, or empty for a whole-snapshot download.
    pub filename: &'static str,
    /// Local directory name under the models root.
    pub dir_name: &'static str,
    /// SHA-256 of `filename`. Empty until the pinned revision is recorded;
    /// [`ModelManifest::is_verifiable`] reports which state a manifest is in.
    pub sha256: &'static str,
    pub auxiliary_files: &'static [WeightFile],
    /// On-disk size, for the download UI and the free-space check.
    pub download_bytes: u64,
    /// Parameter count, used for the resource estimate.
    pub params: u64,
    /// Precisions this model can actually be loaded at, best first.
    pub precisions: &'static [Precision],
    /// Devices this model can run on.
    pub devices: &'static [Device],
    /// BCP-47-ish language codes the model is *validated* for.
    ///
    /// Deliberately not "every language the upstream model card lists": a
    /// language belongs here only once it has passed its own corpus, so the app
    /// can never claim support it has not tested.
    pub languages: &'static [&'static str],
    /// Fixed runtime overhead beyond the weights: activations, KV cache,
    /// CUDA context, allocator slack.
    pub overhead_mb: f32,
}

impl ModelManifest {
    /// Runtime overhead for a load at `precision`, in MiB.
    ///
    /// `overhead_mb` describes a full-precision load. A weight-only quantized
    /// load carries measurably less: the workspace and cached buffers this
    /// runtime allocates scale with the weight dtype, not just the weights
    /// themselves. Two anchors, both from a 4 GiB RTX 2050:
    ///
    /// * 0.6B BF16 sits at ~2.2 GB resident — 1144 MB of weights, so ~900 MB of
    ///   overhead, which is what `overhead_mb` records.
    /// * 0.6B INT8 was measured at 1266 MB self-reported by the sidecar
    ///   (1362 MiB by `nvidia-smi`) against 629 MB of int8 weights, so ~640 MB
    ///   of overhead — about 0.72 of the BF16 figure.
    ///
    /// Treating the two as equal overstated the quantized estimate by ~250 MB,
    /// which was enough for the resolver to refuse a configuration that had been
    /// measured working and drop ASR to the CPU, costing 17546 ms per dictation
    /// instead of 2887 ms.
    fn overhead_for(&self, precision: Precision) -> f32 {
        match precision {
            Precision::Int8 | Precision::Int4 => self.overhead_mb * 0.72,
            Precision::Bf16 | Precision::Fp32 => self.overhead_mb,
        }
    }

    /// Estimated VRAM peak for a GPU load at `precision`, in MiB.
    pub fn estimated_vram_mb(&self, precision: Precision) -> f32 {
        let weights_mb = (self.params as f32 * precision.bytes_per_weight()) / (1024.0 * 1024.0);
        weights_mb + self.overhead_for(precision)
    }

    /// Estimated system-RAM peak for a CPU load, in MiB.
    ///
    /// Higher than the VRAM figure for the same weights: the CPU path runs
    /// fp32 and the allocator is less tightly managed.
    pub fn estimated_cpu_ram_mb(&self) -> f32 {
        let weights_mb =
            (self.params as f32 * Precision::Fp32.bytes_per_weight()) / (1024.0 * 1024.0);
        weights_mb + self.overhead_mb * 1.2
    }

    pub fn supports(&self, device: Device, precision: Precision) -> bool {
        self.devices.contains(&device) && self.precisions.contains(&precision)
    }

    pub fn supports_language(&self, language: &str) -> bool {
        let want = language.trim().to_ascii_lowercase();
        if want.is_empty() || want == "auto" {
            return true;
        }
        // Match on the primary subtag so "en-GB" satisfies "en".
        let primary = want.split(['-', '_']).next().unwrap_or(&want);
        self.languages
            .iter()
            .any(|l| l.eq_ignore_ascii_case(primary) || l.eq_ignore_ascii_case(&want))
    }

    /// `true` when the download can be checksum-verified.
    ///
    /// Reported rather than assumed so Task 44 can require it before release
    /// instead of silently accepting whatever bytes arrive.
    pub fn is_verifiable(&self) -> bool {
        self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            && self.revision.len() == 40
            && self.revision.bytes().all(|b| b.is_ascii_hexdigit())
            && self
                .auxiliary_files
                .iter()
                .all(|f| f.sha256.len() == 64 && f.sha256.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}

/// The Python ASR sidecar's models.
///
/// Language lists cover only what has been validated. Qwen3-ASR's model card
/// advertises far more; those are added by Task 42 as each passes its corpus.
pub const ASR_MODELS: &[ModelManifest] = &[
    ModelManifest {
        id: "0.6b",
        label: "Qwen3-ASR 0.6B",
        runtime: RuntimeKind::PythonAsr,
        repo: "Qwen/Qwen3-ASR-0.6B-hf",
        revision: "7f1569a48a89f3e3f4dc3a5c9d28bddd903bc76c",
        filename: "model.safetensors",
        dir_name: "qwen3-asr-0.6b",
        sha256: "d3f212dd20abecd315d830bc54ae3865e56ebfc3276484e57b771288ba27fd35",
        auxiliary_files: &[],
        download_bytes: 1_564_928_088,
        params: 600_000_000,
        // INT8 is offered but is not expected to help at batch size 1: the
        // dequantization overhead outweighs the bandwidth saving when BF16
        // already fits. Task 17 settles that with numbers.
        precisions: &[Precision::Bf16, Precision::Int8, Precision::Fp32],
        devices: &[Device::Cuda, Device::Cpu],
        languages: &["en"],
        overhead_mb: 900.0,
    },
    ModelManifest {
        id: "1.7b",
        label: "Qwen3-ASR 1.7B",
        runtime: RuntimeKind::PythonAsr,
        repo: "Qwen/Qwen3-ASR-1.7B-hf",
        revision: "bcd2b5b7f32b480ab5790554cfa8347f246a14f3",
        filename: "model.safetensors",
        dir_name: "qwen3-asr-1.7b",
        sha256: "2db53c7d81bd9b8cbc6a074e89be2c968a0d373fb4ee68bb1b1e14f7042dfee1",
        auxiliary_files: &[],
        download_bytes: 4_076_193_080,
        params: 1_700_000_000,
        precisions: &[
            Precision::Int8,
            Precision::Bf16,
            Precision::Int4,
            Precision::Fp32,
        ],
        devices: &[Device::Cuda, Device::Cpu],
        languages: &["en"],
        overhead_mb: 1100.0,
    },
];

/// Refinement models served by `llama-server`.
pub const REFINEMENT_MODELS: &[ModelManifest] = &[
    ModelManifest {
        id: "qwen3.5-0.8b",
        label: "Qwen3.5 0.8B",
        runtime: RuntimeKind::LlamaServer,
        repo: "unsloth/Qwen3.5-0.8B-GGUF",
        revision: "6ab461498e2023f6e3c1baea90a8f0fe38ab64d0",
        filename: "Qwen3.5-0.8B-Q4_K_M.gguf",
        dir_name: "flow",
        // The digest of the pinned file, verified against the Hugging Face API.
        sha256: "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517",
        auxiliary_files: &[],
        download_bytes: 532_517_120,
        params: 800_000_000,
        // A GGUF is already quantized; the precision here describes the file.
        precisions: &[Precision::Int4],
        devices: &[Device::Cpu, Device::Cuda, Device::Vulkan],
        languages: &["en"],
        // At --ctx-size 1024 the KV cache is small; this covers it plus the
        // llama.cpp compute buffers.
        overhead_mb: 260.0,
    },
    ModelManifest {
        id: "qwen3.5-2b",
        label: "Qwen3.5 2B",
        runtime: RuntimeKind::LlamaServer,
        // `Qwen/Qwen3.5-2B-GGUF` does not exist; this is the real repo, and its
        // files carry no `-Instruct` infix.
        repo: "unsloth/Qwen3.5-2B-GGUF",
        revision: "f6d5376be1edb4d416d56da11e5397a961aca8ae",
        filename: "Qwen3.5-2B-Q4_K_M.gguf",
        dir_name: "flow",
        sha256: "aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223",
        auxiliary_files: &[],
        download_bytes: 1_280_835_840,
        params: 2_000_000_000,
        precisions: &[Precision::Int4],
        devices: &[Device::Cpu, Device::Cuda, Device::Vulkan],
        languages: &["en"],
        overhead_mb: 320.0,
    },
];

pub const NATIVE_ASR_MODELS: &[ModelManifest] = &[
    ModelManifest {
        id: "native-0.6b",
        label: "Qwen3-ASR 0.6B (native)",
        runtime: RuntimeKind::NativeAsr,
        repo: "ggml-org/Qwen3-ASR-0.6B-GGUF",
        revision: "928ab958557df9aa2ef1c93e0e83c7ad0933fae2",
        filename: "Qwen3-ASR-0.6B-Q8_0.gguf",
        dir_name: "qwen3-asr-0.6b-native",
        sha256: "bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971",
        auxiliary_files: &[WeightFile {
            filename: "mmproj-Qwen3-ASR-0.6B-Q8_0.gguf",
            sha256: "41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d",
        }],
        download_bytes: 1_019_141_728,
        params: 600_000_000,
        precisions: &[Precision::Int8],
        devices: &[Device::Cpu, Device::Vulkan, Device::Cuda],
        languages: &[],
        overhead_mb: 600.0,
    },
    ModelManifest {
        id: "native-1.7b",
        label: "Qwen3-ASR 1.7B (native)",
        runtime: RuntimeKind::NativeAsr,
        repo: "ggml-org/Qwen3-ASR-1.7B-GGUF",
        revision: "36a678687ba7d07a74ca70ccb0e36902e005fb80",
        filename: "Qwen3-ASR-1.7B-Q8_0.gguf",
        dir_name: "qwen3-asr-1.7b-native",
        sha256: "58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57",
        auxiliary_files: &[WeightFile {
            filename: "mmproj-Qwen3-ASR-1.7B-Q8_0.gguf",
            sha256: "46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d",
        }],
        download_bytes: 2_520_744_288,
        params: 1_700_000_000,
        precisions: &[Precision::Int8],
        devices: &[Device::Cpu, Device::Vulkan, Device::Cuda],
        languages: &[],
        overhead_mb: 600.0,
    },
];

pub fn native_asr_manifest(id: &str) -> Option<&'static ModelManifest> {
    NATIVE_ASR_MODELS
        .iter()
        .find(|m| m.id == id || m.id.strip_prefix("native-") == Some(id))
}

pub fn asr_manifest(id: &str) -> Option<&'static ModelManifest> {
    ASR_MODELS.iter().find(|m| m.id == id)
}

pub fn refinement_manifest(id: &str) -> Option<&'static ModelManifest> {
    REFINEMENT_MODELS.iter().find(|m| m.id == id)
}

/// Every manifest, for the installer and the diagnostics panel.
pub fn all_manifests() -> impl Iterator<Item = &'static ModelManifest> {
    ASR_MODELS
        .iter()
        .chain(REFINEMENT_MODELS.iter())
        .chain(NATIVE_ASR_MODELS.iter())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_manifest_is_internally_consistent() {
        for manifest in all_manifests() {
            assert!(!manifest.id.is_empty());
            assert!(!manifest.label.is_empty());
            assert!(!manifest.repo.is_empty(), "{} has no repo", manifest.id);
            assert!(!manifest.dir_name.is_empty(), "{} has no dir", manifest.id);
            assert!(
                manifest.download_bytes > 0,
                "{} has no download size",
                manifest.id
            );
            assert!(
                manifest.params > 0,
                "{} has no parameter count",
                manifest.id
            );
            assert!(
                !manifest.precisions.is_empty(),
                "{} supports no precision",
                manifest.id
            );
            assert!(
                !manifest.devices.is_empty(),
                "{} supports no device",
                manifest.id
            );
            if manifest.runtime == RuntimeKind::NativeAsr {
                assert!(
                    manifest.languages.is_empty(),
                    "Native prototype must not claim language validation before benchmarking"
                );
            } else {
                assert!(
                    !manifest.languages.is_empty(),
                    "{} claims no validated language",
                    manifest.id
                );
            }
        }
    }

    #[test]
    fn ids_are_unique_across_every_registry() {
        let mut ids: Vec<&str> = all_manifests().map(|m| m.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(before, ids.len(), "duplicate manifest id");
    }

    /// The estimate has to be ordered the way the precisions are, or the
    /// resolver's ladder is meaningless.
    #[test]
    fn vram_estimates_shrink_as_precision_drops() {
        let m = asr_manifest("1.7b").expect("1.7b");
        let fp32 = m.estimated_vram_mb(Precision::Fp32);
        let bf16 = m.estimated_vram_mb(Precision::Bf16);
        let int8 = m.estimated_vram_mb(Precision::Int8);
        let int4 = m.estimated_vram_mb(Precision::Int4);
        assert!(fp32 > bf16, "{fp32} !> {bf16}");
        assert!(bf16 > int8, "{bf16} !> {int8}");
        assert!(int8 > int4, "{int8} !> {int4}");
    }

    /// Sanity-check the estimates against the numbers in the plan: 0.6B BF16 is
    /// about 2.2 GB resident, and 1.7B INT8 has to fit inside 4 GB.
    #[test]
    fn estimates_match_the_known_hardware_reality() {
        let small = asr_manifest("0.6b").expect("0.6b");
        let bf16 = small.estimated_vram_mb(Precision::Bf16);
        assert!(
            (1900.0..2600.0).contains(&bf16),
            "0.6B BF16 estimated at {bf16} MB, expected ~2.2 GB"
        );

        let large = asr_manifest("1.7b").expect("1.7b");
        let int8 = large.estimated_vram_mb(Precision::Int8);
        assert!(
            int8 < 4096.0,
            "1.7B INT8 must fit a 4 GB card, estimated {int8} MB"
        );
        // And 1.7B BF16 must not, which is the configuration Task 7 rejects.
        let large_bf16 = large.estimated_vram_mb(Precision::Bf16);
        assert!(
            large_bf16 > 4096.0,
            "1.7B BF16 should exceed 4 GB, estimated {large_bf16} MB"
        );
    }

    #[test]
    fn cpu_ram_estimate_exceeds_the_gpu_estimate() {
        for manifest in ASR_MODELS {
            let cpu = manifest.estimated_cpu_ram_mb();
            let gpu = manifest.estimated_vram_mb(Precision::Bf16);
            assert!(
                cpu > gpu,
                "{}: cpu {cpu} should exceed bf16 gpu {gpu}",
                manifest.id
            );
        }
    }

    #[test]
    fn language_matching_handles_regional_subtags() {
        let m = asr_manifest("0.6b").expect("0.6b");
        assert!(m.supports_language("en"));
        assert!(m.supports_language("EN"));
        assert!(m.supports_language("en-GB"), "regional subtag");
        assert!(m.supports_language("en_US"));
        assert!(m.supports_language("auto"), "auto defers the choice");
        assert!(m.supports_language(""));
        // Not validated yet, so it must not be claimed.
        assert!(!m.supports_language("hi"));
        assert!(!m.supports_language("bn"));
    }

    #[test]
    fn deep_context_model_points_at_a_real_repo() {
        let m = refinement_manifest("qwen3.5-2b").expect("qwen3.5-2b");
        assert_eq!(m.repo, "unsloth/Qwen3.5-2B-GGUF");
        assert_eq!(m.filename, "Qwen3.5-2B-Q4_K_M.gguf");
        assert!(!m.filename.contains("-Instruct-"));
    }

    /// Pinning is a release requirement, not an implementation detail.
    /// Task 44 ensures all distribution manifests are pinned and verifiable.
    #[test]
    fn verifiability_is_reported_honestly() {
        for manifest in all_manifests() {
            assert!(
                manifest.is_verifiable(),
                "{} must be verifiable with pinned revision and sha256",
                manifest.id
            );
        }
    }

    #[test]
    fn precision_and_device_parse_round_trip() {
        for p in [
            Precision::Fp32,
            Precision::Bf16,
            Precision::Int8,
            Precision::Int4,
        ] {
            assert_eq!(Precision::parse(p.as_str()), Some(p));
        }
        assert_eq!(Precision::parse("BF16"), Some(Precision::Bf16));
        assert_eq!(Precision::parse("nonsense"), None);

        for d in [Device::Cpu, Device::Cuda, Device::Vulkan] {
            assert_eq!(Device::parse(d.as_str()), Some(d));
        }
        // The UI has historically used "gpu" for CUDA.
        assert_eq!(Device::parse("gpu"), Some(Device::Cuda));
        assert_eq!(Device::parse("nonsense"), None);
        assert!(Device::Cuda.is_gpu() && Device::Vulkan.is_gpu());
        assert!(!Device::Cpu.is_gpu());
    }

    #[test]
    fn measured_peaks_distinguish_unmeasured_from_zero() {
        let empty = MeasuredPeaks::default();
        assert!(empty.is_empty());
        assert_eq!(empty.vram_mb, None, "unmeasured must not read as 0");

        let measured = MeasuredPeaks {
            vram_mb: Some(0.0),
            ..Default::default()
        };
        assert!(
            !measured.is_empty(),
            "a measured zero is still a measurement"
        );
    }
}
