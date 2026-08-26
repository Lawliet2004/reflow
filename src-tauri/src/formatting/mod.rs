pub mod backtrack;
pub mod cleaner;
pub mod context;
pub mod hedge;
pub mod normalizers;
pub mod punctuation;
pub mod replacements;

pub use backtrack::apply_backtrack;
pub use cleaner::TextCleaner;
pub use context::{ContextFormatter, DictationMode};
pub use hedge::apply_hedge;
pub use normalizers::apply_normalizers;
pub use punctuation::PunctuationInferer;
pub use replacements::{CustomReplacements, ReplacementRule};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupLevel {
    Raw,
    Light,
    Medium,
    High,
}

impl CleanupLevel {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "raw" => Self::Raw,
            "light" | "smart" => Self::Light,
            "medium" | "flow" => Self::Medium,
            "high" => Self::High,
            _ => Self::Light,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceStyle {
    Faithful,
    Neutral,
    Decisive,
    Email,
    Chat,
}

impl VoiceStyle {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "faithful" => Self::Faithful,
            "neutral" => Self::Neutral,
            "decisive" => Self::Decisive,
            "email" => Self::Email,
            "chat" => Self::Chat,
            _ => Self::Neutral,
        }
    }
}

pub struct FormatRequest<'a> {
    pub cleanup_level: CleanupLevel,
    pub dictation_mode: &'a str,
    pub style: VoiceStyle,
    pub filler_removal_enabled: bool,
    pub spoken_punctuation_enabled: bool,
    pub custom_replacements: &'a CustomReplacements,
    pub focused_process: Option<&'a str>,
}

/// Keep the existing signature. `"raw"` trims only; `"smart"`/`"flow"` run the Light
/// base pipeline (`CleanupLevel::parse` maps `"smart"` → Light and `"flow"` → Medium,
/// and Medium shares the Light steps without hedge).
pub fn format_transcript(
    raw: &str,
    processing_mode: &str,
    dictation_mode: &str,
    filler_removal_enabled: bool,
    spoken_punctuation_enabled: bool,
    custom_replacements: &CustomReplacements,
) -> String {
    format_transcript_ex(
        raw,
        FormatRequest {
            cleanup_level: CleanupLevel::parse(processing_mode),
            dictation_mode,
            style: VoiceStyle::Faithful,
            filler_removal_enabled,
            spoken_punctuation_enabled,
            custom_replacements,
            focused_process: None,
        },
    )
}

pub fn format_transcript_ex(raw: &str, req: FormatRequest<'_>) -> String {
    if req.cleanup_level == CleanupLevel::Raw {
        return raw.trim().to_string();
    }

    // Light base (Light / Medium / High)
    let mut text = if req.spoken_punctuation_enabled {
        PunctuationInferer::replace_spoken_punctuation(raw)
    } else {
        raw.to_string()
    };

    text = TextCleaner::clean(&text, req.filler_removal_enabled);
    text = apply_backtrack(&text);
    text = req.custom_replacements.apply(&text);
    text = apply_normalizers(&text, req.dictation_mode);
    text = PunctuationInferer::capitalize_sentences(&text);

    let d_mode = DictationMode::from_str(req.dictation_mode);
    let coding = matches!(d_mode, DictationMode::Coding);

    if coding {
        text = ContextFormatter::format(&text, d_mode);
    } else {
        text = PunctuationInferer::infer_terminal_punctuation(&text);
    }

    if !coding && should_drop_chat_period(req.style, req.focused_process, &text) {
        text = strip_trailing_period(&text);
    }

    if !coding {
        text = apply_hedge(&text, req.cleanup_level, req.style, req.dictation_mode);
        if req.cleanup_level == CleanupLevel::High && req.style == VoiceStyle::Decisive {
            text = PunctuationInferer::capitalize_sentences(&text);
        }
    }

    text
}

fn should_drop_chat_period(style: VoiceStyle, process: Option<&str>, text: &str) -> bool {
    let chat_app = process
        .map(|p| p.to_lowercase())
        .map(|p| {
            p.contains("discord")
                || p.contains("slack")
                || p.contains("telegram")
                || p.contains("whatsapp")
        })
        .unwrap_or(false);
    if style != VoiceStyle::Chat && !chat_app {
        return false;
    }
    text.split_whitespace().count() < 12
}

fn strip_trailing_period(text: &str) -> String {
    let t = text.trim();
    if t.ends_with("...") {
        return t.to_string();
    }
    if t.ends_with('.') {
        t.trim_end_matches('.').to_string()
    } else {
        t.to_string()
    }
}

/// Unique, order-preserving ASR hotwords. Prefers non-empty `preferred_spelling`,
/// then replacement afters. Dedup is case-insensitive; empty strings are skipped.
pub fn assemble_asr_vocabulary(
    dictionary_terms: &[(String, String)],
    replacement_afters: &[String],
    max_terms: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    let mut push = |value: &str| {
        if out.len() >= max_terms {
            return;
        }
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return;
        }
        let key = trimmed.to_lowercase();
        if seen.insert(key) {
            out.push(trimmed.to_string());
        }
    };

    for (term, preferred) in dictionary_terms {
        if !preferred.trim().is_empty() {
            push(preferred);
        } else {
            push(term);
        }
    }
    for after in replacement_afters {
        push(after);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_rules() -> CustomReplacements {
        CustomReplacements::new(Vec::new())
    }

    fn fmt(raw: &str, level: CleanupLevel, mode: &str, style: VoiceStyle) -> String {
        let rules = empty_rules();
        format_transcript_ex(
            raw,
            FormatRequest {
                cleanup_level: level,
                dictation_mode: mode,
                style,
                filler_removal_enabled: true,
                spoken_punctuation_enabled: true,
                custom_replacements: &rules,
                focused_process: None,
            },
        )
    }

    #[test]
    fn test_full_pipeline() {
        let rules = CustomReplacements::new(CustomReplacements::default_rules());
        let raw = "um I pushed the commit to git hub using vs code and type script comma can you review it";
        let formatted = format_transcript(raw, "smart", "normal", true, true, &rules);
        assert_eq!(
            formatted,
            "I pushed the commit to GitHub using VS Code and TypeScript, can you review it?"
        );
    }

    #[test]
    fn smart_maps_to_light() {
        assert_eq!(CleanupLevel::parse("smart"), CleanupLevel::Light);
        assert_eq!(CleanupLevel::parse("light"), CleanupLevel::Light);
        assert_eq!(CleanupLevel::parse("flow"), CleanupLevel::Medium);
        assert_eq!(CleanupLevel::parse("raw"), CleanupLevel::Raw);
        assert_eq!(CleanupLevel::parse("high"), CleanupLevel::High);
    }

    #[test]
    fn raw_is_trim_only() {
        let out = fmt("  um I want to drink tea  ", CleanupLevel::Raw, "normal", VoiceStyle::Decisive);
        assert_eq!(out, "um I want to drink tea");
        assert!(out.contains("um"), "{out}");
    }

    #[test]
    fn light_keeps_want_and_drops_fillers() {
        let out = fmt("um I want to drink tea", CleanupLevel::Light, "normal", VoiceStyle::Decisive);
        assert_eq!(out, "I want to drink tea.");
        assert!(out.contains("want"), "{out}");
        assert!(!out.to_lowercase().contains("will"), "{out}");
        assert!(!out.to_lowercase().contains("um"), "{out}");
        assert!(out.ends_with('.'), "{out}");
    }

    #[test]
    fn high_decisive_rewrites_i_want_to() {
        let out = fmt("I want to drink tea", CleanupLevel::High, "normal", VoiceStyle::Decisive);
        assert_eq!(out, "I will drink tea.");
    }

    #[test]
    fn high_decisive_does_not_flip_negation() {
        let out = fmt("I don't want this", CleanupLevel::High, "normal", VoiceStyle::Decisive);
        let lower = out.to_lowercase();
        assert!(
            lower.contains("don't want") || lower.contains("do not want"),
            "{out}"
        );
        assert!(!lower.contains("will"), "{out}");
    }

    #[test]
    fn high_decisive_leaves_want_you_to() {
        let out = fmt(
            "I want you to review this",
            CleanupLevel::High,
            "normal",
            VoiceStyle::Decisive,
        );
        assert!(out.to_lowercase().contains("want you"), "{out}");
        assert!(!out.to_lowercase().contains("will you"), "{out}");
    }

    #[test]
    fn light_applies_numeric_backtrack() {
        let out = fmt(
            "let's meet at 5 actually 6",
            CleanupLevel::Light,
            "normal",
            VoiceStyle::Neutral,
        );
        let lower = out.to_lowercase();
        assert!(lower.contains("meet at 6"), "{out}");
        assert!(!lower.contains('5'), "{out}");
        assert!(!lower.contains("actually"), "{out}");
    }

    #[test]
    fn light_keeps_i_actually_enjoyed() {
        let out = fmt(
            "I actually enjoyed the movie",
            CleanupLevel::Light,
            "normal",
            VoiceStyle::Neutral,
        );
        assert!(out.to_lowercase().contains("actually enjoyed"), "{out}");
    }

    #[test]
    fn coding_skips_hedge_on_high_decisive() {
        let out = fmt(
            "I want to drink tea",
            CleanupLevel::High,
            "coding",
            VoiceStyle::Decisive,
        );
        assert!(out.to_lowercase().contains("want to"), "{out}");
        assert!(!out.to_lowercase().contains("will drink"), "{out}");
    }

    #[test]
    fn medium_does_not_apply_hedge() {
        let out = fmt("I want to drink tea", CleanupLevel::Medium, "normal", VoiceStyle::Decisive);
        assert!(out.to_lowercase().contains("want to"), "{out}");
        assert!(!out.to_lowercase().contains("will drink"), "{out}");
    }

    #[test]
    fn chat_style_strips_trailing_period_on_short_text() {
        let out = fmt("hello there", CleanupLevel::Light, "normal", VoiceStyle::Chat);
        assert!(!out.ends_with('.'), "{out}");
        assert!(out.to_lowercase().contains("hello there"), "{out}");
    }

    #[test]
    fn assemble_asr_vocabulary_includes_terms() {
        let terms = vec![
            ("qwen".into(), "Qwen".into()),
            ("tauri".into(), "Tauri".into()),
            ("reflow".into(), "".into()),
        ];
        let afters = vec!["GitHub".into(), "Qwen".into(), "".into()];
        let vocab = assemble_asr_vocabulary(&terms, &afters, 60);
        assert!(vocab.contains(&"Qwen".to_string()), "{vocab:?}");
        assert!(vocab.contains(&"Tauri".to_string()), "{vocab:?}");
        assert!(vocab.contains(&"reflow".to_string()), "{vocab:?}");
        assert!(vocab.contains(&"GitHub".to_string()), "{vocab:?}");
        assert_eq!(vocab, vec!["Qwen", "Tauri", "reflow", "GitHub"]);
    }

    #[test]
    fn assemble_asr_vocabulary_caps_and_dedups() {
        let terms = vec![("qwen".into(), "Qwen".into())];
        let afters = vec!["qwen".into(), "GitHub".into()];
        let vocab = assemble_asr_vocabulary(&terms, &afters, 1);
        assert_eq!(vocab, vec!["Qwen"]);
    }
}
