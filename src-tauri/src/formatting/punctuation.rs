use regex::Regex;

pub struct PunctuationInferer;

impl PunctuationInferer {
    /// Replaces spoken punctuation commands like "period", "comma", "new line"
    pub fn replace_spoken_punctuation(text: &str) -> String {
        let mut out = text.to_string();

        let rules = [
            (r"(?i)\s*\b(new\s+paragraph)\b\s*", "\n\n"),
            (r"(?i)\s*\b(new\s+line)\b\s*", "\n"),
            (r"(?i)\s*\b(question\s+mark)\b", "?"),
            (r"(?i)\s*\b(exclamation\s+mark|exclamation\s+point)\b", "!"),
            (r"(?i)\s*\b(period|full\s+stop)\b", "."),
            (r"(?i)\s*\b(comma)\b", ","),
            (r"(?i)\s*\b(colon)\b", ":"),
            (r"(?i)\s*\b(semicolon)\b", ";"),
            (r"(?i)\s*\b(open\s+quote)\b", "\""),
            (r"(?i)\s*\b(close\s+quote)\b", "\""),
            (r"(?i)\s*\b(em\s+dash)\b\s*", "—"),
            (r"(?i)\b(open\s+parenthesis)\b\s*", "("),
            (r"(?i)\s*\b(close\s+parenthesis)\b", ")"),
            (r"(?i)\b(open\s+paren)\b\s*", "("),
            (r"(?i)\s*\b(close\s+paren)\b", ")"),
        ];

        for (pattern, replacement) in rules {
            if let Ok(re) = Regex::new(pattern) {
                out = re.replace_all(&out, replacement).to_string();
            }
        }

        // Clean up spaces before punctuation marks
        let space_before_punct = Regex::new(r"\s+([,\.\?\!:;])").unwrap();
        out = space_before_punct.replace_all(&out, "$1").to_string();
        let space_after_open = Regex::new(r"\(\s+").unwrap();
        out = space_after_open.replace_all(&out, "(").to_string();
        let space_before_close = Regex::new(r"\s+\)").unwrap();
        out = space_before_close.replace_all(&out, ")").to_string();

        out
    }

    /// Capitalizes the first letter of each sentence
    pub fn capitalize_sentences(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut capitalize_next = true;

        for ch in text.chars() {
            if capitalize_next && ch.is_alphabetic() {
                result.extend(ch.to_uppercase());
                capitalize_next = false;
            } else {
                result.push(ch);
                if ch == '.' || ch == '?' || ch == '!' || ch == '\n' {
                    capitalize_next = true;
                }
            }
        }

        result
    }

    /// Appends terminal punctuation (. or ?) if sentence is unpunctuated
    pub fn infer_terminal_punctuation(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        let last_char = trimmed.chars().last().unwrap_or(' ');
        if last_char == '.' || last_char == '?' || last_char == '!' || last_char == ';' || last_char == ':' {
            return trimmed.to_string();
        }

        // Check if begins or trailing clause begins with common question words
        let lower = trimmed.to_lowercase();
        let last_clause = lower.split([',', ';', '\n']).last().unwrap_or(&lower).trim();
        let is_question = lower.starts_with("what ")
            || lower.starts_with("why ")
            || lower.starts_with("how ")
            || lower.starts_with("where ")
            || lower.starts_with("when ")
            || lower.starts_with("who ")
            || lower.starts_with("can you ")
            || lower.starts_with("could you ")
            || lower.starts_with("would you ")
            || lower.starts_with("is it ")
            || lower.starts_with("are we ")
            || lower.starts_with("kya ")
            || last_clause.starts_with("what ")
            || last_clause.starts_with("why ")
            || last_clause.starts_with("how ")
            || last_clause.starts_with("where ")
            || last_clause.starts_with("when ")
            || last_clause.starts_with("who ")
            || last_clause.starts_with("can you ")
            || last_clause.starts_with("could you ")
            || last_clause.starts_with("would you ")
            || last_clause.starts_with("is it ")
            || last_clause.starts_with("are we ")
            || last_clause.starts_with("kya ");

        if is_question {
            format!("{}?", trimmed)
        } else {
            format!("{}.", trimmed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spoken_punctuation() {
        let input = "hello comma how are you today question mark new line I am good period";
        let replaced = PunctuationInferer::replace_spoken_punctuation(input);
        assert_eq!(replaced, "hello, how are you today?\nI am good.");
    }

    #[test]
    fn test_capitalize_sentences() {
        let input = "hello world. this is great! what time is it?";
        let capitalized = PunctuationInferer::capitalize_sentences(input);
        assert_eq!(capitalized, "Hello world. This is great! What time is it?");
    }

    #[test]
    fn test_infer_question_mark() {
        let input = "Can you check the deployment status";
        let inferred = PunctuationInferer::infer_terminal_punctuation(input);
        assert_eq!(inferred, "Can you check the deployment status?");
    }

    #[test]
    fn test_spoken_emdash_and_parens() {
        let input = "this em dash that open paren hello close paren";
        let replaced = PunctuationInferer::replace_spoken_punctuation(input);
        assert_eq!(replaced, "this—that (hello)");
    }

    #[test]
    fn test_spoken_parenthesis_words() {
        let input = "see open parenthesis notes close parenthesis please";
        let replaced = PunctuationInferer::replace_spoken_punctuation(input);
        assert_eq!(replaced, "see (notes) please");
    }
}
