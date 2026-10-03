use crate::settings::Snippet;

/// Expand once on final output, preserving the stored text byte-for-byte.
/// The longest trigger wins at a position; inserted text is never rescanned.
pub fn apply_snippets(text: &str, snippets: &[Snippet]) -> String {
    let mut active: Vec<_> = snippets
        .iter()
        .filter(|s| s.enabled && !s.trigger.trim().is_empty())
        .collect();
    active.sort_by_key(|s| std::cmp::Reverse(s.trigger.len()));
    let mut output = String::with_capacity(text.len());
    let mut offset = 0;
    while offset < text.len() {
        let boundary = offset == 0
            || text[..offset]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let matched = boundary
            .then(|| {
                active.iter().find_map(|s| {
                    let trigger = s.trigger.trim();
                    let end = offset + trigger.len();
                    let candidate = text.get(offset..end)?;
                    (candidate.to_lowercase() == trigger.to_lowercase()
                        && text[end..]
                            .chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_'))
                    .then_some((*s, end))
                })
            })
            .flatten();
        if let Some((snippet, end)) = matched {
            output.push_str(&snippet.expansion);
            offset = end;
        } else {
            let c = text[offset..].chars().next().unwrap();
            output.push(c);
            offset += c.len_utf8();
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snippet(trigger: &str, expansion: &str) -> Snippet {
        Snippet {
            id: "1".into(),
            trigger: trigger.into(),
            expansion: expansion.into(),
            enabled: true,
        }
    }
    #[test]
    fn expands_phrases_and_preserves_casing_and_newlines() {
        let snippets = vec![snippet("insert my address", "42 rue de Paris\nFRANCE")];
        assert_eq!(
            apply_snippets("Send to INSERT MY ADDRESS, please.", &snippets),
            "Send to 42 rue de Paris\nFRANCE, please."
        );
        assert_eq!(
            apply_snippets("reinsert my address", &snippets),
            "reinsert my address"
        );
    }
    #[test]
    fn disabled_and_nested_expansions_are_not_applied() {
        let mut disabled = snippet("bye", "goodbye");
        disabled.enabled = false;
        assert_eq!(
            apply_snippets("hi bye", &[snippet("hi", "bye"), disabled]),
            "bye bye"
        );
    }
}
