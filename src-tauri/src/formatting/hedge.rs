use regex::Regex;

use super::{CleanupLevel, PunctuationInferer, VoiceStyle};

fn clause_has_negation(s: &str) -> bool {
    let lower = format!(" {} ", s.to_lowercase());
    lower.contains("n't")
        || lower.contains("n’t")
        || lower.contains(" not ")
        || lower.contains(" never ")
        || lower.contains(" don't ")
        || lower.contains(" don’t ")
        || lower.contains(" do not ")
        || lower.contains(" no ")
}

/// High+decisive only: speaker-as-agent hedges. Never flips negation.
pub fn apply_hedge(
    text: &str,
    level: CleanupLevel,
    style: VoiceStyle,
    dictation_mode: &str,
) -> String {
    if level != CleanupLevel::High
        || style != VoiceStyle::Decisive
        || dictation_mode.eq_ignore_ascii_case("coding")
    {
        return text.to_string();
    }

    let want_to = Regex::new(r"(?i)\bI want to ([A-Za-z']+)\b").expect("want-to regex");
    let would_like = Regex::new(r"(?i)\bI would like to ([A-Za-z']+)\b").expect("would-like regex");
    let id_like = Regex::new(r"(?i)\bI['’]d like to ([A-Za-z']+)\b").expect("id-like regex");
    let maybe_we = Regex::new(r"(?i)\bmaybe we should\b").expect("maybe-we regex");

    let rewrite_clause = |clause: &str| -> String {
        if clause_has_negation(clause) {
            return clause.to_string();
        }
        let mut c = maybe_we.replace_all(clause, "We should").to_string();
        c = would_like.replace_all(&c, "I will $1").to_string();
        c = id_like.replace_all(&c, "I will $1").to_string();
        c = want_to.replace_all(&c, "I will $1").to_string();
        c
    };

    let mut pieces: Vec<String> = Vec::new();
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        if ch == '.' || ch == '?' || ch == '!' || ch == ';' || ch == '\n' {
            pieces.push(rewrite_clause(&text[start..i]));
            pieces.push(ch.to_string());
            start = i + ch.len_utf8();
        }
    }
    if start < text.len() || pieces.is_empty() {
        pieces.push(rewrite_clause(&text[start..]));
    }
    let joined = pieces.concat();
    PunctuationInferer::capitalize_sentences(&joined)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn want_to_becomes_will_when_high_decisive() {
        let out = apply_hedge(
            "I want to drink tea",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I will drink tea");
    }

    #[test]
    fn would_like_to_becomes_will() {
        let out = apply_hedge(
            "I would like to drink tea",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I will drink tea");
    }

    #[test]
    fn maybe_we_should_drops_maybe() {
        let out = apply_hedge(
            "maybe we should push this",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "We should push this");
    }

    #[test]
    fn light_does_not_hedge() {
        let out = apply_hedge(
            "I want to drink tea",
            CleanupLevel::Light,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I want to drink tea");
    }

    #[test]
    fn medium_does_not_hedge() {
        let out = apply_hedge(
            "I want to drink tea",
            CleanupLevel::Medium,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I want to drink tea");
    }

    #[test]
    fn negation_is_never_flipped() {
        let out = apply_hedge(
            "I don't want this",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I don't want this");
    }

    #[test]
    fn want_you_to_is_unchanged() {
        let out = apply_hedge(
            "I want you to review this",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I want you to review this");
    }

    #[test]
    fn want_this_to_is_unchanged() {
        let out = apply_hedge(
            "I want this to work",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "normal",
        );
        assert_eq!(out, "I want this to work");
    }

    #[test]
    fn coding_skips_hedge() {
        let out = apply_hedge(
            "I want to drink tea",
            CleanupLevel::High,
            VoiceStyle::Decisive,
            "coding",
        );
        assert_eq!(out, "I want to drink tea");
    }

    #[test]
    fn clause_has_negation_detects_markers() {
        assert!(clause_has_negation("I don't want this"));
        assert!(clause_has_negation("I do not want this"));
        assert!(clause_has_negation("I never want this"));
        assert!(clause_has_negation("I want no tea"));
        assert!(!clause_has_negation("I want to drink tea"));
    }
}
