//! Native streams live on the supervisor thread. Callbacks only convert a bounded
//! packet and try_send; resampling, encryption and SQLite run off the callback.
use super::*;
use crate::audio::AudioResampler;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc::{Receiver, SyncSender};
const PACKET_SAMPLES: usize = 8192;
const QUEUE_PACKETS: usize = 128;
const COMMIT_FRAMES: usize = 32_000; // 2s at 16kHz; room under the 10s crash RPO.
pub struct Packet {
    pub track: usize,
    pub at: u64,
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}
pub struct Native {
    _streams: Vec<(usize, cpal::Stream)>,
    #[cfg(target_os = "linux")]
    _pw: Vec<(usize, crate::audio::pipewire::PipeWireCapture)>,
}
fn send(sender: &SyncSender<Packet>, active: &Active, packet: Packet) {
    if active.stop.load(Ordering::Acquire) {
        return;
    }
    let track = packet.track;
    let start_ms = packet.at / 1000;
    let end_ms =
        start_ms + packet.samples.len() as u64 * 1000 / packet.channels as u64 / packet.rate as u64;
    let depth = active.queued.fetch_add(1, Ordering::AcqRel) + 1;
    match sender.try_send(packet) {
        Ok(()) => {
            active.peak.fetch_max(depth, Ordering::Relaxed);
        }
        Err(_) => {
            active.queued.fetch_sub(1, Ordering::AcqRel);
            active.loss_start[track].fetch_min(start_ms, Ordering::Relaxed);
            active.loss_end[track].fetch_max(end_ms, Ordering::Relaxed);
            active.overflow.store(true, Ordering::Release);
            active.stop.store(true, Ordering::Release);
        }
    }
}
fn cpal_stream(
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    track: usize,
    epoch: Instant,
    sender: SyncSender<Packet>,
    active: Arc<Active>,
) -> Result<cpal::Stream, String> {
    let rate = config.sample_rate().0;
    let channels = config.channels();
    let format = config.sample_format();
    if !matches!(
        format,
        cpal::SampleFormat::F32
            | cpal::SampleFormat::I16
            | cpal::SampleFormat::I32
            | cpal::SampleFormat::U16
    ) {
        return Err("Unsupported meeting audio sample format".into());
    }
    let errors = active.clone();
    let stream = device
        .build_input_stream_raw(
            &config.into(),
            format,
            move |data, info| {
                if active.stop.load(Ordering::Acquire) {
                    return;
                }
                let observed = epoch.elapsed().as_micros() as u64;
                let lag = info
                    .timestamp()
                    .callback
                    .duration_since(&info.timestamp().capture)
                    .map_or(0, |d| d.as_micros() as u64);
                let at = observed.saturating_sub(lag);
                macro_rules! dispatch {
                    ($kind:ty,$convert:expr) => {{
                        let input = data.as_slice::<$kind>().unwrap_or_default();
                        let size = PACKET_SAMPLES / channels as usize * channels as usize;
                        for (index, part) in input.chunks(size).enumerate() {
                            let samples = part.iter().map($convert).collect();
                            send(
                                &sender,
                                &active,
                                Packet {
                                    track,
                                    at: at
                                        + index as u64 * size as u64 * 1_000_000
                                            / (rate as u64 * channels as u64),
                                    rate,
                                    channels,
                                    samples,
                                },
                            );
                        }
                    }};
                }
                match format {
                    cpal::SampleFormat::F32 => {
                        dispatch!(f32, |s: &f32| if s.is_finite() { *s } else { 0.0 })
                    }
                    cpal::SampleFormat::I16 => dispatch!(i16, |s: &i16| *s as f32 / 32768.0),
                    cpal::SampleFormat::I32 => dispatch!(i32, |s: &i32| *s as f32 / 2147483648.0),
                    cpal::SampleFormat::U16 => {
                        dispatch!(u16, |s: &u16| (*s as f32 - 32768.0) / 32768.0)
                    }
                    _ => {}
                }
            },
            move |_| {
                errors.device_errors[track].store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}
impl Native {
    fn start(
        config: &MeetingConfig,
        epoch: Instant,
        tx: SyncSender<Packet>,
        active: Arc<Active>,
    ) -> Result<Self, String> {
        Self::start_selected(config, epoch, tx, active, None)
    }
    fn start_selected(
        config: &MeetingConfig,
        epoch: Instant,
        tx: SyncSender<Packet>,
        active: Arc<Active>,
        only: Option<usize>,
    ) -> Result<Self, String> {
        #[cfg(target_os="macos")]
        return Err("System audio recording is not yet available on macOS. Import an audio recording instead.".into());
        let host = cpal::default_host();
        let mut native = Self {
            _streams: Vec::new(),
            #[cfg(target_os = "linux")]
            _pw: Vec::new(),
        };
        for track in 0..2 {
            if only.is_some_and(|t| t != track) {
                continue;
            }
            let selected = if track == 0 {
                config.microphone.as_deref()
            } else {
                config.playback.as_deref()
            };
            #[cfg(target_os = "linux")]
            {
                use crate::audio::pipewire;
                let selected = if track == 0 {
                    let resolved = if let Some(id) = selected {
                        if id.starts_with(pipewire::ID_PREFIX) {
                            let devices =
                                pipewire::list_sources().ok_or("Cannot list microphone sources")?;
                            let source = devices
                                .iter()
                                .find(|d| d.id == id && d.is_available && !d.is_monitor)
                                .ok_or(
                                    "Selected microphone is unavailable; choose a connected source",
                                )?;
                            Some((
                                id.trim_start_matches(pipewire::ID_PREFIX).to_owned(),
                                source.name.clone(),
                            ))
                        } else {
                            pipewire::resolve_selected(id)?
                        }
                    } else {
                        pipewire::resolve_system_microphone()?
                    };
                    if resolved
                        .as_ref()
                        .is_some_and(|(node, _)| node.ends_with(".monitor"))
                    {
                        return Err("Select a microphone rather than a playback monitor".into());
                    }
                    resolved.map(|v| v.0)
                } else {
                    let id = selected.ok_or("Select a system playback monitor before recording")?;
                    let sources =
                        pipewire::list_sources().ok_or("PipeWire sources are unavailable")?;
                    let source = sources
                        .iter()
                        .find(|s| s.id == id && s.is_monitor && s.is_available)
                        .ok_or("Selected playback monitor is unavailable; refresh sources")?;
                    Some(
                        source
                            .id
                            .trim_start_matches(pipewire::ID_PREFIX)
                            .to_string(),
                    )
                };
                if let Some(node) = selected {
                    let sender = tx.clone();
                    let state = active.clone();
                    let error = active.clone();
                    let source_started = Instant::now();
                    let mut next_at = epoch.elapsed().as_micros() as u64;
                    native._pw.push((
                        track,
                        pipewire::PipeWireCapture::spawn_pinned(
                            &node,
                            move |input| {
                                let at = epoch.elapsed().as_micros() as u64;
                                let observed_start =
                                    at.saturating_sub(input.len() as u64 * 1_000_000 / 16_000);
                                // pw-record buffers during its startup health check.
                                // Anchor to stream creation, not delayed first read.
                                if source_started.elapsed() > Duration::from_secs(1)
                                    && observed_start > next_at + 250_000
                                {
                                    next_at = observed_start;
                                }
                                let start = next_at;
                                next_at += input.len() as u64 * 1_000_000 / 16_000;
                                for (i, samples) in input.chunks(PACKET_SAMPLES).enumerate() {
                                    send(
                                        &sender,
                                        &state,
                                        Packet {
                                            track,
                                            at: start
                                                + i as u64 * PACKET_SAMPLES as u64 * 1_000_000
                                                    / 16_000,
                                            rate: 16_000,
                                            channels: 1,
                                            samples: samples.to_vec(),
                                        },
                                    );
                                }
                            },
                            move |_| {
                                error.device_errors[track].store(true, Ordering::Release);
                            },
                        )?,
                    ));
                    continue;
                }
            }
            let (device, config) = if track == 0 {
                let device = if let Some(id) = selected {
                    host.input_devices()
                        .map_err(|e| e.to_string())?
                        .find(|d| d.name().is_ok_and(|n| n == id))
                        .ok_or("Selected microphone is unavailable; choose a connected source")?
                } else {
                    crate::audio::capture::resolve_input_device(&host, None)?.0
                };
                let config = device.default_input_config().map_err(|e| e.to_string())?;
                (device, config)
            } else {
                crate::audio::meeting::system_device(&host, selected)?
            };
            native._streams.push((
                track,
                cpal_stream(device, config, track, epoch, tx.clone(), active.clone())?,
            ));
        }
        Ok(native)
    }
}
#[derive(Default)]
struct Track {
    format: Option<(u32, u16)>,
    resampler: Option<AudioResampler>,
    pending: Vec<f32>,
    start: u64,
    next: u64,
    seq: u64,
}
impl Track {
    fn flush(
        &mut self,
        store: &mut store::Store,
        m: &mut Meeting,
        track: usize,
    ) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        store.chunk(m, track, self.seq, self.start, &self.pending)?;
        self.seq += 1;
        self.pending.clear();
        Ok(())
    }
    fn push(&mut self, p: Packet, store: &mut store::Store, m: &mut Meeting) -> Result<(), String> {
        if self.format != Some((p.rate, p.channels)) {
            self.resampler = Some(AudioResampler::new(p.rate, p.channels));
            self.format = Some((p.rate, p.channels));
        }
        let pcm = self.resampler.as_mut().unwrap().resample_f32(&p.samples);
        let observed = p.at * 16_000 / 1_000_000;
        if self.next == 0 {
            self.next = observed;
        }
        // Account for suspend/device stalls, rather than joining disconnected
        // samples. Small callback scheduling jitter stays on the sample clock.
        if observed > self.next + 4_000 {
            self.flush(store, m, p.track)?;
            if p.track == 0 {
                m.gaps.push(Gap {
                    track: p.track,
                    start_ms: self.next / 16,
                    end_ms: observed / 16,
                    reason: "No microphone callbacks (device or suspend gap)".into(),
                });
                m.warning = Some("Capture has gaps. See the saved timeline.".into());
            }
            // WASAPI legitimately delivers no loopback packets while playback
            // is silent. Preserve its offset without calling silence a failure.
            self.next = observed;
        }
        let peak = (pcm.iter().map(|v| v * v).sum::<f32>() / pcm.len().max(1) as f32).sqrt();
        m.levels[p.track] = (peak * 6.0).min(1.0);
        for part in pcm.chunks(COMMIT_FRAMES) {
            if self.pending.len() + part.len() > COMMIT_FRAMES {
                self.flush(store, m, p.track)?;
            }
            if self.pending.is_empty() {
                self.start = self.next;
            }
            // Enforce the monotonic deadline even for a callback queued at Stop.
            let count = part.len().min(
                (m.config.limit_hours as u64 * 3600 * 16_000).saturating_sub(self.next) as usize,
            );
            self.pending.extend_from_slice(&part[..count]);
            self.next += count as u64;
            if self.pending.len() >= COMMIT_FRAMES {
                self.flush(store, m, p.track)?;
            }
        }
        Ok(())
    }
}
pub fn estimated_bytes(hours: u32) -> u64 {
    hours as u64 * 3600 * 16_000 * 2 * 2 * 2 + 128 * 1024 * 1024
}
fn free_space(path: &std::path::Path) -> Option<u64> {
    sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().components().count())
        .map(|d| d.available_space())
}
pub fn run(
    ctx: AppContext,
    mut m: Meeting,
    active: Arc<Active>,
    ready: std::sync::mpsc::Sender<Result<(), String>>,
    _guard: tokio::sync::OwnedMutexGuard<()>,
) {
    let mut acknowledged = false;
    let result = (|| -> Result<(), String> {
        let mut store = store::Store::open(&ctx.meetings.path, m.encrypted)?;
        if free_space(&ctx.meetings.path).is_some_and(|n| n < estimated_bytes(m.config.limit_hours))
        {
            return Err("Insufficient disk space for the selected duration (includes WAL and safety headroom)".into());
        }
        let _inhibitor = super::inhibit::Inhibitor::acquire()?;
        let epoch = Instant::now();
        let (tx, rx): (SyncSender<Packet>, Receiver<Packet>) =
            std::sync::mpsc::sync_channel(QUEUE_PACKETS);
        freeze_sources(&mut m.config)?;
        let mut native = Native::start(&m.config, epoch, tx.clone(), active.clone())?;
        let mut retries = [0u32; 2];
        let mut reconnect_at = [None, None];
        store.create(&m)?;
        acknowledged = true;
        let _ = ready.send(Ok(()));
        let mut tracks = [Track::default(), Track::default()];
        let mut last_flush = Instant::now();
        let mut last_space = Instant::now();
        let mut failure = None;
        loop {
            m.elapsed_ms = epoch.elapsed().as_millis() as u64;
            if limit_reached(m.config.limit_hours, m.elapsed_ms) {
                active.stop.store(true, Ordering::Release);
                m.stop_reason = Some("Duration limit reached".into());
            }
            if active.stop.load(Ordering::Acquire) {
                break;
            }
            for t in 0..2 {
                if active.device_errors[t].swap(false, Ordering::AcqRel)
                    && reconnect_at[t].is_none()
                {
                    native._streams.retain(|(track, _)| *track != t);
                    #[cfg(target_os = "linux")]
                    native._pw.retain(|(track, _)| *track != t);
                    m.gaps.push(Gap {
                        track: t,
                        start_ms: tracks[t].next / 16,
                        end_ms: m.elapsed_ms,
                        reason: "Audio source disconnected".into(),
                    });
                    reconnect_at[t] = Some(Instant::now() + Duration::from_secs(1));
                    m.warning = Some(format!(
                        "{} disconnected. Retrying the same source; the other track continues.",
                        if t == 0 { "Microphone" } else { "Playback" }
                    ));
                }
                if reconnect_at[t].is_some_and(|at| Instant::now() >= at) {
                    retries[t] += 1;
                    match Native::start_selected(
                        &m.config,
                        epoch,
                        tx.clone(),
                        active.clone(),
                        Some(t),
                    ) {
                        Ok(mut source) => {
                            native._streams.append(&mut source._streams);
                            #[cfg(target_os = "linux")]
                            native._pw.append(&mut source._pw);
                            reconnect_at[t] = None;
                            if let Some(gap) =
                                m.gaps.iter_mut().rev().find(|g| {
                                    g.track == t && g.reason == "Audio source disconnected"
                                })
                            {
                                gap.end_ms = epoch.elapsed().as_millis() as u64;
                            }
                            m.warning = Some(
                                "Audio source reconnected. A capture gap is saved in the timeline."
                                    .into(),
                            );
                        }
                        Err(e) => {
                            if retries[t] >= 3 {
                                failure = Some(format!(
                                    "Selected source could not reconnect after 3 attempts: {e}"
                                ));
                                active.stop.store(true, Ordering::Release);
                            } else {
                                reconnect_at[t] =
                                    Some(Instant::now() + Duration::from_secs(1 << retries[t]));
                            }
                        }
                    }
                }
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(p) => {
                    active.queued.fetch_sub(1, Ordering::AcqRel);
                    if let Err(e) = tracks[p.track].push(p, &mut store, &mut m) {
                        failure = Some(e);
                        break;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => break,
            }
            if last_flush.elapsed() >= Duration::from_secs(2) {
                for (t, track) in tracks.iter_mut().enumerate() {
                    track.flush(&mut store, &mut m, t)?;
                }
                store.save(&m)?;
                last_flush = Instant::now();
            }
            if last_space.elapsed() >= Duration::from_secs(5) {
                #[cfg(target_os = "linux")]
                {
                    if let Some(devices) = crate::audio::pipewire::list_sources() {
                        for (t, id) in
                            [m.config.microphone.as_deref(), m.config.playback.as_deref()]
                                .into_iter()
                                .enumerate()
                        {
                            if let Some(id) = id.filter(|id| id.starts_with("pipewire:")) {
                                if !devices.iter().any(|d| d.id == id && d.is_available) {
                                    active.device_errors[t].store(true, Ordering::Release);
                                }
                            }
                        }
                    }
                }
                if free_space(&ctx.meetings.path).is_some_and(|n| n < 64 * 1024 * 1024) {
                    failure = Some("Disk space is low; recording safely finalized".into());
                    break;
                }
                last_space = Instant::now();
            }
            m.queue_depth = active.queued.load(Ordering::Acquire);
            m.peak_queue_depth = active.peak.load(Ordering::Relaxed);
            *active.snapshot.write() = Some(m.clone());
        }
        active.stop.store(true, Ordering::Release);
        drop(native);
        m.capture = "saving".into();
        *active.snapshot.write() = Some(m.clone());
        // Producers have stopped; drain all accepted packets before final commit.
        for p in rx.try_iter() {
            active.queued.fetch_sub(1, Ordering::AcqRel);
            let t = p.track;
            if let Err(e) = tracks[t].push(p, &mut store, &mut m) {
                failure = Some(e);
                break;
            }
        }
        for (t, track) in tracks.iter_mut().enumerate() {
            if let Err(e) = track.flush(&mut store, &mut m, t) {
                failure = Some(e);
            }
        }
        if active.overflow.load(Ordering::Acquire) {
            failure = Some(
                "Capture queue overflow. Recording stopped; accepted durable audio is preserved"
                    .into(),
            );
        }
        if active
            .device_errors
            .iter()
            .any(|e| e.load(Ordering::Acquire))
        {
            failure=Some("An audio device disconnected or failed. Recording stopped; reconnect it before starting another meeting".into());
        }
        m.capture = if failure.is_some() {
            "interrupted"
        } else {
            "saved"
        }
        .into();
        m.warning = failure.or(m.warning.take());
        if m.warning.is_some() {
            for (t, track) in tracks.iter().enumerate() {
                if m.elapsed_ms > track.next / 16 {
                    m.gaps.push(Gap {
                        track: t,
                        start_ms: track.next / 16,
                        end_ms: m.elapsed_ms,
                        reason: "Capture ended without confirmed audio".into(),
                    });
                }
            }
        }
        m.queue_depth = active.queued.load(Ordering::Acquire);
        m.peak_queue_depth = active.peak.load(Ordering::Relaxed);
        for t in 0..2 {
            let start = active.loss_start[t].load(Ordering::Acquire);
            let end = active.loss_end[t].load(Ordering::Acquire);
            if start != u64::MAX {
                m.gaps.push(Gap {
                    track: t,
                    start_ms: start,
                    end_ms: end,
                    reason: "Capture queue overflow (known failed packet)".into(),
                });
            }
        }
        m.levels = [0.0; 2];
        store.save(&m)?;
        *active.snapshot.write() = Some(m.clone());
        Ok(())
    })();
    if let Err(e) = result {
        if !acknowledged {
            let _ = ready.send(Err(e.clone()));
        }
        m.capture = "failed".into();
        m.warning = Some(e);
        if let Ok(s) = store::Store::open(&ctx.meetings.path, m.encrypted) {
            let _ = s.save(&m);
        }
        *active.snapshot.write() = Some(m.clone());
    }
    active.finished.store(true, Ordering::Release);
    drop(_guard);
    if acknowledged && m.capture == "saved" {
        let context = ctx.clone();
        let id = m.id.clone();
        tauri::async_runtime::spawn(async move {
            let _ = super::process::start(context, id, false).await;
        });
    }
}

pub(super) fn probe(config: &MeetingConfig, active: Arc<Active>) -> Result<[f32; 2], String> {
    let (tx, rx) = std::sync::mpsc::sync_channel(QUEUE_PACKETS);
    let epoch = Instant::now();
    let native = Native::start(config, epoch, tx, active.clone())?;
    let mut peaks = [0.0f32; 2];
    while epoch.elapsed() < Duration::from_secs(3) {
        if active
            .device_errors
            .iter()
            .any(|e| e.load(Ordering::Acquire))
            || active.overflow.load(Ordering::Acquire)
        {
            return Err("An audio source failed during the test. Check its connection.".into());
        }
        if let Ok(p) = rx.recv_timeout(Duration::from_millis(100)) {
            active.queued.fetch_sub(1, Ordering::AcqRel);
            let rms = (p.samples.iter().map(|v| v * v).sum::<f32>()
                / p.samples.len().max(1) as f32)
                .sqrt();
            peaks[p.track] = peaks[p.track].max((rms * 6.0).min(1.0));
        }
    }
    active.stop.store(true, Ordering::Release);
    drop(native);
    Ok(peaks)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_stops_without_silent_loss() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let a = Active::default();
        let packet = || Packet {
            track: 0,
            at: 0,
            rate: 16000,
            channels: 1,
            samples: vec![0.5; 160],
        };
        send(&tx, &a, packet());
        send(&tx, &a, packet());
        assert!(a.overflow.load(Ordering::Acquire));
        assert!(a.stop.load(Ordering::Acquire));
    }
}

fn freeze_sources(config: &mut MeetingConfig) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    if config.microphone.is_none()
        || config.microphone.as_deref() == Some(crate::audio::pipewire::AUTO_ID)
    {
        if let Some((node, _)) = crate::audio::pipewire::resolve_system_microphone()? {
            config.microphone = Some(format!("pipewire:{node}"));
        }
    }
    #[cfg(target_os = "windows")]
    if config.microphone.is_none() {
        config.microphone = Some(
            cpal::default_host()
                .default_input_device()
                .ok_or("No microphone available")?
                .name()
                .map_err(|e| e.to_string())?,
        );
    }
    Ok(())
}
