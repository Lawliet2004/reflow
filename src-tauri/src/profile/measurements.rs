//! Peaks measured on this machine, persisted so resolver decisions outlive
//! the process that measured them.
//!
//! The resolver stays pure: it is handed a [`MeasuredLookup`] closure, and the
//! file I/O lives here. Keying by a hardware fingerprint means a GPU swap (or
//! a copy of the app directory landing on another machine) reads as "not
//! measured" rather than confidently reusing the wrong card's numbers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::manifest::{Device, MeasuredPeaks, Precision};
use crate::capability::Capabilities;

/// Which hardware a measurement belongs to. CPU rows travel with the CPU
/// model name; GPU rows with the primary GPU's name.
fn hardware_fingerprint(caps: &Capabilities, device: Device) -> String {
    match device {
        Device::Cpu => format!(
            "cpu:{}:threads={}:torch={}",
            caps.cpu.model,
            caps.cpu.physical_cores.unwrap_or(caps.cpu.logical_cores),
            caps.cuda.torch_cuda_version.as_deref().unwrap_or("cpu")
        ),
        _ => format!(
            "gpu:{}:driver={}:torch={}",
            caps.primary_gpu()
                .map(|g| g.name.as_str())
                .unwrap_or("unknown"),
            caps.primary_gpu()
                .and_then(|g| g.driver_version.as_deref())
                .unwrap_or("unknown"),
            caps.cuda.torch_cuda_version.as_deref().unwrap_or("none")
        ),
    }
}

/// Map keys are plain strings because JSON object keys have to be.
fn key(caps: &Capabilities, model_id: &str, device: Device, precision: Precision) -> String {
    format!(
        "{}|{}|{}|{}",
        hardware_fingerprint(caps, device),
        model_id,
        device.as_str(),
        precision.as_str()
    )
}

#[derive(Debug, Default)]
pub struct Measurements {
    entries: HashMap<String, MeasuredPeaks>,
}

fn measurements_path() -> PathBuf {
    crate::platform::PlatformSys::get_app_dir().join("asr_measurements.json")
}

impl Measurements {
    /// The stored table, or an empty one. A corrupt or missing file is "no
    /// measurements yet", not an error — the resolver already knows how to
    /// run on estimates.
    pub fn load() -> Self {
        let entries = std::fs::read_to_string(measurements_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self { entries }
    }

    pub fn lookup(
        &self,
        caps: &Capabilities,
        model_id: &str,
        device: Device,
        precision: Precision,
    ) -> MeasuredPeaks {
        self.entries
            .get(&key(caps, model_id, device, precision))
            .copied()
            .unwrap_or_default()
    }

    /// Merge a new observation into the row for this rung and persist.
    ///
    /// Peaks keep the worst observed value (`vram_mb`, `ram_mb`,
    /// `load_seconds`) — a peak that fluctuated downward is how a config gets
    /// admitted that later OOMs. Rates (`rtf`, `wer`) keep the latest
    /// observation so a warm cache or a newer runtime build is reflected.
    pub fn record(
        &mut self,
        caps: &Capabilities,
        model_id: &str,
        device: Device,
        precision: Precision,
        peaks: MeasuredPeaks,
    ) {
        if peaks.is_empty() {
            return;
        }
        static WRITER: OnceLock<parking_lot::Mutex<()>> = OnceLock::new();
        let _writer = WRITER.get_or_init(|| parking_lot::Mutex::new(())).lock();
        // A watcher and a completed dictation can record concurrently. Merge
        // from the latest complete file instead of overwriting another writer.
        self.entries = Self::load().entries;
        let entry = self
            .entries
            .entry(key(caps, model_id, device, precision))
            .or_default();
        merge(entry, peaks);

        if let Some(parent) = measurements_path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.entries) {
            let path = measurements_path();
            let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let result =
                std::fs::write(&temporary, json).and_then(|()| std::fs::rename(&temporary, &path));
            if let Err(error) = result {
                log::warn!("Could not persist ASR measurements: {error}");
                let _ = std::fs::remove_file(temporary);
            }
        }
    }
}

/// Fold one observation into a stored row. Peaks keep the worst value seen;
/// rates keep the latest.
fn merge(entry: &mut MeasuredPeaks, peaks: MeasuredPeaks) {
    entry.vram_mb = peaks
        .vram_mb
        .map(|v| entry.vram_mb.map_or(v, |e| e.max(v)))
        .or(entry.vram_mb);
    entry.ram_mb = peaks
        .ram_mb
        .map(|v| entry.ram_mb.map_or(v, |e| e.max(v)))
        .or(entry.ram_mb);
    entry.load_seconds = peaks
        .load_seconds
        .map(|v| entry.load_seconds.map_or(v, |e| e.max(v)))
        .or(entry.load_seconds);
    entry.rtf = peaks.rtf.or(entry.rtf);
    entry.wer = peaks.wer.or(entry.wer);
}

/// Build the closure the resolver takes, capturing this machine's caps.
pub fn lookup_fn<'a>(
    store: &'a Measurements,
    caps: &'a Capabilities,
) -> impl Fn(&str, Device, Precision) -> MeasuredPeaks + 'a {
    move |model_id, device, precision| store.lookup(caps, model_id, device, precision)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_worst_peaks_and_latest_rates() {
        let mut row = MeasuredPeaks {
            vram_mb: Some(1800.0),
            rtf: Some(0.30),
            ..Default::default()
        };
        // A later, luckier observation must not lower the VRAM peak — that is
        // how a config gets admitted that OOMs under load.
        merge(
            &mut row,
            MeasuredPeaks {
                vram_mb: Some(1600.0),
                rtf: Some(0.08),
                ..Default::default()
            },
        );
        assert_eq!(row.vram_mb, Some(1800.0));
        assert_eq!(row.rtf, Some(0.08));
        // A worse peak does raise the bar; empty fields never erase data.
        merge(
            &mut row,
            MeasuredPeaks {
                vram_mb: Some(2100.0),
                ..Default::default()
            },
        );
        assert_eq!(row.vram_mb, Some(2100.0));
        assert_eq!(row.rtf, Some(0.08));
    }
}
