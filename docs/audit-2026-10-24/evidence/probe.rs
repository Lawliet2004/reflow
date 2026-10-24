use reflow_lib::meetings::{Meeting, MeetingConfig, store::Store};
use reflow_lib::session::SessionContext;
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

fn main() {
 let root=std::path::PathBuf::from("/tmp/reflow-audit-20261024/probe-data");
 std::fs::create_dir_all(&root).unwrap();
 let path=root.join("meeting.db");
 let mut s=Store::open_with_key(&path,Some([42;32])).unwrap();
 let mut m=meeting(false); m.capture="saved".into(); m.processing="ready".into();
 s.create(&m).unwrap(); s.put(&m.id,"transcript","AUDIT_PRIVATE_PLAINTEXT_MARKER").unwrap();
 let reader=rusqlite::Connection::open(&path).unwrap();
 reader.execute_batch("BEGIN; SELECT * FROM meeting_work;").unwrap();
 let outcome=s.set_encryption(true);
 println!("encryption_with_reader={outcome:?}");
 println!("row_encrypted={}",s.get(&m.id).unwrap().encrypted);
 let wal=std::fs::read(path.with_extension("db-wal")).unwrap();
 println!("plaintext_in_wal={}",wal.windows(b"AUDIT_PRIVATE_PLAINTEXT_MARKER".len()).any(|v|v==b"AUDIT_PRIVATE_PLAINTEXT_MARKER"));
 let c=SessionContext {selected_text:Some("界".repeat(10000)), clipboard:Some("界".repeat(10000)),window_title:Some("界".repeat(10000)), ..Default::default()};
 println!("context_window=4096 captured_chars={} section_lengths={:?}",c.inputs(4096).iter().map(|(_,v)|v.chars().count()).sum::<usize>(),c.inputs(4096).iter().map(|(_,v)|v.chars().count()).collect::<Vec<_>>());
 reader.execute_batch("ROLLBACK").unwrap();
}
