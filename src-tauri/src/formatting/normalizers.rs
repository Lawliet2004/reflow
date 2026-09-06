use regex::Regex;

/// Weekday and month names, which are always capitalised in English.
///
/// Stage 1 has to do this itself. Sentence capitalisation only fixes the first
/// word, so ASR output like "ship this on thursday" kept a lowercase proper noun
/// — and leaving it to the polish model means it is simply never fixed in Fast
/// mode, where there is no polish model. It is a closed, unambiguous list, so
/// there is no reason to spend an LLM call on it.
const PROPER_NOUNS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "march",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Capitalise the closed set of always-capitalised words.
///
/// "may" and "march" are deliberately absent from the pattern as bare words:
/// they are common verbs ("may I", "march on") and capitalising them would be
/// wrong more often than right. Only unambiguous names are corrected.
fn capitalize_proper_nouns(text: &str) -> String {
    let mut out = text.to_string();
    for name in PROPER_NOUNS {
        let Ok(re) = Regex::new(&format!(r"(?i)\b{name}\b")) else {
            continue;
        };
        let mut capitalized = String::with_capacity(name.len());
        let mut chars = name.chars();
        if let Some(first) = chars.next() {
            capitalized.extend(first.to_uppercase());
            capitalized.push_str(chars.as_str());
        }
        out = re.replace_all(&out, capitalized.as_str()).to_string();
    }
    out
}

/// Conservative spoken-list cleanup. Does not rewrite prose number-words.
pub fn apply_normalizers(text: &str, dictation_mode: &str) -> String {
    let mut out = text.trim().to_string();
    let mode = dictation_mode.to_lowercase();
    if matches!(mode.as_str(), "notes" | "email") && looks_like_enumeration(&out) {
        if let Some(list) = try_numbered_list(&out) {
            out = list;
        }
    }
    // Not applied in coding mode: identifiers like `march` or `friday` are
    // ordinary lowercase symbols there, and rewriting them would break code.
    if mode != "coding" {
        out = capitalize_proper_nouns(&out);
    }
    let spaces = Regex::new(r"[ \t]+\n").expect("trail space");
    out = spaces.replace_all(&out, "\n").to_string();
    let multi = Regex::new(r" {2,}").expect("multi space");
    multi.replace_all(&out, " ").trim().to_string()
}

fn word_key(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn marker_num(key: &str) -> Option<u32> {
    match key {
        "one" | "1" | "first" => Some(1),
        "two" | "2" | "second" => Some(2),
        "three" | "3" | "third" => Some(3),
        "four" | "4" | "fourth" => Some(4),
        "five" | "5" | "fifth" => Some(5),
        "six" | "6" | "sixth" => Some(6),
        "seven" | "7" | "seventh" => Some(7),
        "eight" | "8" | "eighth" => Some(8),
        "nine" | "9" | "ninth" => Some(9),
        "ten" | "10" | "tenth" => Some(10),
        "eleven" | "11" | "eleventh" => Some(11),
        "twelve" | "12" | "twelfth" => Some(12),
        "twenty" | "20" => Some(20),
        "thirty" | "30" => Some(30),
        _ => None,
    }
}

fn looks_like_enumeration(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 3 {
        return false;
    }
    let first = word_key(words[0]);
    if marker_num(&first).is_none() {
        return false;
    }
    let marker_count = words
        .iter()
        .filter(|w| marker_num(&word_key(w)).is_some())
        .count();
    marker_count >= 2
}

fn try_numbered_list(text: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    marker_num(&word_key(words[0]))?;

    let mut items: Vec<(u32, Vec<&str>)> = Vec::new();
    let mut current_n: Option<u32> = None;
    let mut current: Vec<&str> = Vec::new();

    for w in &words {
        let key = word_key(w);
        if let Some(n) = marker_num(&key) {
            if let Some(cn) = current_n {
                if current.is_empty() {
                    return None;
                }
                let head = current[0].to_lowercase();
                if matches!(
                    head.as_str(),
                    "of" | "or" | "and" | "hundred" | "thousand" | "million"
                ) {
                    return None;
                }
                items.push((cn, std::mem::take(&mut current)));
            }
            current_n = Some(n);
        } else if current_n.is_some() {
            current.push(*w);
        } else {
            return None;
        }
    }

    if let Some(cn) = current_n {
        if current.is_empty() {
            return None;
        }
        let head = current[0].to_lowercase();
        if matches!(head.as_str(), "of" | "or" | "and" | "hundred" | "thousand") {
            return None;
        }
        items.push((cn, current));
    }

    if items.len() < 2 {
        return None;
    }

    Some(
        items
            .into_iter()
            .map(|(n, ws)| format!("{}. {}", n, ws.join(" ")))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_mode_builds_a_list() {
        let out = apply_normalizers("one apples two bananas", "notes");
        assert_eq!(out, "1. apples\n2. bananas");
    }

    #[test]
    fn email_mode_builds_a_list() {
        let out = apply_normalizers("one apples two bananas", "email");
        assert_eq!(out, "1. apples\n2. bananas");
    }

    #[test]
    fn first_second_list() {
        let out = apply_normalizers("first apples second bananas", "notes");
        assert_eq!(out, "1. apples\n2. bananas");
    }

    #[test]
    fn normal_mode_leaves_prose() {
        let out = apply_normalizers("one apples two bananas", "normal");
        assert_eq!(out, "one apples two bananas");
    }

    #[test]
    fn does_not_rewrite_one_of_two() {
        let out = apply_normalizers("one of the two options", "notes");
        assert_eq!(out, "one of the two options");
    }
}
