//! Adaptive ASR model selection.
//!
//! The behaviour under test exists because of a measured 6x latency cliff on the
//! machine this was developed against — an RTX 2050 with 4096 MiB total and
//! ~1899 MiB already held by the desktop, leaving ~2064 MiB free:
//!
//! | configuration            | transcribing 7.3s of audio |
//! |--------------------------|----------------------------|
//! | 1.7B, did not fit -> CPU | 17546 ms                   |
//! | 0.6B on the GPU          |  2887 ms                   |
//!
//! The runtime's only fallback used to be GPU -> CPU, so asking for the larger
//! model bought a 6x slowdown for an accuracy difference nowhere near 6x. The
//! user's Settings choice is therefore a ceiling, not a command: it may be
//! stepped down to keep the work on the GPU, and every substitution is reported.

use reflow_lib::capability::{Capabilities, GpuInfo, GpuVendor};
use reflow_lib::profile::{no_measurements, select_asr_load, Device, Precision, Preset};

#[test]
fn phonon_keeps_its_own_engine_under_every_preset() {
    for preset in [
        Preset::Auto,
        Preset::Fast,
        Preset::Balanced,
        Preset::Accurate,
        Preset::Custom,
    ] {
        let chosen = select_asr_load(
            preset,
            "phonon-2",
            "cpu",
            "auto",
            &|_| true,
            &Capabilities::default(),
            &no_measurements,
        );
        assert_eq!(chosen.model_id, "phonon-2");
        assert_eq!(chosen.device, Device::Cpu);
        assert_eq!(chosen.precision, Precision::Int8);
        assert!(chosen.error.is_none());
    }
}

#[test]
fn phonon_uses_cuda_when_it_fits_and_cpu_when_it_does_not() {
    for (free, expected) in [(8000.0, Device::Cuda), (200.0, Device::Cpu)] {
        let chosen = select_asr_load(
            Preset::Auto,
            "phonon-2",
            "auto",
            "auto",
            &|_| true,
            &caps_with_free_vram(free),
            &no_measurements,
        );
        assert_eq!(chosen.model_id, "phonon-2");
        assert_eq!(chosen.device, expected);
        assert!(chosen.error.is_none());
    }
    let chosen = select_asr_load(
        Preset::Custom,
        "phonon-2",
        "cpu",
        "int4",
        &|_| true,
        &Capabilities::default(),
        &no_measurements,
    );
    assert!(chosen.error.is_some());
}

#[test]
fn zipformer_stays_cpu_int8_under_every_preset_and_runtime() {
    for preset in [
        Preset::Auto,
        Preset::Fast,
        Preset::Balanced,
        Preset::Accurate,
        Preset::Custom,
    ] {
        for runtime in ["python", "native"] {
            let chosen = reflow_lib::profile::select_asr_load_for_runtime(
                runtime,
                preset,
                "zipformer-20m",
                "auto",
                "auto",
                &|_| true,
                &caps_with_free_vram(8000.0),
                &no_measurements,
            );
            assert_eq!(chosen.model_id, "zipformer-20m");
            assert_eq!(chosen.device, Device::Cpu, "{runtime}/{preset:?}");
            assert_eq!(chosen.precision, Precision::Int8);
            assert!(chosen.error.is_none());
        }
    }
}

#[test]
fn zipformer_explains_ignored_gpu_and_rejects_forced_precision() {
    let chosen = select_asr_load(
        Preset::Custom,
        "zipformer-20m",
        "cuda",
        "auto",
        &|_| true,
        &caps_with_free_vram(8000.0),
        &no_measurements,
    );
    assert_eq!(chosen.device, Device::Cpu);
    assert!(
        chosen.downgrade.is_some(),
        "an ignored GPU request is explained"
    );

    let chosen = select_asr_load(
        Preset::Custom,
        "zipformer-20m",
        "cpu",
        "bf16",
        &|_| true,
        &caps_with_free_vram(8000.0),
        &no_measurements,
    );
    assert_eq!(
        chosen.error.as_ref().map(|e| e.code.as_str()),
        Some("asr_precision_unsupported")
    );
}

/// A 4 GB card with `free_mb` actually available.
fn caps_with_free_vram(free_mb: f32) -> Capabilities {
    let mut caps = Capabilities::default();
    caps.gpus.push(GpuInfo {
        index: 0,
        name: "NVIDIA GeForce RTX 2050".into(),
        vendor: GpuVendor::Nvidia,
        total_vram_mb: 4096.0,
        used_vram_mb: 4096.0 - free_mb,
        free_vram_mb: free_mb,
        driver_version: None,
        compute_capability: None,
    });
    caps.cuda.torch_cuda_available = true;
    caps
}

fn both_installed(_id: &str) -> bool {
    true
}

#[test]
fn both_python_models_accept_explicit_int4_without_substitution() {
    for model in ["0.6b", "1.7b"] {
        let chosen = select_asr_load(
            Preset::Custom,
            model,
            "cuda",
            "int4",
            &both_installed,
            &caps_with_free_vram(3962.0),
            &no_measurements,
        );
        assert!(chosen.error.is_none(), "{model}: {:?}", chosen.error);
        assert_eq!(chosen.model_id, model);
        assert_eq!(chosen.device, Device::Cuda);
        assert_eq!(chosen.precision, Precision::Int4);
        assert!(chosen.downgrade.is_none());
    }
}

fn only_small_installed(id: &str) -> bool {
    id == "0.6b"
}

/// With the GPU essentially free, the requested model is honoured.
#[test]
fn a_model_that_fits_is_loaded_as_requested() {
    let caps = caps_with_free_vram(3962.0);
    let selection = select_asr_load(
        Preset::Balanced,
        "1.7b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(selection.model_id, "1.7b");
    assert_eq!(selection.device, Device::Cuda);
    assert!(
        selection.downgrade.is_none(),
        "honouring the request must not report a downgrade: {:?}",
        selection.downgrade
    );
}

/// The step-down itself: the requested model does not fit, a smaller installed
/// one does, so ASR stays on the GPU rather than falling to the CPU.
///
/// 2600 MB free gives a budget of 2600 - 512 (display reserve) - 256 (transient
/// margin) = 1832 MB. The 0.6B model estimates at ~1277 MB and the 1.7B at
/// ~2575 MB, so exactly one of them is admissible.
#[test]
fn a_model_that_does_not_fit_steps_down_instead_of_falling_to_cpu() {
    let caps = caps_with_free_vram(2600.0);
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(
        selection.device,
        Device::Cuda,
        "the whole point is to keep ASR on the GPU; CPU measured 6x slower"
    );
    assert_eq!(selection.model_id, "0.6b");
    let reason = selection
        .downgrade
        .expect("a substitution must be explained to the user");
    assert!(
        reason.contains("0.6B") && reason.contains("1.7B"),
        "the reason must name both models, got: {reason}"
    );
}

/// The exact state of the development machine with its desktop resident, and the
/// case the whole feature exists for.
///
/// 2064 MB free, budget = 2064 - 512 (display reserve) - 256 (transient) = 1296
/// MB. The 0.6B model estimates at ~1277 MB (629 MB of int8 weights + ~648 MB of
/// precision-adjusted overhead), so it is admitted; the 1.7B at ~2575 MB is not.
///
/// This previously failed. With a 768 MB reserve and an overhead figure that
/// ignored precision, the budget was 1040 MB against a 1529 MB estimate, so
/// nothing was admitted and ASR dropped to the CPU — 17546 ms to transcribe 7.3s
/// of audio, against 2887 ms for this configuration, which was measured loading
/// on that same GPU in that same state using 1362 MiB and leaving 704 MiB free
/// with no spill.
#[test]
fn the_measured_working_configuration_is_admitted_to_the_gpu() {
    let caps = caps_with_free_vram(2064.0);
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(
        selection.device,
        Device::Cuda,
        "0.6B on this GPU was measured at 2887 ms; the CPU fallback was 17546 ms"
    );
    assert_eq!(selection.model_id, "0.6b");
    assert!(
        selection.downgrade.is_some(),
        "stepping down from the requested model must still be explained"
    );
}

/// Never step *up*. Asking for the small model gets the small model even on a
/// machine with room to spare.
#[test]
fn selection_never_upgrades_beyond_the_request() {
    let caps = caps_with_free_vram(3962.0);
    let selection = select_asr_load(
        Preset::Auto,
        "0.6b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(selection.model_id, "0.6b");
    assert!(selection.downgrade.is_none());
}

/// A model that is not on disk is not a candidate, however well it would fit.
#[test]
fn only_installed_models_are_considered() {
    let caps = caps_with_free_vram(3962.0);
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "auto",
        "int8",
        &only_small_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(
        selection.model_id, "0.6b",
        "selecting weights that are not downloaded would just fail the load"
    );
}

/// An explicit CPU request is honoured, but takes the smallest model: on CPU the
/// binding constraint is throughput, and a bigger model is slower without being
/// more usable.
#[test]
fn an_explicit_cpu_request_uses_the_smallest_model() {
    let caps = caps_with_free_vram(3962.0);
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "cpu",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(selection.device, Device::Cpu);
    assert_eq!(selection.model_id, "0.6b");
    assert_eq!(
        selection.precision,
        Precision::Fp32,
        "the CPU path does not quantize"
    );
    assert!(selection.downgrade.is_some(), "explain the substitution");
}

/// No usable GPU at all: CPU, smallest model, and say so.
#[test]
fn with_no_gpu_it_falls_back_to_cpu_and_explains() {
    let caps = Capabilities::default();
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(selection.device, Device::Cpu);
    assert_eq!(selection.model_id, "0.6b");
    assert!(selection.downgrade.is_some());
}

/// A GPU so full that nothing fits must still produce a usable configuration
/// rather than insisting on a model it cannot load.
#[test]
fn a_full_gpu_falls_back_to_cpu_with_the_smallest_model() {
    let caps = caps_with_free_vram(200.0);
    let selection = select_asr_load(
        Preset::Auto,
        "1.7b",
        "auto",
        "int8",
        &both_installed,
        &caps,
        &no_measurements,
    );

    assert_eq!(selection.device, Device::Cpu);
    assert_eq!(selection.model_id, "0.6b");
    let reason = selection.downgrade.expect("must explain");
    assert!(
        reason.to_lowercase().contains("vram"),
        "the reason should name the constraint, got: {reason}"
    );
}

/// The preset has to change what actually loads.
///
/// It previously reached only `preview_profile`, the preview command, so picking
/// "Fast" or "Accurate" in Settings changed what the preview reported and nothing
/// about the dictation that followed. These tests exist so that cannot regress
/// into being decorative again.
mod preset_drives_selection {
    use super::*;

    /// Fast means latency: take the smallest installed model even on a GPU with
    /// room for the larger one.
    #[test]
    fn fast_takes_the_smallest_model_despite_ample_vram() {
        let caps = caps_with_free_vram(3962.0);
        let selection = select_asr_load(
            Preset::Fast,
            "1.7b",
            "auto",
            "int8",
            &both_installed,
            &caps,
            &no_measurements,
        );

        assert_eq!(
            selection.device,
            Device::Cuda,
            "Fast still belongs on the GPU"
        );
        assert_eq!(selection.model_id, "0.6b");
        let reason = selection.downgrade.expect("explain the preset's choice");
        assert!(
            reason.to_lowercase().contains("fast profile"),
            "the reason must attribute the choice to the preset, got: {reason}"
        );
    }

    /// Accurate reaches for the best model that fits, above what Settings names.
    #[test]
    fn accurate_reaches_above_the_settings_model() {
        let caps = caps_with_free_vram(3962.0);
        let selection = select_asr_load(
            Preset::Accurate,
            "0.6b",
            "auto",
            "int8",
            &both_installed,
            &caps,
            &no_measurements,
        );

        assert_eq!(selection.model_id, "1.7b");
        assert_eq!(selection.device, Device::Cuda);
        let reason = selection.downgrade.expect("explain the preset's choice");
        assert!(
            reason.to_lowercase().contains("accurate profile"),
            "got: {reason}"
        );
    }

    /// Accurate cannot conjure VRAM: it still steps down when the best model does
    /// not fit, and reports the hardware reason rather than the preset.
    #[test]
    fn accurate_still_respects_the_vram_budget() {
        let caps = caps_with_free_vram(2064.0);
        let selection = select_asr_load(
            Preset::Accurate,
            "0.6b",
            "auto",
            "int8",
            &both_installed,
            &caps,
            &no_measurements,
        );

        assert_eq!(selection.model_id, "0.6b");
        assert_eq!(selection.device, Device::Cuda);
    }

    /// Balanced and Auto defer to the model named in Settings.
    #[test]
    fn balanced_and_auto_defer_to_settings() {
        let caps = caps_with_free_vram(3962.0);
        for preset in [Preset::Balanced, Preset::Auto] {
            let selection = select_asr_load(
                preset,
                "0.6b",
                "auto",
                "int8",
                &both_installed,
                &caps,
                &no_measurements,
            );
            assert_eq!(
                selection.model_id,
                "0.6b",
                "{} must not override the user's model",
                preset.as_str()
            );
            assert!(selection.downgrade.is_none());
        }
    }

    /// Custom means the user is driving; their model is honoured when it fits.
    #[test]
    fn custom_honours_the_users_model() {
        let caps = caps_with_free_vram(3962.0);
        let selection = select_asr_load(
            Preset::Custom,
            "1.7b",
            "auto",
            "int8",
            &both_installed,
            &caps,
            &no_measurements,
        );

        assert_eq!(selection.model_id, "1.7b");
        assert!(selection.downgrade.is_none());
    }

    /// Even Fast cannot put a model on a GPU that is not usable.
    #[test]
    fn fast_on_a_machine_with_no_gpu_uses_the_cpu() {
        let caps = Capabilities::default();
        let selection = select_asr_load(
            Preset::Fast,
            "1.7b",
            "auto",
            "int8",
            &both_installed,
            &caps,
            &no_measurements,
        );

        assert_eq!(selection.device, Device::Cpu);
        assert_eq!(selection.model_id, "0.6b");
    }
}

#[test]
fn native_runtime_uses_vulkan_without_python_cuda() {
    let mut caps = caps_with_free_vram(3962.0);
    caps.cuda.torch_cuda_available = false;
    let chosen = reflow_lib::profile::select_asr_load_for_runtime(
        "native",
        Preset::Auto,
        "1.7b",
        "auto",
        "auto",
        &both_installed,
        &caps,
        &no_measurements,
    );
    assert_eq!(chosen.model_id, "native-0.6b");
    assert_eq!(chosen.device, Device::Vulkan);
    assert_eq!(chosen.precision, Precision::Int8);
    let cpu = reflow_lib::profile::select_asr_load_for_runtime(
        "native",
        Preset::Auto,
        "0.6b",
        "cpu",
        "auto",
        &both_installed,
        &caps,
        &no_measurements,
    );
    assert_eq!(cpu.device, Device::Cpu);
    assert_eq!(cpu.model_id, "native-0.6b");
}

#[test]
fn native_selection_only_considers_native_installs() {
    let chosen = reflow_lib::profile::select_asr_load_for_runtime(
        "native",
        Preset::Accurate,
        "0.6b",
        "auto",
        "auto",
        &|id| id == "native-0.6b",
        &caps_with_free_vram(3962.0),
        &no_measurements,
    );
    assert_eq!(chosen.model_id, "native-0.6b");
}
