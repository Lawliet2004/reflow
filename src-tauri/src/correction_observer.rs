//! Observe only the recently inserted dictation, never arbitrary desktop typing.
use crate::{context::AppContext, dory::DoryEvent};

pub const MAX_FIELD_CHARS: usize = 16_384;

/// Fixed surrounding text anchors the pasted span. If the surrounding field
/// changes, it is no longer safe to attribute its edits to this dictation.
pub struct TrackedDictation {
    prefix: String,
    suffix: String,
    pub text: String,
}

impl TrackedDictation {
    pub fn new(field: &str, inserted: &str) -> Option<Self> {
        if inserted.trim().is_empty() || field.chars().count() > MAX_FIELD_CHARS {
            return None;
        }
        let start = field.find(inserted)?;
        // Include overlapping matches ("aaa" in "aaaa") in the ambiguity check.
        let next = start + inserted.chars().next()?.len_utf8();
        if field[next..].contains(inserted) {
            return None;
        }
        Some(Self {
            prefix: field[..start].into(),
            suffix: field[start + inserted.len()..].into(),
            text: inserted.into(),
        })
    }

    pub fn extract(&self, field: &str) -> Option<String> {
        if field.chars().count() > MAX_FIELD_CHARS {
            return None;
        }
        let text = field
            .strip_prefix(&self.prefix)?
            .strip_suffix(&self.suffix)?;
        (!text.trim().is_empty() && text.chars().count() <= MAX_FIELD_CHARS)
            .then(|| text.to_owned())
    }
}

/// Compare-and-update prevents a late observer from replacing a newer manual
/// history edit. The user's raw ASR transcript and cleanup result are preserved.
pub fn record_observed_edit(
    ctx: &AppContext,
    history_id: Option<&str>,
    before: &str,
    after: &str,
) -> Result<bool, String> {
    let _guard = ctx.settings_operation.lock();
    if !ctx.settings_store.get().auto_learn_dictionary
        || before == after
        || after.trim().is_empty()
        || after.chars().count() > MAX_FIELD_CHARS
    {
        return Ok(false);
    }
    if let Some(id) = history_id {
        if !ctx
            .history_store
            .update_observed_dictation(id, before, after)?
        {
            return Ok(false);
        }
        if let Some(updated) = ctx.history_store.get_entry(id)? {
            ctx.bus.emit(DoryEvent::HistoryUpdated(Box::new(updated)));
        }
    }
    if let Some(updated) = ctx
        .settings_store
        .learn_dictionary_corrections(before, after)?
    {
        ctx.bus
            .emit(DoryEvent::DictionaryChanged(Box::new(updated)));
    }
    Ok(true)
}

#[cfg(windows)]
#[path = "platform/windows_corrections.rs"]
mod windows;

pub fn start(ctx: AppContext, hwnd: isize, inserted: String, history_id: Option<String>) {
    #[cfg(windows)]
    windows::start(ctx, hwnd, inserted, history_id);
    #[cfg(not(windows))]
    let _ = (ctx, hwnd, inserted, history_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_extracts_the_pasted_span() {
        let tracked =
            TrackedDictation::new("Earlier. Use type script. Later.", "Use type script.").unwrap();
        assert_eq!(
            tracked.extract("Earlier. Use TypeScript. Later."),
            Some("Use TypeScript.".into())
        );
        assert_eq!(tracked.extract("Different. Use TypeScript. Later."), None);
        assert_eq!(tracked.extract(""), None);
    }

    #[test]
    fn refuses_ambiguous_pastes_and_oversized_fields() {
        assert!(TrackedDictation::new("hello hello", "hello").is_none());
        assert!(TrackedDictation::new("aaaa", "aaa").is_none());
        assert!(TrackedDictation::new("hello", "").is_none());
        assert!(TrackedDictation::new(&"a".repeat(MAX_FIELD_CHARS + 1), "a").is_none());
    }
}
