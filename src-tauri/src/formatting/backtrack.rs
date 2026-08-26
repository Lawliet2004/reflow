use regex::Regex;

/// Spoken self-corrections: last value wins; `scratch that` drops the prior clause.
pub fn apply_backtrack(text: &str) -> String {
    if text.trim().is_empty() {
        return text.trim().to_string();
    }

    let mut out = text.to_string();
    out = apply_value_corrections(&out);
    out = apply_scratch_that(&out);
    out = strip_lone_no_wait(&out);
    squeeze_ws(&out)
}

/// `<token> actually|no wait <number/time>` → keep the replacement token.
fn apply_value_corrections(text: &str) -> String {
    let mut out = text.to_string();

    // Digits / times: "at 5 actually 6", "at 5 no wait 6", "at 3:00 actually 4:00"
    let number_fix = Regex::new(
        r"(?i)\b(?:\d{1,2}:\d{2}|\d+)\s+(?:actually|no\s+wait|scratch\s+that|i\s+mean)\s+(\d{1,2}:\d{2}|\d+)\b",
    )
    .expect("backtrack number regex");
    out = number_fix.replace_all(&out, "$1").to_string();

    // Number-words after a value preposition: "at five actually six"
    let word_fix = Regex::new(
        r"(?i)\b((?:at|on|for|by|to|around|from|until|till)\s+)(?:\d{1,2}:\d{2}|\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|noon|midnight)\s+(?:actually|no\s+wait)\s+(\d{1,2}:\d{2}|\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|noon|midnight)\b",
    )
    .expect("backtrack word-value regex");
    out = word_fix.replace_all(&out, "$1$2").to_string();

    out
}

/// `scratch that` removes the preceding clause/sentence (from the last .!? or start).
fn apply_scratch_that(text: &str) -> String {
    let marker = Regex::new(r"(?i)\bscratch\s+that\b[,\.]?").expect("scratch that regex");
    let mut out = text.to_string();
    let mut guard = 0;
    while guard < 32 {
        guard += 1;
        let Some(m) = marker.find(&out) else {
            break;
        };
        let before = &out[..m.start()];
        let mut cut = 0;
        for (i, ch) in before.char_indices() {
            if ch == '.' || ch == '!' || ch == '?' || ch == '\n' {
                cut = i + ch.len_utf8();
            }
        }
        let prefix = &out[..cut];
        let suffix = &out[m.end()..];
        out = format!("{prefix}{suffix}");
    }
    out
}

fn strip_lone_no_wait(text: &str) -> String {
    let re = Regex::new(r"(?i)\s*\bno\s+wait\b[,\.]?").expect("no wait regex");
    re.replace_all(text, " ").to_string()
}

fn squeeze_ws(text: &str) -> String {
    let horiz = Regex::new(r"[^\S\n]+").expect("horiz space");
    let collapsed = horiz.replace_all(text, " ");
    let around_nl = Regex::new(r"[ \t]*\n[ \t]*").expect("nl space");
    around_nl.replace_all(&collapsed, "\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actually_replaces_previous_number() {
        assert_eq!(
            apply_backtrack("let's meet at 5 actually 6"),
            "let's meet at 6"
        );
    }

    #[test]
    fn actually_at_pattern_drops_old_value() {
        let out = apply_backtrack("let's meet at 5 actually 6");
        assert!(out.contains('6'), "{out}");
        assert!(!out.contains('5'), "{out}");
        assert!(!out.to_lowercase().contains("actually"), "{out}");
    }

    #[test]
    fn preserves_actually_when_not_a_correction() {
        assert_eq!(
            apply_backtrack("I actually enjoyed the movie"),
            "I actually enjoyed the movie"
        );
    }

    #[test]
    fn no_wait_replaces_trailing_number() {
        assert_eq!(
            apply_backtrack("let's meet at 5 no wait 6"),
            "let's meet at 6"
        );
    }

    #[test]
    fn scratch_that_removes_preceding_clause() {
        let out = apply_backtrack("send the email scratch that send the slack message");
        let lower = out.to_lowercase();
        assert!(lower.contains("send the slack message"), "{out}");
        assert!(!lower.contains("email"), "{out}");
        assert!(!lower.contains("scratch"), "{out}");
    }

    #[test]
    fn scratch_that_keeps_prior_sentence() {
        let out = apply_backtrack("Hello. Send the email scratch that send the slack message");
        let lower = out.to_lowercase();
        assert!(lower.starts_with("hello."), "{out}");
        assert!(lower.contains("send the slack message"), "{out}");
        assert!(!lower.contains("email"), "{out}");
    }
}
