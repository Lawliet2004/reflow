//! Output validation for the cleanup LLM.
//!
//! A small model can answer instead of clean, translate, drop a negation, or
//! loop. Every check here is a cheap, deterministic guard; on any failure the
//! caller inserts the deterministic-cleaned text instead. Only the reason
//! code leaves this module, never text.

use super::script::{is_bengali, is_devanagari, ScriptCounts};

/// Inputs of at most this many words skip the length-ratio check: short
/// dictations legitimately change length a lot (digits, punctuation, lists).
pub const MIN_WORDS_FOR_RATIO_CHECK: usize = 15;
/// Output/input word ratio bounds for longer inputs.
// Editorial cleanup may remove extensive padding and repeated ideas.
// Keep the novelty, negation and list-detail checks below independent.
pub const MIN_WORD_RATIO: f32 = 0.3;
pub const MAX_WORD_RATIO: f32 = 1.2;
/// Reject when more than this fraction of the output's content words are not
/// in the input.
// Allow modest paraphrasing during editorial cleanup. This lexical heuristic
// is not a semantic proof; protected facts and negation are checked separately.
pub const MAX_NOVEL_WORD_FRACTION: f32 = 0.3;
/// A word counts as present when an input word shares at least this many
/// leading characters and differs by at most `STEM_SLACK_CHARS` (walk/walks).
pub const STEM_MIN_PREFIX_CHARS: usize = 3;
pub const STEM_SLACK_CHARS: usize = 2;
/// A loop is an n-gram repeated back to back at least this many times.
pub const LOOP_NGRAM_WORDS: usize = 4;
pub const LOOP_MIN_REPEATS: usize = 3;

/// Function words are not "content words": a grammar fix such as are -> is
/// must not read as invented content.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "am", "to", "of", "in", "on", "at",
    "for", "and", "or", "but", "if", "it", "its", "this", "that", "these", "those", "i", "you",
    "he", "she", "we", "they", "my", "your", "his", "her", "our", "their", "do", "does", "did",
    "have", "has", "had", "will", "would", "can", "could", "should", "shall", "may", "might",
    "with", "as", "by", "from", "so", "then", "than", "there", "here", "what", "who", "which",
    "when", "where", "why", "how", "also", "just", "very", "too", "me", "us", "them", "up", "out",
];

/// Dates are the facts a model most often "corrects". Any of these in the
/// output that the speaker never said is rejected outright.
const PROTECTED_WORDS: &[&str] = &[
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "january",
    "february",
    "april",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

const ENGLISH_NEGATIONS: &[&str] = &["not", "never", "cannot"];
const HINDI_NEGATIONS: &[&str] = &["नहीं", "नही", "मत", "न"];
const BENGALI_NEGATIONS: &[&str] = &["না", "নেই", "নি", "নয়"];

/// Lead-ins models add. Stripped only when the speaker did not say them.
const PREAMBLES: &[&str] = &[
    "here is",
    "here's",
    "here are",
    "sure",
    "certainly",
    "of course",
    "cleaned text",
    "cleaned transcript",
    "cleaned-up text",
    "the cleaned",
    "output:",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Empty,
    WordRatio,
    Hallucination,
    Translation,
    NegationDropped,
    Loop,
    ListItemChanged,
}

impl Rejection {
    /// Stable code for logs, diagnostics and the latency waterfall.
    pub fn code(self) -> &'static str {
        match self {
            Self::Empty => "empty_output",
            Self::WordRatio => "word_ratio",
            Self::Hallucination => "hallucination",
            Self::Translation => "translation",
            Self::NegationDropped => "negation_dropped",
            Self::Loop => "loop",
            Self::ListItemChanged => "list_item_changed",
        }
    }
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric()
        || matches!(ch, '\'' | '’')
        || ((is_devanagari(ch) || is_bengali(ch))
            && !matches!(ch, '\u{0964}' | '\u{0965}' | '\u{09F7}'))
}

/// Lowercased word tokens. Apostrophes stay inside words; hyphens split them.
fn words(text: &str) -> Vec<String> {
    text.split(|ch: char| !is_word_char(ch))
        .map(|word| {
            word.trim_matches(['\'', '’'])
                .replace('’', "'")
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn is_content_word(word: &str) -> bool {
    !word.chars().any(char::is_numeric) && !STOPWORDS.contains(&word)
}

fn negation_count(text: &str) -> usize {
    words(text).iter().filter(|word| is_negation(word)).count()
}

fn has_novel_loop(input: &[String], output: &[String]) -> bool {
    let span = LOOP_NGRAM_WORDS * LOOP_MIN_REPEATS;
    output.windows(span).any(|window| {
        let first = &window[..LOOP_NGRAM_WORDS];
        (1..LOOP_MIN_REPEATS).all(|repeat| {
            let start = repeat * LOOP_NGRAM_WORDS;
            &window[start..start + LOOP_NGRAM_WORDS] == first
        }) && !input.windows(span).any(|spoken| spoken == window)
    })
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.trim_start()
        .to_lowercase()
        .strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(|ch: char| !ch.is_alphanumeric()))
}

/// Strip preambles, wrapping quotes and stray transcript tags.
pub fn clean_output(raw: &str, input: &str) -> String {
    let mut text = raw
        .replace("<transcript>", "")
        .replace("</transcript>", "")
        .trim()
        .to_owned();
    let preamble = PREAMBLES
        .iter()
        .find(|p| starts_with_ignore_case(&text, p) && !starts_with_ignore_case(input, p));
    if preamble.is_some() {
        if let Some((first, rest)) = text.split_once('\n') {
            // "Here is the cleaned text:" on its own line.
            if first.trim_end().ends_with(':') || first.split_whitespace().count() <= 6 {
                text = rest.trim().to_owned();
            }
        } else if let Some((lead, rest)) = text.split_once(':') {
            if lead.split_whitespace().count() <= 6 {
                text = rest.trim().to_owned();
            }
        }
    }
    // Short acknowledgements may precede the cleaned text on the same line.
    for acknowledgement in ["sure", "certainly", "of course"] {
        if starts_with_ignore_case(&text, acknowledgement)
            && !starts_with_ignore_case(input, acknowledgement)
        {
            text = text[acknowledgement.len()..]
                .trim_start_matches(|ch: char| {
                    ch.is_whitespace() || matches!(ch, ',' | ':' | '!' | '.')
                })
                .to_owned();
        }
    }
    for (open, close) in [('"', '"'), ('“', '”'), ('\'', '\''), ('«', '»')] {
        let Some(inner) = text
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        else {
            continue;
        };
        // Interior quotes mean the quotes belong to the content.
        let balanced = !inner.contains(open) && !inner.contains(close);
        if balanced && !input.trim_start().starts_with(open) {
            text = inner.trim().to_owned();
            break;
        }
    }
    text
}

fn is_negation(word: &str) -> bool {
    word.ends_with("n't")
        || ENGLISH_NEGATIONS.contains(&word)
        || HINDI_NEGATIONS.contains(&word)
        || BENGALI_NEGATIONS.contains(&word)
}

/// Whether `word` is explained by the input: same word, an inflection of it,
/// or the stem of an input contraction ("do" in "don't"). Negations are
/// checked separately, so their spelling ("cannot"/"can't") is not novelty.
fn present_in(word: &str, input_words: &[String]) -> bool {
    if is_negation(word) {
        return true;
    }
    let word_chars: Vec<char> = word.chars().collect();
    input_words.iter().any(|known| {
        if known == word || known.starts_with(&format!("{word}'")) {
            return true;
        }
        let shared = word_chars
            .iter()
            .zip(known.chars())
            .take_while(|(a, b)| **a == *b)
            .count();
        let shorter = word_chars.len().min(known.chars().count());
        shared >= STEM_MIN_PREFIX_CHARS
            && shared == shorter
            && word_chars.len().abs_diff(known.chars().count()) <= STEM_SLACK_CHARS
    })
}

/// Text after a list marker ("- ", "* ", "• ", "1. ", "2) "), if `line` has one.
fn list_item_text(line: &str) -> Option<&str> {
    let line = line.trim_start();
    for marker in ["- ", "* ", "• "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    (digits > 0 && matches!(line.as_bytes().get(digits), Some(b'.' | b')')))
        .then(|| line[digits + 1..].trim_start())
}

/// A list the deterministic stage already built is structure the model may
/// restyle but not edit: every word of every item must survive.
fn list_items_preserved(input: &str, output_words: &[String]) -> bool {
    input
        .lines()
        .filter_map(list_item_text)
        .flat_map(words)
        .filter(|word| is_content_word(word))
        .all(|word| present_in(&word, output_words))
}

/// Validate `raw` (the model's output) against `input` (the deterministic
/// text the model was given). Returns the cleaned output or why it was refused.
pub fn validate(input: &str, raw: &str) -> Result<String, Rejection> {
    let output = clean_output(raw, input);
    if output.trim().is_empty() {
        return Err(Rejection::Empty);
    }
    let input_words = words(input);
    let output_words = words(&output);
    if output_words.is_empty() {
        return Err(Rejection::Empty);
    }

    if input_words.len() > MIN_WORDS_FOR_RATIO_CHECK {
        let ratio = output_words.len() as f32 / input_words.len() as f32;
        if !(MIN_WORD_RATIO..=MAX_WORD_RATIO).contains(&ratio) {
            return Err(Rejection::WordRatio);
        }
    }

    let input_scripts = ScriptCounts::of(input);
    let output_scripts = ScriptCounts::of(&output);
    if let (Some(from), Some(to)) = (input_scripts.dominant(), output_scripts.dominant()) {
        if from != to {
            return Err(Rejection::Translation);
        }
    }

    if negation_count(input) > negation_count(&output) {
        return Err(Rejection::NegationDropped);
    }

    if has_novel_loop(&input_words, &output_words) {
        return Err(Rejection::Loop);
    }

    if !list_items_preserved(input, &output_words) {
        return Err(Rejection::ListItemChanged);
    }

    if output_words
        .iter()
        .any(|w| PROTECTED_WORDS.contains(&w.as_str()) && !input_words.contains(w))
    {
        return Err(Rejection::Hallucination);
    }

    let content: Vec<&String> = output_words.iter().filter(|w| is_content_word(w)).collect();
    let novel = content
        .iter()
        .filter(|w| !present_in(w, &input_words))
        .count();
    if novel > 0 && novel as f32 > MAX_NOVEL_WORD_FRACTION * content.len() as f32 {
        return Err(Rejection::Hallucination);
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONG: &str =
        "we should push this release to the staging environment tomorrow morning and then \
        ask the whole team to check the new dashboard before the demo on friday afternoon";

    #[test]
    fn a_faithful_cleanup_passes() {
        assert_eq!(
            validate(
                "send it to priya before five thirty pm",
                "Send it to Priya before 5:30 PM."
            ),
            Ok("Send it to Priya before 5:30 PM.".into())
        );
        assert!(validate(LONG, &format!("{}.", LONG.replace("and then", "and"))).is_ok());
    }

    #[test]
    fn preambles_quotes_and_tags_are_stripped_then_checked() {
        let input = "i want coffee please";
        for raw in [
            "Here is the cleaned text:\nI want coffee please.",
            "Sure! Here's the cleaned text: I want coffee please.",
            "Cleaned text: I want coffee please.",
            "\"I want coffee please.\"",
            "<transcript>I want coffee please.</transcript>",
        ] {
            assert_eq!(
                validate(input, raw),
                Ok("I want coffee please.".into()),
                "{raw}"
            );
        }
        // A sentence the speaker really started with "sure" is not a preamble.
        assert_eq!(
            validate("sure i will send it today", "Sure, I will send it today."),
            Ok("Sure, I will send it today.".into())
        );
    }

    #[test]
    fn empty_output_is_rejected() {
        for raw in ["", "   \n", "<transcript></transcript>", "\"\""] {
            assert_eq!(
                validate("hello there friend", raw),
                Err(Rejection::Empty),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn editorial_cleanup_can_remove_padding_without_inventing_content() {
        let input = "so basically what I am trying to say is the upload is slow and the upload takes too long on mobile especially when the files are large";
        let polished = "The upload is especially slow on mobile with large files.";
        assert_eq!(validate(input, polished), Ok(polished.into()));
        let paraphrased =
            "The upload is slow on mobile, particularly when dealing with large files.";
        assert_eq!(validate(input, paraphrased), Ok(paraphrased.into()));
        assert_eq!(
            validate(input, "The upload is slow."),
            Err(Rejection::WordRatio)
        );
        assert_eq!(
            validate(
                input,
                "The upload is slow because the server has insufficient memory and bandwidth."
            ),
            Err(Rejection::Hallucination)
        );
    }

    #[test]
    fn word_ratio_is_enforced_only_above_the_input_threshold() {
        assert_eq!(
            validate(LONG, "We should push this."),
            Err(Rejection::WordRatio)
        );
        let bloated = format!("{LONG} {LONG}");
        assert_eq!(validate(LONG, &bloated), Err(Rejection::WordRatio));
        // Short input: a big relative change is allowed.
        assert!(validate("five thirty pm today", "5:30 PM today.").is_ok());
    }

    #[test]
    fn invented_content_is_rejected() {
        assert_eq!(
            validate(
                "i need to buy milk and eggs today",
                "I need to buy milk and eggs today. Thank you so much, best regards."
            ),
            Err(Rejection::Hallucination)
        );
        assert_eq!(
            validate(
                "what is the capital of france",
                "The capital of France is Paris."
            ),
            Err(Rejection::Hallucination)
        );
        // One inflected word is within tolerance.
        assert!(validate(
            "she walk to the store every single morning",
            "She walks to the store every single morning."
        )
        .is_ok());
    }

    #[test]
    fn grammar_fixes_and_changed_dates_are_told_apart() {
        assert!(validate("my beta are ready", "My beta is ready.").is_ok());
        assert_eq!(
            validate(
                "rahul will finish the frontend by friday and priya will test it",
                "Rahul will finish the frontend by Thursday and Priya will test it."
            ),
            Err(Rejection::Hallucination)
        );
    }

    #[test]
    fn translation_is_rejected_by_script() {
        assert_eq!(
            validate(
                "कल हमें API को fix करना है क्योंकि deployment परसों है",
                "We need to fix the API tomorrow because deployment is the day after."
            ),
            Err(Rejection::Translation)
        );
        assert_eq!(
            validate("we will meet tomorrow at the office", "हम कल दफ्तर में मिलेंगे"),
            Err(Rejection::Translation)
        );
        assert!(validate(
            "कल हमें API को fix करना है um because deployment परसों है",
            "कल हमें API को fix करना है because deployment परसों है।"
        )
        .is_ok());
        assert!(validate("আমাদের কাল API fix করতে হবে", "আমাদের কাল API fix করতে হবে।").is_ok());
    }

    #[test]
    fn a_dropped_negation_is_rejected_in_each_language() {
        assert_eq!(
            validate("i don't want this", "I want this."),
            Err(Rejection::NegationDropped)
        );
        assert_eq!(
            validate("we cannot ship it never mind later", "We ship it, later."),
            Err(Rejection::NegationDropped)
        );
        assert_eq!(
            validate("it is not ready", "It is ready."),
            Err(Rejection::NegationDropped)
        );
        assert_eq!(
            validate("मुझे यह नहीं चाहिए", "मुझे यह चाहिए।"),
            Err(Rejection::NegationDropped)
        );
        assert_eq!(
            validate("আমি এটা চাই না", "আমি এটা চাই।"),
            Err(Rejection::NegationDropped)
        );
        assert!(validate("i don't want this", "I don't want this.").is_ok());
        assert!(validate("i can't go", "I cannot go.").is_ok());
        // "no" is a correction cue, not a counted negation.
        assert!(validate("i want tea no no sorry i want coffee", "I want coffee.").is_ok());
    }

    #[test]
    fn words_cannot_be_dropped_from_an_existing_list_item() {
        let input = "Bring these:\n1. My mobile\n2. My power bank";
        assert_eq!(
            validate(input, "Bring these:\n1. My mobile\n2. My power"),
            Err(Rejection::ListItemChanged)
        );
        assert!(validate(input, "Bring these:\n1. My mobile\n2. My power bank").is_ok());
        assert!(validate("- milk\n- eggs\n- bread", "- Milk\n- Eggs\n- Bread").is_ok());
    }

    #[test]
    fn a_different_spoken_loop_does_not_authorize_a_new_loop() {
        let input = "send the report today ".repeat(3) + "bring the coffee tomorrow";
        let output = "bring the coffee tomorrow ".repeat(3);
        assert!(has_novel_loop(&words(&input), &words(&output)));
        assert_eq!(validate(&input, &output), Err(Rejection::Loop));
    }

    #[test]
    fn acknowledgements_are_stripped_without_removing_spoken_words() {
        assert_eq!(
            clean_output("Sure, Send the report.", "send the report"),
            "Send the report."
        );
        assert_eq!(
            clean_output("Surely we can go.", "we can go"),
            "Surely we can go."
        );
        assert_eq!(
            clean_output("Sure, I can go.", "sure i can go"),
            "Sure, I can go."
        );
    }

    #[test]
    fn shared_prefixes_do_not_authorize_unspoken_content() {
        assert!(!present_in("deployment", &words("department")));
        assert!(!present_in("coffee", &words("coffin")));
        assert!(present_in("reports", &words("report")));
    }

    #[test]
    fn a_dominant_script_change_is_rejected_even_when_both_scripts_remain() {
        assert_eq!(validate("API कल", "API कल कल"), Err(Rejection::Translation));
    }

    /// Validation runs on the dictation hot path after the model answers.
    #[test]
    fn validating_a_full_chunk_costs_milliseconds() {
        let text = (0..30)
            .map(|i| format!("Item {i} covers the quarterly plan and the budget review today."))
            .collect::<Vec<_>>()
            .join(" ");
        let started = std::time::Instant::now();
        for _ in 0..10 {
            assert!(validate(&text, &text).is_ok());
        }
        let per_call = started.elapsed() / 10;
        eprintln!("validate(300 words): {per_call:?} per call (debug build)");
        assert!(
            per_call < std::time::Duration::from_millis(50),
            "{per_call:?}"
        );
    }

    #[test]
    fn a_loop_is_rejected_unless_the_speaker_said_it() {
        let looped = "Please send the report. ".repeat(3);
        assert_eq!(
            validate("please send the report today", &looped),
            Err(Rejection::Loop)
        );
        let spoken = "please send the report ".repeat(3);
        assert!(validate(
            &spoken,
            "Please send the report. Please send the report. Please send the report."
        )
        .is_ok());
        assert!(!has_novel_loop(&[], &words("a b c d a b c d")));
    }
}
