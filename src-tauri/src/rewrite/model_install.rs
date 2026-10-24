//! One owned, observable transfer for each optional writing model.
use crate::{
    context::AppContext,
    network_policy::{ArtifactDownload, TransferProgress},
};
use futures_util::FutureExt;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use tauri::{AppHandle, Emitter};

static NEXT_EVENT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Serialize)]
pub struct ModelDownloadEvent {
    pub tier: String,
    pub attempt_id: String,
    pub revision: u64,
    pub model_revision: String,
    pub runtime_generation: String,
    pub phase: String,
    pub progress_pct: u32,
    pub speed_mbps: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct ModelDownload {
    pub snapshot: ModelDownloadEvent,
    pub cancel: Arc<AtomicBool>,
}

impl ModelDownload {
    pub fn active(&self) -> bool {
        !matches!(
            self.snapshot.phase.as_str(),
            "complete" | "error" | "cancelled"
        )
    }
    fn update(
        &mut self,
        attempt_id: &str,
        update: impl FnOnce(&mut ModelDownloadEvent),
    ) -> Option<ModelDownloadEvent> {
        if self.snapshot.attempt_id != attempt_id || !self.active() {
            return None;
        }
        update(&mut self.snapshot);
        if self.cancel.load(Ordering::Acquire) && self.active() {
            self.snapshot.phase = "cancelling".into();
        }
        self.snapshot.revision = NEXT_EVENT.fetch_add(1, Ordering::Relaxed);
        Some(self.snapshot.clone())
    }
    fn request_cancel(&self, attempt_id: &str) -> Result<(), String> {
        if !self.active() || self.snapshot.attempt_id != attempt_id {
            return Err("That model download has ended; refresh its status".into());
        }
        if self.snapshot.phase == "committing" {
            return Err("The verified model is being activated; wait for setup to finish".into());
        }
        self.cancel.store(true, Ordering::Release);
        Ok(())
    }
}

fn publish(
    ctx: &AppContext,
    app: &AppHandle,
    tier: &str,
    attempt_id: &str,
    update: impl FnOnce(&mut ModelDownloadEvent),
) {
    let snapshot = {
        let mut jobs = ctx.active_intelligence_downloads.lock();
        let Some(job) = jobs.get_mut(tier) else {
            return;
        };
        let Some(snapshot) = job.update(attempt_id, update) else {
            return;
        };
        snapshot
    };
    // Publish after committing state and releasing the lock; status refreshes
    // cannot observe an active job after its terminal event.
    let _ = app.emit("intelligence:download-progress", snapshot);
}

pub fn start(tier: String, app: AppHandle, ctx: AppContext) -> Result<(), String> {
    let tier = match tier.trim().to_ascii_lowercase().as_str() {
        "qwen3.5-0.8b" => "smart_flow".to_owned(),
        "qwen3.5-2b" => "deep_context".to_owned(),
        id => id.to_owned(),
    };
    let model = match tier.as_str() {
        "smart_flow" => "qwen3.5-0.8b",
        "deep_context" => "qwen3.5-2b",
        id @ ("qwen3.5-4b" | "minicpm5-2b" | "gemma4-e4b") => id,
        _ => return Err(format!("Unknown writing model or tier: {tier}")),
    };
    let existing = ctx
        .active_intelligence_downloads
        .lock()
        .get(&tier)
        .filter(|job| job.active())
        .map(|job| job.snapshot.clone());
    if let Some(snapshot) = existing {
        let _ = app.emit("intelligence:download-progress", snapshot);
        return Ok(());
    }
    crate::network_policy::require_online(ctx.settings_store.get().offline_mode)?;
    let manifest = crate::profile::manifest::refinement_manifest(model)
        .ok_or("Refinement manifest missing")?;
    let destination = super::flow_gguf_path(model);
    let partial = destination.with_extension("gguf.part");
    let parent = destination.parent().ok_or("Invalid model directory")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let cached = std::fs::metadata(&partial)
        .map(|file| file.len())
        .unwrap_or(0);
    crate::network_policy::check_disk_space(
        parent,
        manifest.download_bytes.saturating_sub(cached),
    )?;
    let operation = ctx
        .session_operation
        .try_lock()
        .map_err(|_| "Finish the active operation before starting model setup")?;
    let root = crate::platform::PlatformSys::get_app_dir().join("bin");
    let generation = super::runtime_inventory::active_manifest(&root)
        .map(|manifest| manifest.id)
        .unwrap_or_else(|| "legacy-or-custom".into());
    let attempt_id = uuid::Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = ctx.active_intelligence_downloads.lock();
        if jobs.get(&tier).is_some_and(ModelDownload::active) {
            return Ok(());
        }
        jobs.insert(
            tier.clone(),
            ModelDownload {
                cancel: cancel.clone(),
                snapshot: ModelDownloadEvent {
                    tier: tier.clone(),
                    attempt_id: attempt_id.clone(),
                    revision: NEXT_EVENT.fetch_add(1, Ordering::Relaxed),
                    model_revision: manifest.revision.into(),
                    runtime_generation: generation.clone(),
                    phase: "checking_runtime".into(),
                    progress_pct: 0,
                    speed_mbps: 0.0,
                    downloaded_bytes: cached,
                    total_bytes: manifest.download_bytes,
                    error: None,
                },
            },
        );
    }
    publish(&ctx, &app, &tier, &attempt_id, |_| {});
    drop(operation);
    tauri::async_runtime::spawn(async move {
        let runtime_ctx = ctx.clone();
        let runtime_app = app.clone();
        let runtime_tier = tier.clone();
        let runtime_attempt = attempt_id.clone();
        let runtime_cancel = cancel.clone();
        let verified = tokio::task::spawn_blocking(move || {
            let backend = runtime_ctx.settings_store.get().refinement.device;
            super::runtime_install::ensure_for_model(
                &runtime_app,
                &runtime_ctx,
                &backend,
                runtime_cancel,
                |phase| {
                    publish(
                        &runtime_ctx,
                        &runtime_app,
                        &runtime_tier,
                        &runtime_attempt,
                        |event| {
                            event.phase = phase.into();
                        },
                    )
                },
            )
        })
        .await
        .map_err(|error| format!("Runtime setup worker stopped: {error}"))
        .and_then(|result| result);
        match verified {
            Ok(generation) if !cancel.load(Ordering::Acquire) => {
                publish(&ctx, &app, &tier, &attempt_id, |event| {
                    event.runtime_generation = generation;
                });
            }
            outcome => {
                publish(&ctx, &app, &tier, &attempt_id, |event| {
                    event.phase = if cancel.load(Ordering::Acquire) {
                        "cancelled"
                    } else {
                        "error"
                    }
                    .into();
                    event.error = outcome.err();
                });
                return;
            }
        }
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            manifest.repo, manifest.revision, manifest.filename
        );
        let started = Instant::now();
        let initial = cached;
        let result = std::panic::AssertUnwindSafe(crate::network_policy::fetch_verified(
            ArtifactDownload {
                url: &url,
                sha256: manifest.sha256,
                exact_bytes: Some(manifest.download_bytes),
                partial: &partial,
                destination: &destination,
            },
            &cancel,
            |progress: TransferProgress| {
                publish(&ctx, &app, &tier, &attempt_id, |event| {
                    event.phase = progress.phase.into();
                    event.downloaded_bytes = progress.bytes;
                    event.total_bytes = progress.total.unwrap_or(manifest.download_bytes);
                    event.progress_pct = ((progress.bytes as u128 * 100
                        / event.total_bytes.max(1) as u128)
                        .min(99)) as u32;
                    event.speed_mbps = progress.bytes.saturating_sub(initial) as f32
                        / 1_048_576.0
                        / started.elapsed().as_secs_f32().max(0.001);
                });
            },
        ))
        .catch_unwind()
        .await
        .unwrap_or_else(|_| {
            Err(
                "Model download stopped unexpectedly. Retry to resume verified partial data."
                    .into(),
            )
        });
        publish(&ctx, &app, &tier, &attempt_id, |event| {
            event.phase = match &result {
                Ok(()) => "complete",
                Err(_) if cancel.load(Ordering::Acquire) => "cancelled",
                Err(_) => "error",
            }
            .into();
            if result.is_ok() {
                event.progress_pct = 100;
                event.downloaded_bytes = manifest.download_bytes;
            }
            event.error = result.err();
        });
    });
    Ok(())
}

pub fn cancel(
    ctx: &AppContext,
    app: &AppHandle,
    tier: &str,
    attempt_id: &str,
) -> Result<(), String> {
    {
        let jobs = ctx.active_intelligence_downloads.lock();
        let job = jobs
            .get(tier)
            .ok_or("That model download has ended; refresh its status")?;
        job.request_cancel(attempt_id)?;
    }
    publish(ctx, app, tier, attempt_id, |event| {
        event.phase = "cancelling".into()
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn job(id: &str) -> ModelDownload {
        ModelDownload {
            cancel: Arc::new(AtomicBool::new(false)),
            snapshot: ModelDownloadEvent {
                tier: "smart_flow".into(),
                attempt_id: id.into(),
                revision: 0,
                model_revision: "pinned".into(),
                runtime_generation: "verified".into(),
                phase: "downloading".into(),
                progress_pct: 20,
                speed_mbps: 0.0,
                downloaded_bytes: 20,
                total_bytes: 100,
                error: None,
            },
        }
    }
    #[test]
    fn prerequisite_phases_remain_owned_and_cancellable() {
        for phase in ["checking_runtime", "installing_runtime"] {
            let mut current = job("owned");
            current
                .update("owned", |event| event.phase = phase.into())
                .unwrap();
            assert!(current.active());
            current.request_cancel("owned").unwrap();
            current
                .update("owned", |event| event.phase = "downloading".into())
                .unwrap();
            assert_eq!(current.snapshot.phase, "cancelling");
        }
    }

    #[test]
    fn stale_cancellation_and_progress_cannot_take_over_a_replacement() {
        let mut current = job("replacement");
        assert!(current.request_cancel("previous").is_err());
        assert!(!current.cancel.load(Ordering::Acquire));
        assert!(current
            .update("previous", |event| event.phase = "complete".into())
            .is_none());
        assert!(current.active());
        assert_eq!(current.snapshot.progress_pct, 20);
    }
    #[test]
    fn cancellation_racing_completion_cannot_revive_a_terminal_job() {
        let mut current = job("owned");
        current.request_cancel("owned").unwrap();
        current
            .update("owned", |event| event.phase = "cancelled".into())
            .unwrap();
        assert!(current
            .update("owned", |event| event.phase = "cancelling".into())
            .is_none());
        assert!(!current.active());
        assert_eq!(current.snapshot.phase, "cancelled");
    }
    #[test]
    fn commit_decision_and_cancellation_have_a_defined_order() {
        let mut committing = job("committing");
        committing
            .update("committing", |event| event.phase = "committing".into())
            .unwrap();
        assert!(committing.request_cancel("committing").is_err());
        assert!(!committing.cancel.load(Ordering::Acquire));
        committing
            .update("committing", |event| event.phase = "complete".into())
            .unwrap();
        assert!(!committing.active());
        let mut cancelled = job("cancelled");
        cancelled.request_cancel("cancelled").unwrap();
        cancelled
            .update("cancelled", |event| event.phase = "committing".into())
            .unwrap();
        assert_eq!(cancelled.snapshot.phase, "cancelling");
    }
}
