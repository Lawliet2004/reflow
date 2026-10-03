//! Local-only file import, note storage, and correction learning contracts.

use reflow_lib::asr::{ASREngine, MockASREngine};
use reflow_lib::audio::file_decode::{decode_file, wav_or_pcm_to_f32, CONFIRM_FILE_BYTES};
use reflow_lib::context::AppContext;
use reflow_lib::expansion_commands::{correction_suggestions, note_entry, notes_markdown};
use reflow_lib::file_jobs::{segment_audio, subtitle_text, transcribe_samples, TranscriptSegment};
use reflow_lib::history::db::HistoryQuery;
use reflow_lib::history::HistoryStore;
use reflow_lib::settings::{AppSettings, OutputAction};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

struct LocalFiles(std::path::PathBuf);

impl LocalFiles {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("reflow-phase2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

impl Drop for LocalFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn pcm_wav(rate: u32, channels: u16, frames: usize) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
        for frame in 0..frames {
            for channel in 0..channels {
                // A repeatable nonconstant signal also catches channel/phase errors.
                let sample = ((frame % 97) as i16 - 48) * 200 + channel as i16 * 500;
                writer.write_sample(sample).unwrap();
            }
        }
        writer.finalize().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn segmentation_covers_each_sample_once_with_bounded_nonempty_chunks() {
    for size in [0, 1, 16_000, 30 * 16_000, 61 * 16_000 + 37] {
        let samples = vec![0.1; size];
        let ranges = segment_audio(&samples);
        let mut covered = 0;
        for (start, end) in ranges {
            assert_eq!(start, covered, "gap or overlap at sample {covered}");
            assert!(end > start);
            assert!(end - start <= 30 * 16_000);
            covered = end;
        }
        assert_eq!(covered, samples.len());
    }
}

#[test]
fn segmentation_places_a_boundary_at_a_quiet_window_near_thirty_seconds() {
    let mut samples = vec![0.5; 40 * 16_000];
    let quiet_start = 27 * 16_000;
    samples[quiet_start..quiet_start + 1600].fill(0.0);

    let ranges = segment_audio(&samples);
    assert_eq!(ranges[0], (0, quiet_start));
    assert_eq!(ranges[1], (quiet_start, samples.len()));
}

#[test]
fn subtitle_exports_keep_unicode_and_correct_hour_and_millisecond_timing() {
    let segments = vec![TranscriptSegment {
        text: "नमस्ते\nBonjour".into(),
        offset_ms: 3_599_999,
        end_ms: 3_601_234,
        speakers: None,
    }];
    assert_eq!(subtitle_text(&segments, "txt").unwrap(), "नमस्ते\nBonjour\n");
    assert_eq!(
        subtitle_text(&segments, "srt").unwrap(),
        "1\n00:59:59,999 --> 01:00:01,234\nनमस्ते\nBonjour\n\n"
    );
    assert_eq!(
        subtitle_text(&segments, "vtt").unwrap(),
        "WEBVTT\n\n1\n00:59:59.999 --> 01:00:01.234\nनमस्ते\nBonjour\n\n"
    );
    assert!(subtitle_text(&segments, "html").is_err());
}

#[test]
fn file_decoder_matches_existing_wav_conversion_for_mono_and_resampled_stereo() {
    let files = LocalFiles::new();
    for (rate, channels) in [(16_000, 1), (44_100, 2), (48_000, 2)] {
        let bytes = pcm_wav(rate, channels, rate as usize / 2);
        let path = files.path(&format!("{rate}-{channels}.wav"));
        std::fs::write(&path, &bytes).unwrap();
        let expected = wav_or_pcm_to_f32(&bytes).unwrap();
        let actual = decode_file(&path, false).unwrap();

        assert_eq!(
            actual.len(),
            expected.len(),
            "rate {rate}, channels {channels}"
        );
        for (actual, expected) in actual.iter().zip(&expected) {
            assert!(
                (actual - expected).abs() < 0.000_001,
                "rate {rate}, channels {channels}: {actual} versus {expected}"
            );
        }
    }
}

#[test]
fn file_decoder_rejects_invalid_empty_and_out_of_range_audio() {
    let files = LocalFiles::new();
    for (name, bytes) in [
        ("corrupt.wav", b"not a wave file".to_vec()),
        ("empty.wav", pcm_wav(16_000, 1, 0)),
        ("invalid-rate.wav", pcm_wav(7999, 1, 100)),
    ] {
        let path = files.path(name);
        std::fs::write(&path, bytes).unwrap();
        assert!(decode_file(&path, false).is_err(), "accepted {name}");
    }
    assert!(decode_file(&files.0, false).is_err());
}

#[test]
fn file_decoder_rejects_nonfinite_float_audio() {
    let files = LocalFiles::new();
    let path = files.path("nonfinite.wav");
    let mut writer = hound::WavWriter::create(
        &path,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )
    .unwrap();
    writer.write_sample(f32::NAN).unwrap();
    writer.finalize().unwrap();

    assert!(decode_file(&path, false).is_err());
}

#[test]
fn large_file_import_requires_confirmation_before_decoding() {
    let files = LocalFiles::new();
    let path = files.path("large.wav");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(CONFIRM_FILE_BYTES + 1).unwrap();
    drop(file);

    let error = decode_file(&path, false).expect_err("large import needs confirmation");
    assert!(error.contains("100 MB"), "{error}");
}

#[test]
fn notes_query_excludes_other_kinds_and_pins_survive_reopening() {
    let files = LocalFiles::new();
    let path = files.path("history.db");
    let mut older = note_entry("Older pinned note".into());
    older.created_at = "2026-01-01T12:00:00Z".into();
    let mut newer = note_entry("Newer note".into());
    newer.created_at = "2026-01-02T12:00:00Z".into();
    let mut dictation = note_entry("Ordinary dictation".into());
    dictation.kind = "dictation".into();
    {
        let store = HistoryStore::new(path.clone()).unwrap();
        for entry in [&older, &newer, &dictation] {
            store.insert_entry(entry).unwrap();
        }
        store
            .update_metadata(&older.id, true, "work,planning")
            .unwrap();
    }
    let store = HistoryStore::new(path).unwrap();
    let page = store
        .query_entries(&HistoryQuery {
            kind: Some("note".into()),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.entries[0].id, older.id);
    assert_eq!(page.entries[1].id, newer.id);
    assert_eq!(page.entries[0].tags, "work,planning");
    let pinned = store
        .query_entries(&HistoryQuery {
            kind: Some("note".into()),
            pinned_only: Some(true),
            tag: Some("work".into()),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(pinned.total, 1);
    assert_eq!(pinned.entries[0].id, older.id);
}

#[test]
fn notes_markdown_preserves_multiline_text_and_uses_current_edited_transcript() {
    let mut note = note_entry("Original dictation".into());
    note.created_at = "2026-10-02T12:00:00Z".into();
    note.final_transcript = "# My note\n\n- café\n- नमस्ते\n```text\ncode\n```".into();
    assert_eq!(
        notes_markdown(std::slice::from_ref(&note)),
        format!("## {}\n\n{}\n\n", note.created_at, note.final_transcript)
    );
    assert!(notes_markdown(&[]).is_empty());
}

#[test]
fn correction_learning_accepts_spelling_case_and_hyphen_changes_without_punctuation() {
    for (before, after, expected_before, expected_after) in [
        ("Use refloww.", "Use reflow.", "refloww", "reflow"),
        ("Use reflow.", "Use Reflow.", "reflow", "Reflow"),
        ("Use re-flow.", "Use Reflow.", "re-flow", "Reflow"),
    ] {
        let suggestions = correction_suggestions(before, after);
        assert_eq!(suggestions.len(), 1, "{before} -> {after}");
        assert_eq!(suggestions[0].before, expected_before);
        assert_eq!(suggestions[0].after, expected_after);
        assert_eq!(suggestions[0].frequency, 1);
    }
}

#[test]
fn correction_learning_recognizes_common_transposed_letter_spelling_errors() {
    let suggestions = correction_suggestions("Please recieve this.", "Please receive this.");
    assert_eq!(suggestions.len(), 1);
    assert_eq!(suggestions[0].before, "recieve");
    assert_eq!(suggestions[0].after, "receive");
}

#[test]
fn correction_learning_does_not_learn_punctuation_or_broad_rewrites() {
    for (before, after) in [
        ("Send this today", "Send this today!"),
        ("Please purchase that item", "Please sell that item"),
        (
            "The draft needs a shorter introduction",
            "Shorten the draft",
        ),
        ("alpha beta gamma", "Alpha Beta Gamma"),
    ] {
        assert!(
            correction_suggestions(before, after).is_empty(),
            "learned rewrite: {before} -> {after}"
        );
    }
}

#[derive(Default)]
struct AudioWork {
    starts: AtomicUsize,
    received_samples: AtomicUsize,
    stops: AtomicUsize,
    cancellations: AtomicUsize,
}

/// The cancellation signal is raised by the engine itself when its first
/// packet arrives, so cancellation between packets requires no timing sleeps.
struct CancellationProbe {
    engine: MockASREngine,
    work: Arc<AudioWork>,
    cancel_on_audio: Option<Arc<AtomicBool>>,
}

impl ASREngine for CancellationProbe {
    fn initialize(&mut self) -> Result<(), String> {
        self.engine.initialize()
    }

    fn load_model_with_precision(
        &mut self,
        directory: &str,
        backend: &str,
        precision: &str,
    ) -> Result<(), String> {
        self.engine
            .load_model_with_precision(directory, backend, precision)
    }

    fn unload_model(&mut self) -> Result<(), String> {
        self.engine.unload_model()
    }

    fn is_model_loaded(&self) -> bool {
        self.engine.is_model_loaded()
    }

    fn start_stream(&mut self, language: &str, vocabulary: &[String]) -> Result<(), String> {
        self.work.starts.fetch_add(1, Ordering::SeqCst);
        self.engine.start_stream(language, vocabulary)
    }

    fn push_audio(&mut self, samples: &[f32]) -> Result<Option<String>, String> {
        self.work
            .received_samples
            .fetch_add(samples.len(), Ordering::SeqCst);
        if let Some(cancel) = &self.cancel_on_audio {
            cancel.store(true, Ordering::SeqCst);
        }
        self.engine.push_audio(samples)
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        self.engine.get_partial_transcript()
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        self.work.stops.fetch_add(1, Ordering::SeqCst);
        self.engine.stop_stream()
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        self.work.cancellations.fetch_add(1, Ordering::SeqCst);
        self.engine.cancel_stream()
    }

    fn get_detected_language(&self) -> String {
        self.engine.get_detected_language()
    }

    fn get_backend_name(&self) -> String {
        self.engine.get_backend_name()
    }
}

#[tokio::test]
async fn already_cancelled_file_does_no_asr_work_and_writes_no_history() {
    let files = LocalFiles::new();
    let ctx = AppContext::bootstrap_test(files.path("cancelled"));
    let work = Arc::new(AudioWork::default());
    ctx.asr_handle
        .swap_engine(Box::new(CancellationProbe {
            engine: MockASREngine::new(),
            work: work.clone(),
            cancel_on_audio: None,
        }))
        .await
        .unwrap();
    let cancel = AtomicBool::new(true);
    let mut progress = Vec::new();
    let samples = vec![0.2; 32_000];
    let error = transcribe_samples(&ctx, &samples, "en", &cancel, |done| progress.push(done))
        .await
        .expect_err("cancelled import must stop");

    assert!(error.to_lowercase().contains("cancelled"));
    assert_eq!(work.starts.load(Ordering::SeqCst), 0);
    assert_eq!(work.received_samples.load(Ordering::SeqCst), 0);
    assert!(progress.is_empty());
    assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
    drop(ctx);
}

#[tokio::test]
async fn cancellation_between_audio_packets_discards_partial_output_and_cancels_stream() {
    let files = LocalFiles::new();
    let ctx = AppContext::bootstrap_test(files.path("cancel-between-packets"));
    let cancel = Arc::new(AtomicBool::new(false));
    let work = Arc::new(AudioWork::default());
    let mut engine = MockASREngine::new();
    engine.initialize().unwrap();
    ctx.asr_handle
        .swap_engine(Box::new(CancellationProbe {
            engine,
            work: work.clone(),
            cancel_on_audio: Some(cancel.clone()),
        }))
        .await
        .unwrap();
    let mut progress = Vec::new();
    let samples = vec![0.2; 48_000];
    let error = transcribe_samples(&ctx, &samples, "en", &cancel, |done| progress.push(done))
        .await
        .expect_err("cancelled import must not return partial segments");

    assert!(error.to_lowercase().contains("cancelled"));
    assert_eq!(work.starts.load(Ordering::SeqCst), 1);
    assert_eq!(work.received_samples.load(Ordering::SeqCst), 16_000);
    assert_eq!(work.stops.load(Ordering::SeqCst), 0);
    assert_eq!(work.cancellations.load(Ordering::SeqCst), 1);
    assert!(progress.is_empty());
    assert!(ctx.history_store.get_entries(10, 0).unwrap().is_empty());
    drop(ctx);
}

#[test]
fn old_settings_deserialize_directly_with_safe_expansion_defaults() {
    let mut old = serde_json::to_value(AppSettings::default()).unwrap();
    let document = old.as_object_mut().unwrap();
    for field in [
        "notes_folder",
        "dictionary_suggestions",
        "dismissed_corrections",
        "hotkeys",
        "output_action",
        "send_key",
        "modes",
        "default_mode_id",
        "mode_triggers_enabled",
        "snippets",
    ] {
        document.remove(field);
    }
    document.insert("settings_version".into(), 4.into());
    document.insert("hotkey".into(), "Ctrl+Shift+F8".into());
    document.insert("language".into(), "bn".into());
    let settings: AppSettings =
        serde_json::from_value(old).expect("old saved settings must decode");

    assert_eq!(settings.hotkey, "Ctrl+Shift+F8");
    assert_eq!(settings.language, "bn");
    assert!(settings.notes_folder.is_empty());
    assert!(settings.dictionary_suggestions.is_empty());
    assert!(settings.dismissed_corrections.is_empty());
    assert_eq!(settings.output_action, OutputAction::Paste);
    assert_eq!(settings.send_key, "enter");
    assert_eq!(settings.default_mode_id, "dictation");
    assert_eq!(settings.modes.len(), 4);
    assert!(settings.mode_triggers_enabled);
    assert!(settings.snippets.is_empty());
}
