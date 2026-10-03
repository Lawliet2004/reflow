use crate::{
    context::AppContext,
    history::{db::HistoryQuery, HistoryEntry},
};
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

#[tauri::command]
pub async fn summarize_history(id: String, ctx: State<'_, AppContext>) -> Result<String, String> {
    let ctx = ctx.inner().clone();
    let _operation = ctx.session_operation.lock().await;
    if !matches!(
        *ctx.state_enum.read(),
        crate::state::AppStateEnum::Ready | crate::state::AppStateEnum::Idle
    ) {
        return Err("Finish the current recording before summarizing".into());
    }
    let entry = ctx
        .history_store
        .get_entry(&id)?
        .ok_or("Transcript no longer exists")?;
    let settings = ctx.settings_store.get();
    let intent = settings.resolve_intent();
    let model = if intent.flow_model == "none" {
        "qwen3.5-0.8b".to_owned()
    } else {
        intent.flow_model
    };
    let runtime = ctx.flow_runtime.clone();
    tokio::task::spawn_blocking(move || {
        runtime.ensure(
            &model,
            &settings.refinement.device,
            (settings.refinement.gpu_layers >= 0).then_some(settings.refinement.gpu_layers as u32),
            settings.memory_policy.vram_reserve_mb,
            settings.refinement.context_size.max(4096),
        )?;
        let client = runtime.client.read().clone();
        let result = crate::rewrite::tasks::summarize_transcript(&client, &entry.final_transcript);
        if !settings.refinement.keep_warm {
            runtime.shutdown();
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionarySuggestion {
    pub before: String,
    pub after: String,
    pub frequency: u32,
}

pub fn correction_suggestions(before: &str, after: &str) -> Vec<DictionarySuggestion> {
    let tokens = |text: &str| {
        text.split_whitespace()
            .map(|s| {
                s.trim_matches(|c: char| !c.is_alphanumeric() && c != '-')
                    .to_owned()
            })
            .collect::<Vec<_>>()
    };
    let a = tokens(before);
    let b = tokens(after);
    if a.len() != b.len() {
        return vec![];
    }
    let changes = a.iter().zip(&b).filter(|(a, b)| a != b).collect::<Vec<_>>();
    if changes.len() > 2 {
        return vec![];
    }
    changes
        .into_iter()
        .filter_map(|(a, b)| {
            // Dictionary terms are limited to 512 bytes. Longer prose/code
            // tokens cannot become terms and must not trigger quadratic work.
            if a.is_empty() || b.is_empty() || a.len() > 512 || b.len() > 512 {
                return None;
            }
            let simple = |s: &str| s.to_lowercase().replace('-', "");
            let ac = simple(a);
            let bc = simple(b);
            let same = ac == bc;
            let close = a.chars().count() >= 4
                && b.chars().count() >= 4
                && (edit_distance(&ac, &bc) <= 1 || adjacent_transposition(&ac, &bc));
            if !(same || close) {
                return None;
            }
            Some(DictionarySuggestion {
                before: a.clone(),
                after: b.clone(),
                frequency: 1,
            })
        })
        .collect()
}
fn adjacent_transposition(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len() != b.len() {
        return false;
    }
    let differences: Vec<usize> = a
        .iter()
        .zip(&b)
        .enumerate()
        .filter_map(|(i, (x, y))| (x != y).then_some(i))
        .collect();
    differences.len() == 2
        && differences[1] == differences[0] + 1
        && a[differences[0]] == b[differences[1]]
        && a[differences[1]] == b[differences[0]]
}
fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1)
                .min(old + 1)
                .min(previous + usize::from(x != *y));
            previous = old;
        }
    }
    row[b.len()]
}

pub fn note_entry(text: String) -> HistoryEntry {
    HistoryEntry {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        duration_ms: 0,
        language: String::new(),
        raw_transcript: text.clone(),
        smart_transcript: text.clone(),
        word_count: text.split_whitespace().count(),
        character_count: text.chars().count(),
        final_transcript: text,
        rewriter_used: false,
        application_name: "Reflow Notes".into(),
        application_process: "reflow".into(),
        model_version: "manual".into(),
        processing_mode: "raw".into(),
        kind: "note".into(),
        source: "manual".into(),
        command_input: None,
        pinned: false,
        tags: String::new(),
        audio_available: false,
        audio_expires_at: None,
    }
}
#[tauri::command]
pub fn add_note(text: String, ctx: State<'_, AppContext>) -> Result<HistoryEntry, String> {
    if text.trim().is_empty() || text.chars().count() > 100_000 {
        return Err("Enter a note of at most 100,000 characters".into());
    }
    let entry = note_entry(text);
    ctx.history_store.insert_entry(&entry)?;
    Ok(entry)
}
#[tauri::command]
pub fn update_history_metadata(
    id: String,
    pinned: bool,
    tags: String,
    ctx: State<'_, AppContext>,
) -> Result<(), String> {
    ctx.history_store.update_metadata(&id, pinned, &tags)
}
#[tauri::command]
pub fn edit_history_transcript(
    id: String,
    text: String,
    ctx: State<'_, AppContext>,
    app: tauri::AppHandle,
) -> Result<HistoryEntry, String> {
    if text.chars().count() > 100_000 {
        return Err("Transcript exceeds 100,000 characters".into());
    }
    let _guard = ctx.settings_operation.lock();
    let entry = ctx
        .history_store
        .get_entry(&id)?
        .ok_or("Transcript no longer exists")?;
    let mut settings = ctx.settings_store.get();
    for suggestion in correction_suggestions(&entry.final_transcript, &text) {
        let pair = format!("{}\n{}", suggestion.before, suggestion.after);
        if settings.dismissed_corrections.contains(&pair)
            || settings
                .dictionary_terms
                .iter()
                .any(|term| term.preferred_spelling == suggestion.after)
        {
            continue;
        }
        if let Some(existing) = settings
            .dictionary_suggestions
            .iter_mut()
            .find(|s| s.before == suggestion.before && s.after == suggestion.after)
        {
            existing.frequency = existing.frequency.saturating_add(1);
        } else if settings.dictionary_suggestions.len() < 200 {
            settings.dictionary_suggestions.push(suggestion);
        }
    }
    // Persist suggestions first; if it fails, preserve the user's edit draft in the UI.
    ctx.settings_store.merge_update(
        serde_json::json!({"dictionary_suggestions": settings.dictionary_suggestions}),
    )?;
    ctx.history_store.update_transcript(
        &id,
        &entry.smart_transcript,
        &text,
        entry.rewriter_used,
    )?;
    let _ = app.emit("settings:changed", ctx.settings_store.get());
    ctx.history_store
        .get_entry(&id)?
        .ok_or("Transcript no longer exists".into())
}
pub fn notes_markdown(entries: &[HistoryEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("## {}\n\n{}\n\n", e.created_at, e.final_transcript))
        .collect()
}
#[tauri::command]
pub async fn export_notes(ctx: State<'_, AppContext>) -> Result<String, String> {
    let store = ctx.history_store.clone();
    let folder = ctx.settings_store.get().notes_folder;
    tokio::task::spawn_blocking(move || {
        let directory = if folder.trim().is_empty() {
            dirs::document_dir()
                .unwrap_or_else(crate::platform::PlatformSys::get_app_dir)
                .join("Reflow Notes")
        } else {
            std::path::PathBuf::from(folder)
        };
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let mut offset = 0;
        let mut markdown = String::new();
        loop {
            let page = store.query_entries(&HistoryQuery {
                kind: Some("note".into()),
                limit: Some(200),
                offset: Some(offset),
                ..Default::default()
            })?;
            markdown.push_str(&notes_markdown(&page.entries));
            if page.next_offset.is_none() {
                break;
            }
            offset = page.next_offset.unwrap();
        }
        let path = directory.join(format!("Reflow-notes-{}.md", uuid::Uuid::new_v4()));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        file.write_all(markdown.as_bytes())
            .map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn get_stats(ctx: State<'_, AppContext>) -> Result<crate::history::stats::UsageStats, String> {
    ctx.history_store.usage_stats()
}
#[tauri::command]
pub async fn repaste_last(ctx: State<'_, AppContext>) -> Result<bool, String> {
    let entry = ctx
        .history_store
        .get_entries(1, 0)?
        .into_iter()
        .next()
        .ok_or("No transcript to paste")?;
    let settings = ctx.settings_store.get();
    let hwnd = crate::platform::foreground_hwnd();
    tokio::task::spawn_blocking(move || {
        crate::injection::TextInjector::deliver_configured(
            &entry.final_transcript,
            &crate::settings::OutputAction::Paste,
            hwnd,
            settings.clipboard_restore_enabled,
            &settings.send_key,
            settings.paste_delay_ms,
            &settings.inject_method,
        )
        .map(|outcome| outcome.pasted)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").into()
}
#[tauri::command]
pub fn open_releases() -> Result<(), String> {
    crate::platform::open_browser("https://github.com/Lawliet2004/reflow/releases")
}
