use regex::Regex;

pub struct PunctuationInferer;

impl PunctuationInferer {
    /// Replaces spoken punctuation commands like "period", "comma", "new line"
    pub fn replace_spoken_punctuation(text: &str) -> String {
        let mut out = text.to_string();

        let rules = [
            (r"(?i)\s*\b(new\s+paragraph)\b\s*", "\n\n"),
            (r"(?i)\s*\b(new\s+line)\b\s*", "\n"),
            (r"(?i)\s*\b(question\s+mark)\b\.?", "?"),
            (
                r"(?i)\s*\b(exclamation\s+mark|exclamation\s+point)\b\.?",
                "!",
            ),
            (r"(?i)\s*\b(period|full\s+stop)\b\.?", "."),
            (r"(?i)\s*\b(comma)\b\.?", ","),
            (r"(?i)\s*\b(semicolon|semi\s+colon)\b\.?", ";"),
            (r"(?i)\s*\b(colon)\b\.?", ":"),
            (r"(?i)\bopen\s+quote\b\s*|\s*\bclose\s+quote\b", "\""),
            (r"(?i)\s*\b(em\s+dash)\b\s*", "—"),
            (r"(?i)\b(open\s+bracket)\b\s*", "["),
            (r"(?i)\s*\b(close\s+bracket)\b", "]"),
            (r"(?i)\b(smiley)\b", ":)"),
            (r"(?i)\b(open\s+parenthesis)\b\s*", "("),
            (r"(?i)\s*\b(close\s+parenthesis)\b", ")"),
            (r"(?i)\b(open\s+paren)\b\s*", "("),
            (r"(?i)\s*\b(close\s+paren)\b", ")"),
        ];

        for (pattern, replacement) in rules {
            if let Ok(re) = Regex::new(pattern) {
                let literal = quoted_spans(&out);
                out = re
                    .replace_all(&out, |capture: &regex::Captures<'_>| {
                        let found = capture.get(0).unwrap();
                        if literal
                            .iter()
                            .any(|(start, end)| found.start() < *end && found.end() > *start)
                            || literal_punctuation_name(&out, found.start(), found.end())
                        {
                            found.as_str().to_owned()
                        } else {
                            replacement.to_owned()
                        }
                    })
                    .into_owned();
            }
        }

        // Clean up spaces before punctuation marks
        let space_before_punct = Regex::new(r"\s+([,\.\?\!;]|:(?:[^)]|$))").unwrap();
        out = space_before_punct.replace_all(&out, "$1").to_string();
        let space_after_open = Regex::new(r"\(\s+").unwrap();
        out = space_after_open.replace_all(&out, "(").to_string();
        let space_before_close = Regex::new(r"\s+\)").unwrap();
        out = space_before_close.replace_all(&out, ")").to_string();

        out
    }

    /// Capitalizes the first letter of each sentence
    pub fn capitalize_sentences(text: &str) -> String {
        Self::capitalize_sentences_with_first(text, true)
    }

    pub fn capitalize_sentences_with_first(text: &str, first: bool) -> String {
        let mut result = String::with_capacity(text.len());
        let mut capitalize_next = first;

        for (index, ch) in text.char_indices() {
            if capitalize_next && ch.is_alphabetic() {
                let token = text[index..].split_whitespace().next().unwrap_or_default();
                let dotted_parts: Vec<_> =
                    token.split('.').filter(|part| !part.is_empty()).collect();
                let technical = token.contains(['/', '@', '_', '\\'])
                    || (dotted_parts.len() > 1 && dotted_parts.iter().any(|part| part.len() > 1));
                if technical {
                    result.push(ch);
                } else {
                    result.extend(ch.to_uppercase());
                }
                capitalize_next = false;
            } else {
                result.push(ch);
                if ch == '\n' || is_sentence_boundary(text, index, ch) {
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

        // A list item carries no terminal punctuation; appending "." to the
        // last line would punctuate the list, not the sentence.
        let last_line = trimmed.rsplit('\n').next().unwrap_or(trimmed);
        if last_line.starts_with("• ") || starts_with_number_marker(last_line) {
            return trimmed.to_string();
        }

        let last_char = trimmed.chars().last().unwrap_or(' ');
        if last_char == '.'
            || last_char == '?'
            || last_char == '!'
            || last_char == ';'
            || last_char == ':'
        {
            return trimmed.to_string();
        }

        // Check if begins or trailing clause begins with common question words
        let sentence_start = trimmed
            .char_indices()
            .filter(|(index, ch)| *ch == '\n' || is_sentence_boundary(trimmed, *index, *ch))
            .map(|(index, ch)| index + ch.len_utf8())
            .next_back()
            .unwrap_or(0);
        let lower = trimmed[sentence_start..].trim().to_lowercase();
        let last_clause = lower
            .split([',', ';', '\n'])
            .next_back()
            .unwrap_or(&lower)
            .trim();
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

/// Sentence stops require a real token boundary. Dots inside email addresses,
/// paths, decimals and abbreviations do not start a new sentence.
pub(crate) fn is_sentence_boundary(text: &str, index: usize, ch: char) -> bool {
    if matches!(ch, '。' | '！' | '？') {
        return true;
    }
    if !matches!(ch, '.' | '!' | '?') {
        return false;
    }
    let after = text[index + ch.len_utf8()..].trim_start_matches(['"', '\'', '”', '’', ')', ']']);
    if !after.is_empty() && !after.starts_with(char::is_whitespace) {
        return false;
    }
    if ch != '.' {
        return true;
    }
    let token = text[..index]
        .split_whitespace()
        .next_back()
        .unwrap_or_default()
        .trim_start_matches(['(', '[', '\'', '"']);
    let initial = token.chars().count() == 1 && token.chars().all(char::is_alphabetic);
    let dotted_initials = token.contains('.')
        && token
            .split('.')
            .all(|part| part.chars().count() == 1 && part.chars().all(char::is_alphabetic));
    let abbreviation = matches!(
        token.to_ascii_lowercase().as_str(),
        "dr" | "mr" | "mrs" | "ms" | "prof" | "st" | "sr" | "jr" | "etc" | "vs"
    );
    !initial && !dotted_initials && !abbreviation
}

fn quoted_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut active = None;
    for (index, ch) in text.char_indices() {
        let before = text[..index].chars().next_back();
        let after = text[index + ch.len_utf8()..].chars().next();
        if before == Some('\\')
            || (matches!(ch, '\'' | '’')
                && before.is_some_and(char::is_alphanumeric)
                && after.is_some_and(char::is_alphanumeric))
        {
            continue;
        }
        if let Some((start, close)) = active {
            if ch == close {
                spans.push((start, index + ch.len_utf8()));
                active = None;
            }
        } else {
            if ch == '\'' && before.is_some_and(char::is_alphanumeric) {
                continue;
            }
            let close = match ch {
                '"' | '\'' | '`' => ch,
                '“' => '”',
                '‘' => '’',
                _ => continue,
            };
            active = Some((index, close));
        }
    }
    if let Some((start, _)) = active {
        spans.push((start, text.len()));
    }
    spans
}

// Spoken command names also occur as ordinary nouns. Preserve quoted names,
// embedded identifiers, and determiner-led noun phrases with a following
// predicate. Ambiguous single-word commands remain a conservative heuristic.
fn literal_punctuation_name(text: &str, start: usize, end: usize) -> bool {
    let matched = &text[start..end];
    let name = matched.trim().trim_end_matches('.');
    let term_start = start + matched.len() - matched.trim_start().len();
    let term_end = end - (matched.len() - matched.trim_end().len());
    if super::replacements::is_embedded_identifier(text, term_start, term_end) {
        return true;
    }
    if !matches!(
        name.to_ascii_lowercase().as_str(),
        "period" | "comma" | "colon" | "semicolon" | "semi colon"
    ) {
        return false;
    }
    let before: Vec<_> = text[..start]
        .split_whitespace()
        .rev()
        .take(2)
        .map(str::to_ascii_lowercase)
        .collect();
    let determiner = |word: &str| {
        matches!(
            word,
            "a" | "an" | "the" | "my" | "your" | "our" | "his" | "her" | "their" | "this" | "that"
        )
    };
    if before.first().is_some_and(|word| determiner(word)) {
        return true;
    }
    let next = text[end..]
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let predicate = matches!(
        next.as_str(),
        "is" | "are" | "was" | "were" | "has" | "had" | "of"
    ) || next.ends_with('s');
    !name.is_empty() && predicate && before.iter().any(|word| determiner(word))
}

/// `true` for lines like "1. Apples" or "12. Review" — a number followed by
/// ". " is a list marker, not a sentence.
fn starts_with_number_marker(line: &str) -> bool {
    let digits = line.bytes().take_while(|b| b.is_ascii_digit()).count();
    digits > 0
        && line.as_bytes().get(digits) == Some(&b'.')
        && line.as_bytes().get(digits + 1) == Some(&b' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writing_audit_quote_commands_and_asr_terminal_period_work_together() {
        for (source, expected) in [
            ("open quote hello close quote", "\"hello\""),
            ("hello world period.", "hello world."),
            (
                "We reviewed the users' feedback period",
                "We reviewed the users' feedback.",
            ),
        ] {
            assert_eq!(
                PunctuationInferer::replace_spoken_punctuation(source),
                expected
            );
        }
    }

    #[test]
    fn sentence_casing_preserves_dotted_tokens_and_abbreviations() {
        for (source, expected) in [
            (
                "email alex@example.com about version 3.14 tomorrow.",
                "Email alex@example.com about version 3.14 tomorrow.",
            ),
            ("edit src/api.ts then wait.", "Edit src/api.ts then wait."),
            (
                "use e.g. two examples. then stop.",
                "Use e.g. two examples. Then stop.",
            ),
            (
                "visit example.com. then send the report.",
                "Visit example.com. Then send the report.",
            ),
        ] {
            assert_eq!(PunctuationInferer::capitalize_sentences(source), expected);
        }
    }

    #[test]
    fn quoted_punctuation_names_and_noun_phrases_remain_literal() {
        for source in [
            "the trial period lasts 14 days",
            "the warranty period is too short",
            "please explain the comma operator",
            "write the word \"comma\" in the heading",
            "replace “semicolon” with “comma”",
        ] {
            assert_eq!(
                PunctuationInferer::replace_spoken_punctuation(source),
                source
            );
        }
    }

    #[test]
    fn separated_semicolon_spelling_is_a_single_command() {
        assert_eq!(
            PunctuationInferer::replace_spoken_punctuation(
                "the train is late semi colon we can wait"
            ),
            "the train is late; we can wait"
        );
    }

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
    fn test_infer_terminal_punctuation_leaves_lists_alone() {
        for text in [
            "These are the things:\n\n• Look at this\n• Fix the login",
            "Steps:\n\n1. Run the tests\n2. Deploy",
        ] {
            assert_eq!(
                PunctuationInferer::infer_terminal_punctuation(text),
                text,
                "list got terminal punctuation: {text:?}"
            );
        }
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
