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
            DictationMode::Email => text.trim().to_string(),
            DictationMode::Notes => text.trim().to_string(),
            DictationMode::Normal => text.trim().to_string(),
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
