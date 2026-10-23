use regex::Regex;

/// Words that mark a spoken self-correction, i.e. "what I said before this is
/// wrong, what follows replaces it".
const REPAIR_MARKERS: &str =
    r"(?:actually|no[ \t]+wait|no|i[ \t]+mean|rather|sorry|scratch[ \t]+that)";

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
    out = super::literals::map_unquoted(&out, |text, _, _| {
        Regex::new(r"(?i)[ \t,]*\bno[ \t,]+seriously[ \t,]+(do not|don't)\b")
            .unwrap()
            .replace_all(text, |capture: &regex::Captures<'_>| {
                let before = text[..capture.get(0).unwrap().start()].trim_end();
                let separator = if before.is_empty() {
                    ""
                } else if before.ends_with(['.', '!', '?']) {
                    " "
                } else {
                    ". "
                };
                format!("{separator}Seriously, {}", &capture[1])
            })
            .into_owned()
    });
    // An explicit replacement may discard only a closed-class choice or a
    // restarted proposition. Never discard a quoted/code span along with it.
    for _ in 0..8 {
        let Some(next) = explicit_correction_sources(&out).into_iter().find(|next| {
            super::literals::literal_contents(&out) == super::literals::literal_contents(next)
        }) else {
            break;
        };
        if next == out {
            break;
        }
        out = next;
    }
    // An unresolved "no wait" may be ordinary speech ("No, wait for me").
    // Preserve it for contextual refinement rather than deleting a negation.
    super::literals::map_unquoted(&out, |text, _, _| squeeze_ws(text))
        .trim()
        .to_string()
}

/// Only explicit closed-class replacements and restarted propositions can
/// abandon content. A bare "actually" in ordinary prose is not a repair.
pub(crate) fn explicit_correction_sources(original: &str) -> Vec<String> {
    let tokenize = |text: &str| {
        text.split_whitespace()
            .map(|word| {
                word.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '.')
                    .trim_end_matches('.')
                    .to_lowercase()
            })
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
    };
    const TECH_CHOICES: &[&str] = &[
        "node.js",
        "node js",
        "javascript",
        "java script",
        "python",
        "fastapi",
        "fast api",
        "postgres",
        "postgresql",
        "mysql",
        "mongo db",
        "mongodb",
        "redis",
        "typescript",
        "type script",
        "react",
        "vue",
        "angular",
        "svelte",
        "next.js",
        "next js",
        "express",
        "django",
        "flask",
        "fastify",
        "spring boot",
        "sqlite",
        "supabase",
        "tauri",
        "rust",
        "golang",
        "c sharp",
        "c#",
    ];
    let choice_words = TECH_CHOICES
        .iter()
        .map(|choice| tokenize(choice))
        .collect::<Vec<_>>();
    let is_choice = |text: &str| {
        let words = tokenize(text);
        choice_words.contains(&words)
    };
    let starts_with_choice = |text: &str| {
        let words = tokenize(text);
        choice_words.iter().any(|choice| words.starts_with(choice))
    };
    static MARKER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(?:no(?:[ \t,]+no)*[ \t,]+(?:wait(?:[ \t,]+actually)?|actually|sorry)|sorry|i[ \t,]+mean|rather|actually)\b(?:[ \t,]+use\b)?")
            .expect("fixed correction marker")
    });
    static WORD: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let word = WORD.get_or_init(|| Regex::new(r"\S+").expect("fixed word boundary"));
    let mut sources = Vec::new();
    for found in marker.find_iter(original) {
        if crate::formatting::literals::overlaps_literal(original, found.range()) {
            continue;
        }
        let before = original[..found.start()].trim_end_matches([' ', '\t', ',', ':']);
        if before.ends_with(['.', '!', '?', ';', '\n', '\r']) {
            continue;
        }
        let after = original[found.end()..].trim_start_matches([' ', '\t', ',', ':']);
        if after.is_empty() || after.starts_with(['\n', '\r']) {
            continue;
        }
        // A repair cannot abandon text across an explicit line break. Keep byte
        // offsets so applying it leaves every earlier paragraph and literal
        // untouched, rather than reconstructing the prefix with joined words.
        let line_start = before.rfind(['\n', '\r']).map_or(0, |at| at + 1);
        let words: Vec<_> = word.find_iter(&before[line_start..]).collect();
        let replacement_line = after.split(['\n', '\r']).next().unwrap_or_default();
        let replacement: Vec<&str> = replacement_line.split_whitespace().collect();
        for start in 0..words.len().saturating_sub(1) {
            let anchor = words[start]
                .as_str()
                .trim_matches(|ch: char| !ch.is_alphanumeric());
            if !matches!(
                anchor.to_ascii_lowercase().as_str(),
                "i" | "we"
                    | "you"
                    | "he"
                    | "she"
                    | "they"
                    | "send"
                    | "give"
                    | "bring"
                    | "call"
                    | "email"
                    | "book"
                    | "take"
            ) {
                continue;
            }
            let common = words[start..]
                .iter()
                .zip(&replacement)
                .take_while(|(left, right)| {
                    left.as_str()
                        .trim_matches(|ch: char| !ch.is_alphanumeric())
                        .eq_ignore_ascii_case(right.trim_matches(|ch: char| !ch.is_alphanumeric()))
                })
                .count();
            if common >= 2
                && common < replacement.len()
                && !words[start..]
                    .iter()
                    .any(|word| word.as_str().ends_with(['.', '!', '?', ';']))
            {
                let prefix = &before[..line_start + words[start].start()];
                sources.push(format!("{prefix}{after}"));
                break;
            }
        }
        for count in 1..=4.min(words.len()) {
            let cut = line_start + words[words.len() - count].start();
            if !is_choice(&before[cut..]) || !starts_with_choice(replacement_line) {
                continue;
            }
            sources.push(format!("{}{after}", &before[..cut]));
        }
    }
    sources
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
        let pattern =
            format!(r"(?i)\b({class})[ \t]*,?[ \t]+{REPAIR_MARKERS}[ \t]*,?[ \t]+({class})\b");
        let re = Regex::new(&pattern).expect("backtrack value-correction regex");
        // Repeat so a chain ("5 actually 6 no wait 7") collapses fully.
        for _ in 0..4 {
            let next = re
                .replace_all(&out, |captures: &regex::Captures<'_>| {
                    let before = captures.get(1).expect("value before marker");
                    let after = captures.get(2).expect("value after marker");
                    let whole = captures.get(0).expect("value correction");
                    if super::literals::overlaps_literal(&out, whole.range())
                        || class == NUMERIC
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
    let marker = Regex::new(r"(?i)\bscratch[ \t]+that\b[,\.]?").expect("scratch that regex");
    let mut out = text.to_string();
    let mut guard = 0;
    let mut search_from = 0;
    while guard < 32 {
        guard += 1;
        let Some(m) = marker.find_at(&out, search_from) else {
            break;
        };
        if super::literals::overlaps_literal(&out, m.range())
            || unsupported_numeric_repair(&out, m.start(), m.end())
        {
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
    fn later_repairs_preserve_preceding_paragraphs_and_literals() {
        for (source, expected) in [
            (
                "Ship the browser first.\n\nUse Node.js no wait use Python FastAPI.\n\nKeep `npm run check` unchanged.",
                "Ship the browser first.\n\nUse Python FastAPI.\n\nKeep `npm run check` unchanged.",
            ),
            (
                "Élodie owns accessibility.\n\nWe want React no actually Svelte.\n\nI want tea no no sorry I want coffee",
                "Élodie owns accessibility.\n\nWe want Svelte.\n\nI want coffee",
            ),
            (
                "I want tea\n\nI want coffee no wait I want water",
                "I want tea\n\nI want water",
            ),
            (
                "Keep `React  no wait  Svelte` exactly.\n\nUse Node.js no wait use Python.",
                "Keep `React  no wait  Svelte` exactly.\n\nUse Python.",
            ),
        ] {
            assert_eq!(apply_backtrack(source), expected, "{source}");
        }
    }

    #[test]
    fn explicit_breaks_do_not_join_a_repair_marker_or_its_replacement() {
        for source in [
            "Use React\nno wait use Svelte",
            "Use React no wait\nuse Svelte",
            "Use Node\nJS no wait use Python",
            "I want tea\nno wait I want coffee",
            "I want tea no\nwait I want coffee",
            "Use React no wait,\n\nuse Svelte",
            "meet Monday i\nmean Tuesday",
            "send the draft scratch\nthat send the report",
            "do not delete no\nseriously don't delete",
        ] {
            assert_eq!(apply_backtrack(source), source, "{source}");
        }
    }

    #[test]
    fn clear_phrase_repairs_keep_the_final_choice_and_unrelated_clauses() {
        for (source, expected) in [
            ("I want tea no no sorry I want coffee", "I want coffee"),
            ("I want to no no no sorry I want coffee", "I want coffee"),
            ("Use React no actually Svelte", "Use Svelte"),
            ("Use React no wait use Svelte", "Use Svelte"),
            ("I would like to book the morning train no wait I would like to book the evening train", "I would like to book the evening train"),
            ("The deadline is Monday sorry Tuesday", "The deadline is Tuesday"),
        ] { assert_eq!(apply_backtrack(source), expected); }
        for source in [
            "Keep `React no wait Svelte` exactly",
            "I actually like React",
            "I want tea. Keep the 3 reports, sorry I want coffee",
            "He shouted no no no",
            "This is very very important",
        ] {
            assert_eq!(apply_backtrack(source), source);
        }
    }

    #[test]
    fn ordinary_wait_requests_and_quoted_repairs_preserve_their_meaning() {
        for text in [
            "no wait for me",
            "No, wait until I arrive.",
            "the sign says \"scratch that\"",
            "say 'friday i mean thursday' exactly",
            "print `5 actually 6`",
            "say “scratch that\nno wait”",
            "say \"scratch that without closing the quote",
            "we should meet friday\ni mean thursday",
        ] {
            assert_eq!(apply_backtrack(text), text, "{text}");
        }
    }

    #[test]
    fn punctuated_self_corrections_keep_the_final_value() {
        for (source, expected) in [
            ("meet at 5, actually, 6", "meet at 6"),
            ("ship friday, i mean, thursday", "ship thursday"),
            ("ship june, sorry, july", "ship july"),
            ("meet at 5 actually 6, no wait, 7", "meet at 7"),
        ] {
            assert_eq!(apply_backtrack(source), expected, "{source}");
        }
    }

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

#[cfg(test)]
mod adaptive_repair_tests {
    use super::apply_backtrack;

    #[test]
    fn bare_no_repairs_only_same_class_values_outside_literals() {
        assert_eq!(
            apply_backtrack("ship Friday, no, Thursday"),
            "ship Thursday"
        );
        for source in [
            "Friday no deployments Thursday",
            "do not delete the database",
            "very very important",
            "say \"Friday, no, Thursday\"",
            "run `Friday no Thursday`",
        ] {
            assert_eq!(apply_backtrack(source), source);
        }
    }
}

#[cfg(test)]
mod imperative_restart_tests {
    use super::apply_backtrack;

    #[test]
    fn a_restarted_command_keeps_only_the_final_recipient() {
        assert_eq!(
            apply_backtrack("send it to Rahul no wait send it to Priya before five thirty pm"),
            "send it to Priya before five thirty pm"
        );
        assert_eq!(
            apply_backtrack("bring the red folder no sorry bring the blue folder"),
            "bring the blue folder"
        );
    }

    #[test]
    fn an_ordinary_wait_request_does_not_discard_a_command() {
        assert_eq!(
            apply_backtrack("send it today no wait for me"),
            "send it today no wait for me"
        );
        assert_eq!(
            apply_backtrack("actually I think we should wait"),
            "actually I think we should wait"
        );
    }
}
