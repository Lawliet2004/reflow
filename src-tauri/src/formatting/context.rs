pub enum DictationMode {
    Normal,
    Coding,
    Email,
    Chat,
    Notes,
}

impl DictationMode {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "coding" => Self::Coding,
            "email" => Self::Email,
            "chat" => Self::Chat,
            "notes" => Self::Notes,
            _ => Self::Normal,
        }
    }
}

pub struct ContextFormatter;

impl ContextFormatter {
    pub fn format(text: &str, mode: DictationMode) -> String {
        match mode {
            DictationMode::Coding => {
                // In coding mode, do not append trailing punctuation if it looks like syntax, commands, or identifiers
                let trimmed = text.trim();
                let lower = trimmed.to_lowercase();
                if lower.starts_with("git ")
                    || lower.starts_with("npm ")
                    || lower.starts_with("cargo ")
                    || lower.starts_with("docker ")
                    || lower.starts_with("python ")
                    || lower.starts_with("const ")
                    || lower.starts_with("let ")
                    || lower.starts_with("fn ")
                    || lower.starts_with("pub ")
                    || lower.starts_with("def ")
                {
                    return trimmed.to_string();
                }
                trimmed.to_string()
            }
            DictationMode::Chat => text.trim().to_string(),
            DictationMode::Email => text.trim().to_string(),
            DictationMode::Notes => text.trim().to_string(),
            DictationMode::Normal => text.trim().to_string(),
        }
    }
}
