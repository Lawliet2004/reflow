use regex::Regex;

pub struct TextCleaner;

impl TextCleaner {
    /// Removes speech filler words such as "um", "uh", "er", "ah", "hmm"
    pub fn remove_fillers(text: &str) -> String {
        // Match filler words at word boundaries (case insensitive)
        let filler_re = Regex::new(r"(?i)\b(um+|uh+|er+|ah+|hmm+)\b").unwrap();
        let cleaned = filler_re.replace_all(text, "");

        // Collapse multiple spaces
        let space_re = Regex::new(r"\s+").unwrap();
        space_re.replace_all(&cleaned, " ").trim().to_string()
    }

    /// Removes immediate stuttering word duplicates ("the the" -> "the")
    pub fn remove_duplicates(text: &str) -> String {
        let words: Vec<&str> = text.split_whitespace().collect();
        if words.is_empty() {
            return String::new();
        }

        let mut result = Vec::new();
        let mut prev_word = "";

        for word in words {
            let clean_w = word.to_lowercase();
            let clean_prev = prev_word.to_lowercase();

            if clean_w != clean_prev || clean_w.len() <= 1 {
                result.push(word);
            }
            prev_word = word;
        }

        result.join(" ")
    }

    /// Full cleaning pipeline
    pub fn clean(text: &str, enable_filler_removal: bool) -> String {
        let mut out = text.to_string();
        if enable_filler_removal {
            out = Self::remove_fillers(&out);
        }
        out = Self::remove_duplicates(&out);
        out.trim().to_string()
    }

    /// Apply dictionary preferred spellings (whole-word, case-insensitive).
    pub fn apply_glossary(text: &str, terms: &[(String, String)]) -> String {
        let mut out = text.to_string();
        for (term, preferred) in terms {
            let from = term.trim();
            let to = preferred.trim();
            if from.is_empty() || to.is_empty() || from == to {
                continue;
            }
            let escaped = regex::escape(from);
            if let Ok(re) = Regex::new(&format!(r"(?i)\b{}\b", escaped)) {
                out = re.replace_all(&out, to).to_string();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remove_fillers() {
        let input = "um I think we should uh go to the office ah tomorrow";
        let cleaned = TextCleaner::remove_fillers(input);
        assert_eq!(cleaned, "I think we should go to the office tomorrow");
    }

    #[test]
    fn test_remove_duplicates() {
        let input = "we we need to to fix the the bug";
        let cleaned = TextCleaner::remove_duplicates(input);
        assert_eq!(cleaned, "we need to fix the bug");
    }

    #[test]
    fn test_glossary_preferred_spelling() {
        let terms = vec![
            ("tauri".into(), "Tauri".into()),
            ("qwen".into(), "Qwen".into()),
        ];
        let out = TextCleaner::apply_glossary("ship this tauri app with qwen", &terms);
        assert_eq!(out, "ship this Tauri app with Qwen");
    }
}
