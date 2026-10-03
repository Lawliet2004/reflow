/// Clipboard restoration policy, separated from OS clipboard access so races
/// can be reproduced without changing the user's actual clipboard.
pub(super) struct RestoreChain<S> {
    pending: Option<Pending<S>>,
}

impl<S> Default for RestoreChain<S> {
    fn default() -> Self {
        Self { pending: None }
    }
}

struct Pending<S> {
    original: S,
    inserted: Option<String>,
    sequence: Option<u32>,
    generation: u64,
}

impl<S: Clone> RestoreChain<S> {
    pub(super) fn original_for_injection(
        &self,
        observed: S,
        text: Option<&str>,
        sequence: Option<u32>,
    ) -> S {
        if let Some(pending) = &self.pending {
            if pending.unchanged(text, sequence) {
                return pending.original.clone();
            }
        }
        observed
    }

    pub(super) fn mark_injected(
        &mut self,
        original: S,
        inserted: String,
        sequence: Option<u32>,
        generation: u64,
    ) {
        self.pending = Some(Pending {
            original,
            inserted: Some(inserted),
            sequence,
            generation,
        });
    }

    pub(super) fn candidate_if_unchanged(
        &mut self,
        generation: u64,
        text: Option<&str>,
        sequence: Option<u32>,
    ) -> Option<S> {
        let pending = self.pending.as_ref()?;
        if pending.generation != generation {
            return None;
        }
        let unchanged = pending.unchanged(text, sequence);
        if unchanged {
            Some(pending.original.clone())
        } else {
            self.pending = None;
            None
        }
    }

    pub(super) fn complete_restore(&mut self, generation: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation == generation)
        {
            self.pending = None;
        }
    }

    pub(super) fn note_failed_restore(
        &mut self,
        generation: u64,
        text: Option<String>,
        sequence: Option<u32>,
    ) {
        if let Some(pending) = self
            .pending
            .as_mut()
            .filter(|pending| pending.generation == generation)
        {
            // A failed native write can have changed some clipboard formats.
            // Retain the full original and record only our resulting contents.
            pending.inserted = text;
            pending.sequence = sequence;
        }
    }

    pub(super) fn generation(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| pending.generation)
    }

    pub(super) fn discard(&mut self) {
        self.pending = None;
    }
}

impl<S> Pending<S> {
    fn unchanged(&self, text: Option<&str>, sequence: Option<u32>) -> bool {
        match self.sequence {
            Some(expected) => sequence == Some(expected),
            None => self
                .inserted
                .as_deref()
                .is_some_and(|inserted| text == Some(inserted)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_injections_restore_the_original_image_and_html() {
        let original = vec![(8, vec![1, 2, 3]), (49_152, b"<b>original</b>".to_vec())];
        let temporary = vec![(13, b"first transcript".to_vec())];
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected(original.clone(), "first transcript".into(), Some(10), 1);
        let retained = chain.original_for_injection(temporary, Some("first transcript"), Some(10));
        chain.mark_injected(retained, "second transcript".into(), Some(11), 2);
        assert_eq!(
            chain.candidate_if_unchanged(2, Some("second transcript"), Some(11)),
            Some(original)
        );
    }

    #[test]
    fn copying_the_same_text_cancels_restore_using_the_clipboard_sequence() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("original", "transcript".into(), Some(10), 1);
        assert_eq!(
            chain.candidate_if_unchanged(1, Some("transcript"), Some(11)),
            None
        );
    }

    #[test]
    fn an_old_timer_cannot_discard_a_new_injection() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("original", "second".into(), Some(11), 2);
        assert_eq!(
            chain.candidate_if_unchanged(1, Some("second"), Some(11)),
            None
        );
        assert_eq!(
            chain.candidate_if_unchanged(2, Some("second"), Some(11)),
            Some("original")
        );
    }

    #[test]
    fn a_new_user_copy_becomes_the_next_original() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("old original", "first".into(), Some(10), 1);
        let new_original =
            chain.original_for_injection("new user copy", Some("new user copy"), Some(11));
        chain.mark_injected(new_original, "second".into(), Some(12), 2);
        assert_eq!(
            chain.candidate_if_unchanged(2, Some("second"), Some(12)),
            Some("new user copy")
        );
    }

    #[test]
    fn a_failed_restore_keeps_the_original_for_a_safe_retry() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("original image and html", "transcript".into(), Some(10), 1);
        assert_eq!(
            chain.candidate_if_unchanged(1, Some("transcript"), Some(10)),
            Some("original image and html")
        );
        chain.note_failed_restore(1, None, Some(11));
        assert_eq!(
            chain.candidate_if_unchanged(1, None, Some(11)),
            Some("original image and html")
        );
        chain.complete_restore(1);
        assert_eq!(chain.generation(), None);
    }

    #[test]
    fn a_user_copy_after_a_failed_restore_prevents_retry() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("original", "transcript".into(), Some(10), 1);
        chain.note_failed_restore(1, None, Some(11));
        assert_eq!(
            chain.candidate_if_unchanged(1, Some("user copy"), Some(12)),
            None
        );
    }

    #[test]
    fn portable_restore_requires_identical_readable_text() {
        let mut chain = RestoreChain { pending: None };
        chain.mark_injected("original", "transcript".into(), None, 1);
        assert_eq!(
            chain.candidate_if_unchanged(1, Some("transcript"), None),
            Some("original")
        );
        chain.note_failed_restore(1, None, None);
        assert_eq!(chain.candidate_if_unchanged(1, None, None), None);
    }
}
