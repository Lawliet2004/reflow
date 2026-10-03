//! Model output is a proposal, never a shell command. UI activation revalidates it.
use crate::context::AppContext;
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantTool {
    Search { query: String },
    Note { text: String },
}

pub fn parse_tool(response: &str) -> Option<AssistantTool> {
    let text = response.trim();
    // Exactly one command line. No wrappers, code fences, or second command.
    if text.contains(['\n', '\r']) || text.chars().any(char::is_control) {
        return None;
    }
    if let Some(query) = text.strip_prefix("SEARCH: ") {
        let query = query.trim();
        if !query.is_empty() && query.chars().count() <= 500 {
            return Some(AssistantTool::Search {
                query: query.into(),
            });
        }
    } else if let Some(note) = text.strip_prefix("NOTE: ") {
        let note = note.trim();
        if !note.is_empty() && note.chars().count() <= 4000 {
            return Some(AssistantTool::Note { text: note.into() });
        }
    }
    None
}

pub fn search_url(query: &str) -> Result<reqwest::Url, String> {
    let mut url =
        reqwest::Url::parse("https://www.google.com/search").map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("q", query);
    Ok(url)
}

#[tauri::command]
pub fn execute_assistant_tool(
    response: String,
    ctx: State<'_, AppContext>,
) -> Result<String, String> {
    match parse_tool(&response).ok_or("Assistant proposal is not a supported command")? {
        AssistantTool::Search { query } => {
            crate::platform::open_browser(search_url(&query)?.as_str())?;
            Ok("Search opened in your browser".into())
        }
        AssistantTool::Note { text } => {
            ctx.history_store
                .insert_entry(&crate::expansion_commands::note_entry(text))?;
            Ok("Saved in Notes".into())
        }
    }
}
