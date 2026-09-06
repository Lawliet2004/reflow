use regex::Regex;

pub struct TextCleaner;

impl TextCleaner {
    /// Removes speech filler words such as "um", "uh", "er", "ah", "hmm".
    ///
    /// A filler is deleted together with the punctuation that only existed to
    /// set it off. Deleting the word alone leaves that punctuation stranded,
    /// which is what produced the transcripts this project actually shipped:
    /// "Uh, I need the key" became ", I need the key", and
    /// "start, uh, like this" became "start, , like this". The comma after a
    /// filler belongs to the filler, not to the sentence, so it goes with it —
    /// but a comma *before* the filler is kept, so "the key, uh, for the door"
    /// correctly keeps one comma instead of losing the clause boundary.
    pub fn remove_fillers(text: &str) -> String {
        // Fillers that carry their own trailing separator. Handled first and
        // as one unit, because matching the bare word first would strip the
        // word and leave the separator behind.
        let filler_with_sep = Regex::new(r"(?i)\b(?:um+|uh+|er+|ah+|hmm+)\s*[,;:]+\s*").unwrap();
        let mut out = filler_with_sep.replace_all(text, "").to_string();

        // Bare fillers. The leading `\s*` keeps "to ah do" from collapsing
        // into "todo".
        let bare_filler = Regex::new(r"(?i)\s*\b(?:um+|uh+|er+|ah+|hmm+)\b").unwrap();
        out = bare_filler.replace_all(&out, "").to_string();

        Self::repair_orphaned_punctuation(&out)
    }

    /// Repairs punctuation left dangling by a deletion.
    ///
    /// Split out from [`Self::remove_fillers`] because stutter removal and the
    /// backtrack pass can strand punctuation the same way, and because these
    /// rules are the part worth testing directly.
    fn repair_orphaned_punctuation(text: &str) -> String {
        let mut out = text.to_string();

        // ", ," -> ","  (a filler removed from between two clauses)
        let doubled = Regex::new(r"([,;:])(\s*[,;:])+").unwrap();
        out = doubled.replace_all(&out, "$1").to_string();

        // " ," -> ","  (space stranded ahead of punctuation)
        let space_before = Regex::new(r"\s+([,.;:!?])").unwrap();
        out = space_before.replace_all(&out, "$1").to_string();

        // ",." -> "."  (a filler removed from the end of a sentence)
        let comma_then_stop = Regex::new(r"[,;:]+([.!?])").unwrap();
        out = comma_then_stop.replace_all(&out, "$1").to_string();

        // ". , Today" -> ". Today"  (a filler opened the sentence)
        let stop_then_comma = Regex::new(r"([.!?])\s*[,;:]+\s*").unwrap();
        out = stop_then_comma.replace_all(&out, "$1 ").to_string();

        // Leading and trailing separators have nothing left to separate.
        let leading = Regex::new(r"^[\s,;:]+").unwrap();
        out = leading.replace(&out, "").to_string();
        let trailing = Regex::new(r"[\s,;:]+$").unwrap();
        out = trailing.replace(&out, "").to_string();

        let space_re = Regex::new(r"\s+").unwrap();
        space_re.replace_all(&out, " ").trim().to_string()
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

    /// Every case here is a verbatim `raw_transcript` -> `final_transcript`
    /// pair taken from a real Reflow history database, where deleting the
    /// filler word left its punctuation stranded.
    #[test]
    fn filler_removal_does_not_strand_punctuation() {
        let cases = [
            // was ", I need to work for like."
            ("Uh, I need to work for like.", "I need to work for like."),
            // was "start, , like doing the task"
            (
                "Can you review the code for me and start, uh, like doing the task of code review?",
                "Can you review the code for me and start, like doing the task of code review?",
            ),
            // was "I need the key, , for opening the door."
            (
                "Please give me the key. I need the key, uh, for opening the door.",
                "Please give me the key. I need the key, for opening the door.",
            ),
            // was ", Can you bring me tea?"
            ("Ah, can you bring me tea?", "can you bring me tea?"),
            // was "things. , Today I am gonna". Re-capitalization is a later
            // pipeline stage (`capitalize_sentences`), so this function is
            // only responsible for not leaving the comma behind.
            (
                "I like to ah do some bizarre things. Ah, today I am gonna go swim.",
                "I like to do some bizarre things. today I am gonna go swim.",
            ),
            // was "Like, , they meet together"
            (
                "It started in a castle. Like, ah, they meet together downtown.",
                "It started in a castle. Like, they meet together downtown.",
            ),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                TextCleaner::remove_fillers(raw),
                expected,
                "filler removal mangled punctuation for {raw:?}"
            );
        }
    }

    /// A filler at the very end must not leave a dangling comma either.
    #[test]
    fn filler_at_a_boundary_leaves_no_dangling_separator() {
        assert_eq!(TextCleaner::remove_fillers("So, um"), "So");
        assert_eq!(TextCleaner::remove_fillers("I think, uh."), "I think.");
        assert_eq!(TextCleaner::remove_fillers("um"), "");
        assert_eq!(
            TextCleaner::remove_fillers("Um, uh, I mean yes."),
            "I mean yes."
        );
    }

    /// Filler removal must not run words together, and must leave text that
    /// contains no fillers completely untouched.
    #[test]
    fn filler_removal_preserves_surrounding_words() {
        assert_eq!(
            TextCleaner::remove_fillers("I like to ah do some things"),
            "I like to do some things"
        );
        let clean = "The quick brown fox jumps over the lazy dog.";
        assert_eq!(TextCleaner::remove_fillers(clean), clean);
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
