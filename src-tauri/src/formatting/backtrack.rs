use regex::Regex;

/// Words that mark a spoken self-correction, i.e. "what I said before this is
/// wrong, what follows replaces it".
const REPAIR_MARKERS: &str = r"(?:actually|no\s+wait|i\s+mean|sorry|scratch\s+that)";

/// Closed classes of "value" words that a correction can swap.
///
/// Matching is restricted to *pairs drawn from the same class*, and that is the
/// entire safety argument. A general "replace the word before the marker with the
/// word after it" rule mangles ordinary speech — "I'm going to work, I mean it"
/// would become "I'm going to it". Requiring both sides to be the same kind of
/// value makes the rule apply exactly when a substitution is what the speaker
/// meant, and never otherwise.
const NUMERIC: &str = r"(?:\d{1,2}:\d{2}|\d+|one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|noon|midnight)";
const DAYS: &str = r"(?:monday|tuesday|wednesday|thursday|friday|saturday|sunday|today|tomorrow|yesterday|tonight)";
const MONTHS: &str =
    r"(?:january|february|march|april|may|june|july|august|september|october|november|december)";

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

/// `<value> actually|i mean|no wait <value>` → keep the replacement value.
///
/// Deliberately deterministic rather than delegated to the polish LLM. Speech
/// repair is signalled by explicit lexical markers, which makes it a pattern
/// problem, not a language-modelling one: a 0.8B model asked to resolve
/// "ship this on friday i mean thursday" answered "ship this on Friday, meaning
/// Thursday" — it kept both values, which is the one outcome that is definitely
/// wrong. Doing it here is exact, costs nothing, and — the part that matters
/// most — also works in Fast mode, where there is no LLM in the pipeline at all.
fn apply_value_corrections(text: &str) -> String {
    let mut out = text.to_string();

    for class in [NUMERIC, DAYS, MONTHS] {
        // Both sides drawn from the same class; the second one wins.
        let pattern = format!(r"(?i)\b({class})\s+{REPAIR_MARKERS}\s+({class})\b");
        let re = Regex::new(&pattern).expect("backtrack value-correction regex");
        // Repeat so a chain ("5 actually 6 no wait 7") collapses fully.
        for _ in 0..4 {
            let next = re
                .replace_all(&out, |captures: &regex::Captures<'_>| {
                    let before = captures.get(1).expect("value before marker");
                    let after = captures.get(2).expect("value after marker");
                    if class == NUMERIC
                        && (!standalone_numeric_value(&out, before.start(), before.end())
                            || !standalone_numeric_value(&out, after.start(), after.end()))
                    {
                        captures[0].to_string()
                    } else {
                        after.as_str().to_string()
                    }
                })
                .to_string();
            if next == out {
                break;
            }
            out = next;
        }
    }

    out
}

/// A numeric regex match can be only the suffix/prefix of a decimal, date,
/// currency amount or compound spoken number. Keep these ambiguous expressions
/// complete instead of repairing one token inside them.
fn standalone_numeric_value(text: &str, start: usize, end: usize) -> bool {
    let before = &text[..start];
    let after = &text[end..];
    if before.ends_with(['.', ',', ':', '/', '-', '+', '$', '€', '£', '₹', '¥', '₩'])
        || after.starts_with([':', '/', '-', '+', '%', '$', '€', '£', '₹', '¥', '₩'])
        || after.starts_with(['.', ','])
            && after.chars().nth(1).is_some_and(|ch| !ch.is_whitespace())
    {
        return false;
    }
    let mut previous = before.split_whitespace().rev();
    let mut next = after.split_whitespace();
    !numeric_continuation(previous.next(), previous.next())
        && !numeric_continuation(next.next(), next.next())
}

fn numeric_continuation(nearest: Option<&str>, following: Option<&str>) -> bool {
    nearest.is_some_and(numeric_component)
        || nearest.is_some_and(|word| word.eq_ignore_ascii_case("and"))
            && following.is_some_and(numeric_component)
}

fn numeric_component(word: &str) -> bool {
    matches!(
        word,
        "+" | "-" | "−" | "$" | "€" | "£" | "₹" | "¥" | "₩" | "%"
    ) || word.chars().any(char::is_numeric)
        || word.split('-').any(|part| {
            matches!(
                part.trim_matches(|ch: char| !ch.is_alphanumeric())
                    .to_ascii_lowercase()
                    .as_str(),
                "zero"
                    | "one"
                    | "two"
                    | "three"
                    | "four"
                    | "five"
                    | "six"
                    | "seven"
                    | "eight"
                    | "nine"
                    | "ten"
                    | "eleven"
                    | "twelve"
                    | "thirteen"
                    | "fourteen"
                    | "fifteen"
                    | "sixteen"
                    | "seventeen"
                    | "eighteen"
                    | "nineteen"
                    | "twenty"
                    | "thirty"
                    | "forty"
                    | "fifty"
                    | "sixty"
                    | "seventy"
                    | "eighty"
                    | "ninety"
                    | "hundred"
                    | "thousand"
                    | "million"
                    | "billion"
                    | "trillion"
                    | "point"
                    | "minus"
                    | "negative"
                    | "plus"
                    | "positive"
                    | "noon"
                    | "midnight"
            )
        })
}

fn unsupported_numeric_repair(text: &str, start: usize, end: usize) -> bool {
    text[..start]
        .split_whitespace()
        .next_back()
        .is_some_and(numeric_component)
        && text[end..]
            .split_whitespace()
            .next()
            .is_some_and(numeric_component)
}

/// `scratch that` removes the preceding clause/sentence (from the last .!? or start).
fn apply_scratch_that(text: &str) -> String {
    let marker = Regex::new(r"(?i)\bscratch\s+that\b[,\.]?").expect("scratch that regex");
    let mut out = text.to_string();
    let mut guard = 0;
    let mut search_from = 0;
    while guard < 32 {
        guard += 1;
        let Some(m) = marker.find_at(&out, search_from) else {
            break;
        };
        if unsupported_numeric_repair(&out, m.start(), m.end()) {
            search_from = m.end();
            continue;
        }
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
        search_from = 0;
    }
    out
}

fn strip_lone_no_wait(text: &str) -> String {
    let re = Regex::new(r"(?i)\s*\bno\s+wait\b[,\.]?").expect("no wait regex");
    re.replace_all(text, |captures: &regex::Captures<'_>| {
        let marker = captures.get(0).expect("repair marker");
        if unsupported_numeric_repair(text, marker.start(), marker.end()) {
            marker.as_str().to_string()
        } else {
            " ".to_string()
        }
    })
    .to_string()
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

    /// The regression this generalisation exists for. Asked to resolve this, the
    /// 0.8B polish model produced "ship this on Friday, meaning Thursday" — it
    /// kept both values, which is worse than leaving the text alone. Handling it
    /// deterministically is exact and also works with the LLM switched off.
    #[test]
    fn i_mean_replaces_the_previous_weekday() {
        assert_eq!(
            apply_backtrack("ship this on friday i mean thursday"),
            "ship this on thursday"
        );
    }

    #[test]
    fn day_and_month_corrections_are_resolved() {
        assert_eq!(
            apply_backtrack("let's do it monday sorry tuesday"),
            "let's do it tuesday"
        );
        assert_eq!(
            apply_backtrack("the deadline is june i mean july"),
            "the deadline is july"
        );
        assert_eq!(
            apply_backtrack("see you tomorrow no wait today"),
            "see you today"
        );
    }

    /// A chain of corrections must collapse to the final value.
    #[test]
    fn chained_corrections_keep_only_the_last_value() {
        assert_eq!(
            apply_backtrack("meet at 5 actually 6 no wait 7"),
            "meet at 7"
        );
    }

    /// The same-class restriction is the safety mechanism. Without it, an
    /// idiomatic "I mean" swallows the sentence: "going to work, I mean it"
    /// would become "going to it".
    #[test]
    fn idiomatic_markers_are_left_alone() {
        for text in [
            "I'm going to work I mean it",
            "I actually enjoyed the movie",
            "sorry about the delay",
            "I mean what I say",
            "no wait for me",
        ] {
            let out = apply_backtrack(text);
            assert!(
                out.to_lowercase().contains(
                    &text
                        .to_lowercase()
                        .split_whitespace()
                        .last()
                        .unwrap()
                        .to_string()
                ),
                "{text:?} lost its final word: {out:?}"
            );
        }
    }

    /// Mixed classes must not cross-match: a day is not a replacement for a
    /// number.
    #[test]
    fn corrections_do_not_cross_value_classes() {
        let out = apply_backtrack("at five i mean friday");
        assert!(out.to_lowercase().contains("five"), "{out}");
        assert!(out.to_lowercase().contains("friday"), "{out}");
    }

    #[test]
    fn compound_numeric_values_are_not_partially_replaced() {
        for text in [
            "pay 12.50 actually 12.60",
            "pay -12.50 i mean -12.60",
            "pay minus five actually six",
            "pay negative twelve actually thirteen",
            "pay - 12 actually 13",
            "pay $ 12 actually 13",
            "pay $12.50 actually 12.60",
            "pay €12 actually 13",
            "pay 12.50% sorry 13.50%",
            "send 1,250 actually 1,350 units",
            "ship on 2026-10-01 actually 2026-10-02",
            "ship on 10/01/2026 i mean 10/02/2026",
            "ship on 2026 10 01 actually 2026 10 02",
            "meet at 12:30:15 actually 12:30:30",
            "bring twenty five actually thirty six notebooks",
            "bring twenty-five i mean thirty-six notebooks",
            "bring one hundred and five actually two hundred and six notebooks",
            "bring one thousand five actually six notebooks",
            "bring twelve point five actually thirteen point six units",
        ] {
            assert_eq!(apply_backtrack(text), text, "{text}");
        }
    }

    #[test]
    fn a_compound_on_either_side_blocks_partial_numeric_repair() {
        for text in [
            "pay 12.50 actually 13",
            "pay 12 actually 13.50",
            "bring twenty five actually six notebooks",
            "bring five actually twenty six notebooks",
            "bring five actually one hundred notebooks",
            "set 1/2 actually 3",
            "set 1 actually 3/4",
        ] {
            assert_eq!(apply_backtrack(text), text, "{text}");
        }
    }

    #[test]
    fn unsupported_numeric_repairs_keep_their_full_value_and_marker() {
        for text in [
            "pay 12.50 no wait 12.60",
            "pay 12.50 scratch that 12.60",
            "bring twenty five no wait thirty six notebooks",
            "bring twenty five scratch that thirty six notebooks",
            "ship on 2026-10-01 no wait 2026-10-02",
            "meet at 12:30:15 scratch that 12:30:30",
        ] {
            assert_eq!(apply_backtrack(text), text, "{text}");
        }
    }

    #[test]
    fn complete_standalone_numbers_and_simple_times_still_correct() {
        for (source, expected) in [
            ("meet at 5 actually 6", "meet at 6"),
            ("meet at 12:30 actually 13:40", "meet at 13:40"),
            ("meet at noon no wait midnight", "meet at midnight"),
            (
                "bring one i mean thirteen notebooks",
                "bring thirteen notebooks",
            ),
            ("send 12500 actually 12600 units", "send 12600 units"),
            (
                "bring five scratch that six notebooks",
                "bring six notebooks",
            ),
            ("meet at 5 actually 6 no wait 7", "meet at 7"),
        ] {
            assert_eq!(apply_backtrack(source), expected, "{source}");
        }
    }
}
