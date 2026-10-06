pub enum DictationMode {
    Normal,
    Coding,
    Email,
    Chat,
    Notes,
}

impl DictationMode {
    // Intentionally infallible and not `std::str::FromStr`: an unrecognised
    // dictation mode falls back to the default rather than erroring, and the
    // name is part of the existing call surface.
    #[allow(clippy::should_implement_trait)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseTransform {
    Camel,
    Pascal,
    Snake,
    ScreamingSnake,
    Kebab,
    Dot,
}

pub struct ContextFormatter;

impl ContextFormatter {
    pub fn format(text: &str, mode: DictationMode) -> String {
        match mode {
            DictationMode::Coding => Self::format_coding(text),
            DictationMode::Chat => text.trim().to_string(),
            DictationMode::Email => Self::format_email(text),
            DictationMode::Notes => text.trim().to_string(),
            DictationMode::Normal => text.trim().to_string(),
        }
    }

    /// Copy explicit email boundaries; leave uncertain or existing structure
    /// for the optional rewriter. Never supply a missing greeting or signature.
    pub fn format_email(text: &str) -> String {
        let text = text.trim();
        let literal = text.char_indices().any(|(at, ch)| {
            matches!(ch, '"' | '“' | '”' | '‘' | '`')
                || ch == '\'' && (at == 0 || text[..at].ends_with(char::is_whitespace))
        });
        let listed = text.split_whitespace().any(|word| {
            matches!(word, "•" | "-" | "*")
                || word.ends_with(['.', ')'])
                    && word
                        .trim_end_matches(['.', ')'])
                        .bytes()
                        .all(|ch| ch.is_ascii_digit())
        });
        if text.contains('\n') || literal || listed {
            return text.into();
        }
        let Some((greeting, body)) = text.split_once(',') else {
            return text.into();
        };
        let words = greeting.split_whitespace().collect::<Vec<_>>();
        if words.is_empty()
            || words.len() > 4
            || !matches!(
                words[0].to_ascii_lowercase().as_str(),
                "hi" | "hello" | "dear"
            )
            || words[1..].iter().any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "i" | "you" | "we" | "it" | "is" | "are" | "can" | "could" | "will" | "please"
                )
            })
            || body.trim().is_empty()
        {
            return text.into();
        }
        let body = body.trim();
        let lower = body.to_ascii_lowercase();
        let mut ambiguous = false;
        for closing in [
            "best regards",
            "kind regards",
            "best wishes",
            "thank you",
            "thanks",
            "regards",
            "sincerely",
            "cheers",
        ] {
            let Some(at) = lower.rfind(closing) else {
                continue;
            };
            if at > 0 && !body[..at].ends_with(char::is_whitespace) {
                continue;
            }
            ambiguous = true;
            let tail = &body[at + closing.len()..];
            let (signoff, name) = if let Some(name) = tail.strip_prefix(',') {
                (&body[at..at + closing.len() + 1], name.trim())
            } else if tail.trim_matches(['.', '!']).is_empty() {
                (&body[at..], "")
            } else {
                continue;
            };
            let names = name.split_whitespace().collect::<Vec<_>>();
            if names.len() > 3
                || names.iter().any(|word| {
                    !word.chars().next().is_some_and(char::is_uppercase)
                        || !word
                            .chars()
                            .all(|ch| ch.is_alphabetic() || matches!(ch, '-' | '\'' | '’' | '.'))
                })
                || !body[..at].trim_end().ends_with(['.', '?', '!', ';'])
            {
                continue;
            }
            return format!(
                "{greeting},\n\n{}\n\n{signoff}{}",
                body[..at].trim_end(),
                if name.is_empty() {
                    String::new()
                } else {
                    format!("\n{name}")
                }
            );
        }
        if ambiguous {
            text.into()
        } else {
            format!("{greeting},\n\n{body}")
        }
    }

    pub fn format_coding(text: &str) -> String {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }

        // 1. Transform spoken identifiers (camel case, snake case, etc.)
        let transformed = Self::transform_spoken_casing(trimmed);

        // 2. Transform spoken coding operators/symbols
        Self::transform_spoken_operators(&transformed)
    }

    pub fn transform_spoken_casing(text: &str) -> String {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        if tokens.is_empty() {
            return String::new();
        }

        let mut result = Vec::new();
        let mut i = 0;

        while i < tokens.len() {
            let mut detected_transform: Option<(CaseTransform, usize)> = None;

            // Check 3-token trigger
            if i + 2 < tokens.len() {
                let triplet = format!(
                    "{} {} {}",
                    tokens[i].to_lowercase(),
                    tokens[i + 1].to_lowercase(),
                    tokens[i + 2].to_lowercase()
                );
                if triplet == "screaming snake case" || triplet == "screaming snake case:" {
                    detected_transform = Some((CaseTransform::ScreamingSnake, 3));
                }
            }

            // Check 2-token triggers
            if detected_transform.is_none() && i + 1 < tokens.len() {
                let pair = format!(
                    "{} {}",
                    tokens[i].to_lowercase(),
                    tokens[i + 1].to_lowercase()
                );
                let clean_pair = pair.trim_end_matches(':');
                match clean_pair {
                    "camel case" | "camel cases" => {
                        detected_transform = Some((CaseTransform::Camel, 2));
                    }
                    "pascal case" | "pascal cases" => {
                        detected_transform = Some((CaseTransform::Pascal, 2));
                    }
                    "snake case" | "snake cases" => {
                        detected_transform = Some((CaseTransform::Snake, 2));
                    }
                    "screaming snake" | "constant case" => {
                        detected_transform = Some((CaseTransform::ScreamingSnake, 2));
                    }
                    "kebab case" | "dash case" => {
                        detected_transform = Some((CaseTransform::Kebab, 2));
                    }
                    "dot case" | "dot notation" => {
                        detected_transform = Some((CaseTransform::Dot, 2));
                    }
                    _ => {}
                }
            }

            // Check 1-token triggers
            if detected_transform.is_none() {
                let single = tokens[i].to_lowercase();
                let clean_single = single.trim_end_matches(':');
                match clean_single {
                    "camelcase" => detected_transform = Some((CaseTransform::Camel, 1)),
                    "pascalcase" => detected_transform = Some((CaseTransform::Pascal, 1)),
                    "snakecase" => detected_transform = Some((CaseTransform::Snake, 1)),
                    "kebabcase" => detected_transform = Some((CaseTransform::Kebab, 1)),
                    _ => {}
                }
            }

            if let Some((transform, prefix_len)) = detected_transform {
                let start_idx = i + prefix_len;
                let mut end_idx = start_idx;

                // Collect identifier words until punctuation/keyword boundary
                while end_idx < tokens.len() {
                    let word = tokens[end_idx];
                    let lower = word.to_lowercase();
                    let clean = lower.trim_matches(|c: char| !c.is_alphanumeric());

                    // Stop if we encounter another case trigger
                    if (clean == "camel"
                        || clean == "pascal"
                        || clean == "snake"
                        || clean == "kebab"
                        || clean == "screaming")
                        && end_idx + 1 < tokens.len()
                    {
                        let next_lower = tokens[end_idx + 1]
                            .to_lowercase()
                            .trim_matches(|c: char| !c.is_alphanumeric())
                            .to_string();
                        if next_lower == "case" || next_lower == "snake" {
                            break;
                        }
                    }

                    // Stop if word is a structural keyword connector
                    if end_idx > start_idx && is_code_boundary_word(clean) {
                        break;
                    }

                    end_idx += 1;

                    // Stop if word ends with punctuation attached
                    if word.ends_with(';')
                        || word.ends_with(',')
                        || word.ends_with('.')
                        || word.ends_with('(')
                        || word.ends_with(')')
                        || word.ends_with('{')
                        || word.ends_with('}')
                        || word.ends_with('[')
                        || word.ends_with(']')
                    {
                        break;
                    }
                }

                if end_idx > start_idx {
                    let words = &tokens[start_idx..end_idx];
                    let formatted = apply_casing(words, transform);
                    result.push(formatted);
                    i = end_idx;
                } else {
                    // No target words followed trigger; output trigger tokens verbatim
                    for t in &tokens[i..start_idx] {
                        result.push(t.to_string());
                    }
                    i = start_idx;
                }
            } else {
                result.push(tokens[i].to_string());
                i += 1;
            }
        }

        result.join(" ")
    }

    fn transform_spoken_operators(text: &str) -> String {
        let mut out = text.to_string();
        const OPERATORS: &[(&str, &str)] = &[
            (" arrow function ", " => "),
            (" fat arrow ", " => "),
            (" thin arrow ", " -> "),
            (" skinny arrow ", " -> "),
            (" triple equals ", " === "),
            (" double equals ", " == "),
            (" not equals ", " != "),
            (" not equal to ", " != "),
            (" greater than or equal to ", " >= "),
            (" greater than or equal ", " >= "),
            (" less than or equal to ", " <= "),
            (" less than or equal ", " <= "),
            (" plus equals ", " += "),
            (" minus equals ", " -= "),
            (" times equals ", " *= "),
            (" divide equals ", " /= "),
            (" double colon ", "::"),
            (" nullish coalescing ", " ?? "),
            (" logical and ", " && "),
            (" double and ", " && "),
            (" logical or ", " || "),
            (" double pipe ", " || "),
            (" strictly equals ", " === "),
        ];

        for (spoken, symbol) in OPERATORS {
            // Case-insensitive replacement
            let lower = out.to_lowercase();
            let mut pos = 0;
            while let Some(found) = lower[pos..].find(spoken) {
                let actual_idx = pos + found;
                out.replace_range(actual_idx..actual_idx + spoken.len(), symbol);
                pos = actual_idx + symbol.len();
            }
        }

        out
    }
}

fn is_code_boundary_word(word: &str) -> bool {
    matches!(
        word,
        "equals"
            | "equal"
            | "colon"
            | "semicolon"
            | "comma"
            | "open"
            | "close"
            | "return"
            | "import"
            | "from"
            | "export"
            | "class"
            | "struct"
            | "enum"
            | "function"
            | "fn"
            | "def"
            | "let"
            | "const"
            | "var"
            | "if"
            | "else"
            | "while"
            | "for"
    )
}

fn apply_casing(words: &[&str], transform: CaseTransform) -> String {
    if words.is_empty() {
        return String::new();
    }

    // Extract trailing punctuation from the last word if any
    let last_word = words.last().unwrap();
    let trailing_punct: String = last_word
        .chars()
        .rev()
        .take_while(|c| !c.is_alphanumeric())
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    let clean_words: Vec<String> = words
        .iter()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();

    if clean_words.is_empty() {
        return words.join(" ");
    }

    let transformed = match transform {
        CaseTransform::Camel => {
            let mut s = String::new();
            for (idx, w) in clean_words.iter().enumerate() {
                if idx == 0 {
                    s.push_str(w);
                } else {
                    let mut chars = w.chars();
                    if let Some(first) = chars.next() {
                        s.push(first.to_ascii_uppercase());
                        s.push_str(chars.as_str());
                    }
                }
            }
            s
        }
        CaseTransform::Pascal => {
            let mut s = String::new();
            for w in &clean_words {
                let mut chars = w.chars();
                if let Some(first) = chars.next() {
                    s.push(first.to_ascii_uppercase());
                    s.push_str(chars.as_str());
                }
            }
            s
        }
        CaseTransform::Snake => clean_words.join("_"),
        CaseTransform::ScreamingSnake => {
            let upper: Vec<String> = clean_words.iter().map(|w| w.to_uppercase()).collect();
            upper.join("_")
        }
        CaseTransform::Kebab => clean_words.join("-"),
        CaseTransform::Dot => clean_words.join("."),
    };

    format!("{transformed}{trailing_punct}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_layout_copies_present_greeting_and_signoff() {
        for (source, expected) in [
            (
                "Hi Maya, please send the report. Thanks, Alex",
                "Hi Maya,\n\nplease send the report.\n\nThanks,\nAlex",
            ),
            (
                "Hello Sam, can you review the draft? Best regards, Priya Rao",
                "Hello Sam,\n\ncan you review the draft?\n\nBest regards,\nPriya Rao",
            ),
            (
                "Dear team, the release is ready. Regards, Jordan.",
                "Dear team,\n\nthe release is ready.\n\nRegards,\nJordan.",
            ),
            (
                "Hi everyone, the release is ready. Thanks.",
                "Hi everyone,\n\nthe release is ready.\n\nThanks.",
            ),
            (
                "Hi Maya, can you send the report?",
                "Hi Maya,\n\ncan you send the report?",
            ),
        ] {
            assert_eq!(ContextFormatter::format_email(source), expected, "{source}");
            assert_eq!(
                ContextFormatter::format(source, DictationMode::Email),
                expected
            );
        }
    }

    #[test]
    fn email_layout_keeps_ambiguous_and_existing_structure_intact() {
        for source in [
            "Good morning Maya, please send the report. Thanks, Alex",
            "Hi can you help, please send the report. Thanks, Alex",
            "Hi Maya please send the report. Thanks, Alex",
            "The note ends with Thanks, Alex",
            "Hi team, the note ends with Thanks, Alex",
            "Hi Maya, please send the report. Thanks for helping",
            "Hi Maya, she said \"Thanks, Alex\".",
            "Hi Maya, she said 'Thanks, Alex'.",
            "Hi Maya,\n\nplease send the report.\n\nThanks,\nAlex",
            "Hi team,\n\n• Read the report\n• Check the draft",
            "Hi team, 1. Read the report 2. Check the draft",
        ] {
            assert_eq!(ContextFormatter::format_email(source), source, "{source}");
        }
    }

    #[test]
    fn transforms_camel_case() {
        let out = ContextFormatter::format("camel case get user id", DictationMode::Coding);
        assert_eq!(out, "getUserId");

        let out_inline = ContextFormatter::format(
            "let camel case user account equals 5",
            DictationMode::Coding,
        );
        assert!(out_inline.contains("userAccount"));
    }

    #[test]
    fn transforms_pascal_case() {
        let out = ContextFormatter::format("pascal case user profile", DictationMode::Coding);
        assert_eq!(out, "UserProfile");
    }

    #[test]
    fn transforms_snake_case() {
        let out =
            ContextFormatter::format("snake case db connection string", DictationMode::Coding);
        assert_eq!(out, "db_connection_string");
    }

    #[test]
    fn transforms_screaming_snake_case() {
        let out =
            ContextFormatter::format("screaming snake case max timeout", DictationMode::Coding);
        assert_eq!(out, "MAX_TIMEOUT");

        let out2 = ContextFormatter::format("constant case buffer size", DictationMode::Coding);
        assert_eq!(out2, "BUFFER_SIZE");
    }

    #[test]
    fn transforms_kebab_case() {
        let out = ContextFormatter::format("kebab case auth token header", DictationMode::Coding);
        assert_eq!(out, "auth-token-header");
    }

    #[test]
    fn transforms_spoken_operators() {
        let out =
            ContextFormatter::format("const x fat arrow y double equals z", DictationMode::Coding);
        assert_eq!(out, "const x => y == z");
    }

    #[test]
    fn preserves_trailing_punctuation_on_cased_identifiers() {
        let out = ContextFormatter::format("return camel case user id;", DictationMode::Coding);
        assert_eq!(out, "return userId;");
    }
}
