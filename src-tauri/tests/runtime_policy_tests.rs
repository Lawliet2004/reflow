use reflow_lib::capability::{Capabilities, CpuInfo, CudaStatus, GpuInfo, GpuVendor, RamInfo};
use reflow_lib::profile::{no_measurements, Preset};
use reflow_lib::runtime::{effective_settings, plan};
use reflow_lib::settings::AppSettings;

fn hardware() -> Capabilities {
    Capabilities {
        cpu: CpuInfo {
            physical_cores: Some(6),
            logical_cores: 12,
            ..Default::default()
        },
        ram: RamInfo {
            total_mb: 16384.0,
            available_mb: 8192.0,
            ..Default::default()
        },
        cuda: CudaStatus {
            torch_cuda_available: true,
            ..Default::default()
        },
        gpus: vec![GpuInfo {
            index: 0,
            name: "RTX 2050".into(),
            vendor: GpuVendor::Nvidia,
            total_vram_mb: 4096.0,
            used_vram_mb: 596.0,
            free_vram_mb: 3500.0,
            driver_version: None,
            compute_capability: None,
        }],
        ..Default::default()
    }
}

#[test]
fn auto_uses_small_asr_and_reserves_its_weights_before_llm() {
    let mut settings = AppSettings::default();
    settings.asr.model = "1.7b".into();
    settings.asr.precision = "auto".into();
    let result = plan(
        &settings,
        &hardware(),
        &|_| true,
        &no_measurements,
        false,
        false,
    );
    assert_eq!(result.asr_model, "0.6b");
    assert_eq!(result.context_size, 1024);
    assert!(
        result.refinement_gpu_layers < 99,
        "both models cannot fully offload into this budget"
    );
}

#[test]
fn fast_execution_skips_llm_without_persisting_preferences() {
    let settings = AppSettings {
        preset: "fast".into(),
        ..Default::default()
    };
    let result = plan(
        &settings,
        &hardware(),
        &|_| true,
        &no_measurements,
        false,
        false,
    );
    let effective = effective_settings(&settings, &result);
    assert_eq!(result.refinement_model, None);
    assert!(!effective.resolve_intent().run_llm);
    assert!(settings.resolve_intent().run_llm);
}

#[test]
fn custom_settings_are_preserved_exactly() {
    let mut settings = AppSettings {
        preset: "custom".into(),
        ..Default::default()
    };
    settings.refinement.context_size = 3072;
    settings.refinement.keep_warm = false;
    let result = plan(
        &settings,
        &hardware(),
        &|_| true,
        &no_measurements,
        false,
        false,
    );
    let effective = effective_settings(&settings, &result);
    assert_eq!(result.preset, Preset::Custom);
    assert_eq!(
        serde_json::to_value(effective).unwrap(),
        serde_json::to_value(settings).unwrap()
    );
}

#[test]
fn low_ram_suppresses_additional_llm_load_and_warm_residency() {
    let mut caps = hardware();
    caps.ram.available_mb = 1000.0;
    let result = plan(
        &AppSettings::default(),
        &caps,
        &|_| true,
        &no_measurements,
        true,
        false,
    );
    assert!(result.refinement_model.is_none());
    assert!(!result.keep_refinement_warm);
    assert!(result
        .reasons
        .iter()
        .any(|r| r.code == "refinement_ram_pressure"));
}

#[test]
fn larger_gpu_gets_a_larger_bounded_context() {
    let mut caps = hardware();
    caps.gpus[0].total_vram_mb = 24576.0;
    caps.gpus[0].free_vram_mb = 22000.0;
    let result = plan(
        &AppSettings::default(),
        &caps,
        &|_| true,
        &no_measurements,
        false,
        false,
    );
    assert_eq!(result.context_size, 4096);
}

#[test]
fn transient_free_vram_does_not_change_a_warm_runtime_request() {
    let settings = AppSettings::default();
    let mut caps = hardware();
    let before = plan(&settings, &caps, &|_| true, &no_measurements, true, true);
    caps.gpus[0].free_vram_mb = 900.0;
    let after = plan(&settings, &caps, &|_| true, &no_measurements, true, true);
    let before = effective_settings(&settings, &before);
    let after = effective_settings(&settings, &after);
    assert_eq!(before.refinement.device, after.refinement.device);
    assert_eq!(before.refinement.gpu_layers, after.refinement.gpu_layers);
    assert_eq!(
        before.refinement.context_size,
        after.refinement.context_size
    );
}

#[test]
fn quantized_cpu_llm_is_not_budgeted_as_fp32_weights() {
    let mut caps = hardware();
    caps.ram.available_mb = 2500.0;
    let result = plan(
        &AppSettings::default(),
        &caps,
        &|_| true,
        &no_measurements,
        true,
        false,
    );
    assert!(result.refinement_model.is_some());
}

#[test]
fn custom_automatic_offload_budgets_the_resolved_writing_model() {
    let mut settings = AppSettings {
        preset: "custom".into(),
        intelligence_tier: "deep_context".into(),
        ..Default::default()
    };
    settings.refinement.device = "auto".into();
    settings.refinement.gpu_layers = -1;
    settings.memory_policy.allow_gpu_refinement = true;
    let mut caps = hardware();
    caps.gpus[0].free_vram_mb = 1800.0;
    let large = plan(&settings, &caps, &|_| true, &no_measurements, true, false);
    settings.intelligence_tier = "smart_flow".into();
    let small = plan(&settings, &caps, &|_| true, &no_measurements, true, false);
    assert_eq!(large.refinement_model.as_deref(), Some("qwen3.5-2b"));
    assert!(large.refinement_gpu_layers < small.refinement_gpu_layers);
}
