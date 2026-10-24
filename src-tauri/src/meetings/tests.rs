use super::*;
fn root() -> PathBuf {
    let p = std::env::temp_dir().join(format!("reflow-meeting-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&p).unwrap();
    p
}
fn meeting(encrypted: bool) -> Meeting {
    Meeting {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        config: MeetingConfig {
            microphone: None,
            playback: None,
            limit_hours: 5,
            summary_model: None,
        },
        encrypted,
        capture: "recording".into(),
        processing: "pending".into(),
        transcription_status: "pending".into(),
        summary_status: "pending".into(),
        elapsed_ms: 2000,
        saved_frames: [0; 2],
        last_commit_ms: 0,
        queue_depth: 0,
        peak_queue_depth: 0,
        levels: [0.0; 2],
        gaps: vec![],
        warning: None,
        error: None,
        stop_reason: None,
        processed_frames: 0,
        summary_progress: 0,
        audio_removed: false,
        asr_model: None,
        language: "en".into(),
    }
}
#[test]
fn accelerated_deadlines() {
    for hours in [1, 2, 5] {
        for seconds in (0..=hours * 3600).step_by(7) {
            assert_eq!(
                limit_reached(hours, seconds as u64 * 1000),
                seconds >= hours * 3600
            );
        }
        assert!(!limit_reached(hours, hours as u64 * 3_600_000 - 1));
        assert!(limit_reached(hours, hours as u64 * 3_600_000));
    }
    assert!(validate(&MeetingConfig {
        microphone: None,
        playback: None,
        limit_hours: 6,
        summary_model: None
    })
    .is_err());
}
#[test]
fn commit_recovery_and_checksum() {
    let root = root();
    let path = root.join("meeting.db");
    let mut m = meeting(false);
    {
        let mut s = store::Store::open_with_key(&path, None).unwrap();
        s.create(&m).unwrap();
        s.chunk(&mut m, 0, 0, 16000, &vec![0.25; 32000]).unwrap();
        assert_eq!(s.get(&m.id).unwrap().saved_frames, [32000, 0]);
    }
    let s = store::Store::open_with_key(&path, None).unwrap();
    s.recover().unwrap();
    let recovered = s.get(&m.id).unwrap();
    assert_eq!(recovered.capture, "interrupted");
    assert_eq!(recovered.saved_frames, [32000, 0]);
    let c = s.chunks(&m.id).unwrap();
    assert_eq!(c[0].start, 16000);
    assert_eq!(s.audio(&m.id, &c[0]).unwrap().len(), 32000);
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn encryption_audio_and_job_inputs_are_authenticated() {
    let root = root();
    let path = root.join("meeting.db");
    let mut m = meeting(true);
    let mut s = store::Store::open_with_key(&path, Some([42; 32])).unwrap();
    s.create(&m).unwrap();
    s.chunk(&mut m, 1, 0, 32000, &[0.5; 160]).unwrap();
    s.put(&m.id, "asr/v1/1", "private transcript marker")
        .unwrap();
    assert_eq!(
        s.work(&m.id, "asr/v1/1").unwrap().unwrap(),
        "private transcript marker"
    );
    let wrong = store::Store::open_with_key(&path, Some([43; 32])).unwrap();
    assert!(wrong.get(&m.id).is_err());
    assert!(wrong.audio(&m.id, &s.chunks(&m.id).unwrap()[0]).is_err());
    drop(wrong);
    drop(s);
    let data = std::fs::read(&path).unwrap();
    assert!(!data.windows(25).any(|b| b == b"private transcript marker"));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn retention_never_removes_unfinished_audio() {
    let root = root();
    let path = root.join("meeting.db");
    let mut m = meeting(false);
    let mut s = store::Store::open_with_key(&path, None).unwrap();
    s.create(&m).unwrap();
    s.chunk(&mut m, 0, 0, 0, &[0.1; 160]).unwrap();
    s.retention("disabled", "disabled").unwrap();
    assert_eq!(s.chunks(&m.id).unwrap().len(), 1);
    m.capture = "saved".into();
    m.processing = "ready".into();
    s.save(&m).unwrap();
    s.retention("disabled", "indefinite").unwrap();
    assert!(s.get(&m.id).unwrap().audio_removed);
    assert!(s.chunks(&m.id).unwrap().is_empty());
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn idempotent_work_and_restart_checkpoints() {
    let root = root();
    let s = store::Store::open_with_key(&root.join("meeting.db"), None).unwrap();
    let mut m = meeting(false);
    s.create(&m).unwrap();
    m.capture = "saved".into();
    m.processing = "transcribing".into();
    s.save(&m).unwrap();
    s.put(&m.id, "asr/one", "beginning").unwrap();
    s.put(&m.id, "asr/one", "beginning").unwrap();
    s.put(&m.id, "asr/last", "late critical action").unwrap();
    s.recover().unwrap();
    assert_eq!(s.get(&m.id).unwrap().processing, "interrupted");
    assert_eq!(
        s.work(&m.id, "asr/last").unwrap().unwrap(),
        "late critical action"
    );
    s.delete(&m.id).unwrap();
    assert!(s.get(&m.id).is_err());
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "records real native audio; set REFLOW_MEETING_SOAK_SECONDS=18000 for the required five-hour soak"]
fn native_soak() {
    let root = std::env::var_os("REFLOW_MEETING_TEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(root);
    let seconds = std::env::var("REFLOW_MEETING_SOAK_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(12);
    let ctx = AppContext::bootstrap_test(root.clone());
    ctx.settings_store
        .merge_update(serde_json::json!({"audio_retention":"forever"}))
        .unwrap();
    let mut m = meeting(false);
    m.config.microphone = std::env::var("REFLOW_MEETING_MIC").ok();
    #[cfg(target_os = "linux")]
    {
        m.config.playback = Some(
            std::env::var("REFLOW_MEETING_PLAYBACK").unwrap_or_else(|_| {
                format!(
                    "pipewire:{}.monitor",
                    crate::audio::pipewire::default_sink().expect("playback sink")
                )
            }),
        );
    }
    let id = m.id.clone();
    let active = Arc::new(Active::default());
    let supervisor = active.clone();
    let thread_ctx = ctx.clone();
    let guard = tauri::async_runtime::block_on(ctx.session_operation.clone().lock_owned());
    let (tx, rx) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || capture::run(thread_ctx, m, supervisor, tx, guard));
    rx.recv_timeout(Duration::from_secs(15))
        .expect("start acknowledgement")
        .expect("native capture starts");
    println!("MEETING_STARTED {} {}", root.display(), id);
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_secs(1));
        if active.finished.load(Ordering::Acquire) {
            break;
        }
        if let Some(m) = active.snapshot.read().as_ref() {
            println!(
                "MEETING_HEALTH {} {} {:?} {}",
                m.elapsed_ms,
                m.last_commit_ms,
                m.saved_frames,
                m.gaps.len()
            );
        }
    }
    active.stop.store(true, Ordering::Release);
    thread.join().unwrap();
    let store = store::Store::open_with_key(&ctx.meetings.path, None).unwrap();
    let saved = store.get(&id).unwrap();
    println!("MEETING_RESULT {}", serde_json::to_string(&saved).unwrap());
    assert_eq!(saved.capture, "saved");
    assert!(saved.saved_frames[0] > 0);
    assert!(saved.saved_frames[1] > 0);
    for c in store.chunks(&id).unwrap() {
        store.audio(&id, &c).unwrap();
    }
}
#[test]
fn chunk_failure_never_acknowledges_uncommitted_audio() {
    let root = root();
    let mut s = store::Store::open_with_key(&root.join("meeting.db"), None).unwrap();
    let mut m = meeting(false);
    s.create(&m).unwrap();
    s.chunk(&mut m, 0, 0, 0, &[0.2; 160]).unwrap();
    assert!(s.chunk(&mut m, 0, 0, 0, &[0.2; 160]).is_err());
    assert_eq!(m.saved_frames, [160, 0]);
    assert_eq!(s.get(&m.id).unwrap().saved_frames, [160, 0]);
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn windows_preserve_gaps_and_late_markers() {
    let root = root();
    let mut s = store::Store::open_with_key(&root.join("meeting.db"), None).unwrap();
    let mut m = meeting(false);
    s.create(&m).unwrap();
    for i in 0..35 {
        let mut pcm = vec![0.0; 32000];
        if [0, 17, 34].contains(&i) {
            pcm[8000] = 0.9;
        }
        let start = i * 32000 + if i >= 20 { 16000 } else { 0 };
        s.chunk(&mut m, 0, i, start, &pcm).unwrap();
    }
    let chunks = s.chunks(&m.id).unwrap();
    let ranges = process::windows(&s, &m.id, &chunks).unwrap();
    assert!(ranges.iter().all(|w| w.frames <= 480000));
    assert_eq!(ranges.iter().map(|w| w.frames).sum::<usize>(), 35 * 32000);
    assert!(ranges.iter().any(|w| w.start == 20 * 32000 + 16000));
    let mut markers = Vec::new();
    for w in &ranges {
        for (i, sample) in process::read_window(&s, &m.id, &chunks, w)
            .unwrap()
            .iter()
            .enumerate()
        {
            if *sample > 0.8 {
                markers.push(w.start + i as u64);
            }
        }
    }
    assert_eq!(
        markers,
        vec![8000, 17 * 32000 + 8000, 34 * 32000 + 16000 + 8000]
    );
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
#[ignore = "reconciles only the isolated REFLOW_MEETING_TEST_DIR used by the crash harness"]
fn recover_test_dir() {
    let root = PathBuf::from(
        std::env::var_os("REFLOW_MEETING_TEST_DIR").expect("isolated test dir required"),
    );
    let s = store::Store::open_with_key(&root.join("meetings.db"), None).unwrap();
    s.recover().unwrap();
    for m in s.list().unwrap() {
        assert_eq!(m.capture, "interrupted");
        for c in s.chunks(&m.id).unwrap() {
            s.audio(&m.id, &c).unwrap();
        }
        println!("RECOVERED {}", serde_json::to_string(&m).unwrap());
    }
}
#[test]
fn disk_full_rolls_back_chunk_and_counters() {
    let root = root();
    let mut s = store::Store::open_with_key(&root.join("meeting.db"), None).unwrap();
    let mut m = meeting(false);
    s.create(&m).unwrap();
    s.refuse_growth();
    assert!(s.chunk(&mut m, 0, 0, 0, &[0.4; 32000]).is_err());
    assert_eq!(m.saved_frames, [0, 0]);
    assert_eq!(s.get(&m.id).unwrap().saved_frames, [0, 0]);
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn privacy_migration_preserves_pcm_and_job_checkpoints() {
    let root = root();
    let mut s = store::Store::open_with_key(&root.join("meeting.db"), Some([42; 32])).unwrap();
    let mut m = meeting(false);
    s.create(&m).unwrap();
    s.chunk(&mut m, 0, 0, 0, &[0.5; 160]).unwrap();
    s.put(&m.id, "checkpoint", "private words").unwrap();
    let chunk = s.chunks(&m.id).unwrap().remove(0);
    let before = s.audio(&m.id, &chunk).unwrap();
    s.set_encryption(true).unwrap();
    assert!(s.get(&m.id).unwrap().encrypted);
    assert_eq!(s.audio(&m.id, &chunk).unwrap(), before);
    s.set_encryption(false).unwrap();
    assert!(!s.get(&m.id).unwrap().encrypted);
    assert_eq!(
        s.work(&m.id, "checkpoint").unwrap().unwrap(),
        "private words"
    );
    drop(s);
    std::fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn dictation_admission_cancel_and_concurrent_stop() {
    let root = root();
    let ctx = AppContext::bootstrap_test(root.clone());
    let active = Arc::new(Active::default());
    *active.snapshot.write() = Some(meeting(false));
    *ctx.meetings.active.write() = Some(active.clone());
    assert!(
        crate::session::start_external_owned(&ctx, Some("en".into()))
            .await
            .is_err()
    );
    crate::session::cancel(&ctx).unwrap();
    assert!(!active.stop.load(Ordering::Acquire));
    let finalizer = tokio::spawn(async move {
        while !active.stop.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        active.finished.store(true, Ordering::Release);
    });
    let (a, b) = tokio::join!(stop(&ctx), stop(&ctx));
    a.unwrap();
    b.unwrap();
    finalizer.await.unwrap();
    assert!(!ctx.meetings.recording());
    drop(ctx);
    std::fs::remove_dir_all(root).unwrap();
}
