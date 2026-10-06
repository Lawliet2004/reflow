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
        // "erm"/"uhm" are the same fillers spelled the British way; "mm" is
        // deliberately absent because it also means millimetres.
        let filler_with_sep =
            Regex::new(r"(?i)\b(um+|uhm+|uh+|erm|er+|ah+|hmm+)[ \t]*[,;:]+[ \t]*").unwrap();
        let mut out = filler_with_sep
            .replace_all(text, filler_replacement)
            .to_string();

        // Bare fillers. The leading `\s*` keeps "to ah do" from collapsing
        // into "todo".
        let bare_filler = Regex::new(r"(?i)[ \t]*\b(um+|uhm+|uh+|erm|er+|ah+|hmm+)\b").unwrap();
        out = bare_filler
            .replace_all(&out, filler_replacement)
            .to_string();

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
        let doubled = Regex::new(r"([,;:])([ \t]*[,;:])+").unwrap();
        out = doubled.replace_all(&out, "$1").to_string();

        // " ," -> ","  (space stranded ahead of punctuation)
        let space_before = Regex::new(r"[ \t]+([,.;:!?])").unwrap();
        out = space_before.replace_all(&out, "$1").to_string();

        // ",." -> "."  (a filler removed from the end of a sentence)
        let comma_then_stop = Regex::new(r"[,;:]+([.!?])").unwrap();
        out = comma_then_stop.replace_all(&out, "$1").to_string();

        // ". , Today" -> ". Today"  (a filler opened the sentence)
        let stop_then_comma = Regex::new(r"([.!?])[ \t]*[,;:]+[ \t]*").unwrap();
        out = stop_then_comma.replace_all(&out, "$1 ").to_string();

        // A leading separator or trailing comma can be stranded by deletion.
        // Terminal colons and semicolons can be explicit formatting commands.
        let leading = Regex::new(r"^[\s,;:]+").unwrap();
        out = leading.replace(&out, "").to_string();
        let trailing = Regex::new(r"[\s,]+$").unwrap();
        out = trailing.replace(&out, "").to_string();

        let space_re = Regex::new(r"[ \t]+").unwrap();
        space_re.replace_all(&out, " ").trim().to_string()
    }

    /// Removes immediate stuttering word duplicates ("the the" -> "the")
    /// and whole repeated phrases ("we could, we could ship" -> "we could ship").
    pub fn remove_duplicates(text: &str) -> String {
        // Work within each line: a spoken paragraph is intentional structure.
        // These repetitions can carry emphasis or grammatical meaning; let the
        // optional language model judge them with context instead of deleting.
        const MEANINGFUL: &[&str] = &["very", "really", "so", "had", "that", "no"];
        let mut quote = None;
        text.split('\n')
            .map(|line| {
                let words: Vec<&str> = line.split_whitespace().collect();
                let quoted = quoted_words(&words, &mut quote);
                let mut result = Vec::new();
                let mut protected = Vec::new();
                let mut previous = String::new();
                for (index, word) in words.iter().enumerate() {
                    let key = word.to_lowercase();
                    let named_repeat =
                        index > 0 && looks_capitalized(words[index - 1]) && looks_capitalized(word);
                    if key != previous
                        || key.len() <= 1
                        || MEANINGFUL.contains(&key.as_str())
                        || quoted[index]
                        || named_repeat
                    {
                        result.push(*word);
                        protected.push(quoted[index]);
                    }
                    previous = key;
                }
                collapse_repeated_phrases(&result, &protected).join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        let mut rules = terms.to_vec();
        // A learned alias also enforces its preferred spelling when the ASR
        // recognizes the joined word but gets its capitalization wrong.
        rules.extend(
            terms
                .iter()
                .map(|(_, preferred)| (preferred.clone(), preferred.clone())),
        );
        super::replacements::apply_literal_rules(text, &rules)
    }
}

fn filler_replacement(captures: &regex::Captures<'_>) -> String {
    let word = captures.get(1).expect("filler word capture").as_str();
    // All-capital tokens may be acronyms. Sentence-initial "Uh"/"Um" are
    // still removable; guessing that an acronym is hesitation would lose data.
    if word.chars().count() >= 2 && word.chars().all(char::is_uppercase) {
        captures.get(0).unwrap().as_str().to_owned()
    } else {
        String::new()
    }
}

/// Collapses consecutive repeats of a 2-6 word phrase ("said twice or
/// thrice"), keeping the last copy so its punctuation survives.
///
/// The speaker restarting a phrase is the common case ("i think we should i
/// think we should move it"), and the last attempt is the one that continues
/// into the rest of the sentence. Single words are handled by the caller.
/// Quoted text, completed sentences and possible names are retained because
/// their repetition may be content rather than an abandoned restart.
fn collapse_repeated_phrases<'a>(words: &[&'a str], protected: &[bool]) -> Vec<&'a str> {
    let keys: Vec<String> = words
        .iter()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .collect();
    let mut keep: Vec<bool> = vec![true; words.len()];
    // Longest first, so a 4-word repeat is not half-eaten by the 2-word pass.
    for n in (2..=6usize).rev() {
        let mut i = 0;
        while i + 2 * n <= words.len() {
            let repeats = |a: usize, b: usize| keys[a..a + n] == keys[b..b + n];
            if repeats(i, i + n) {
                let mut reps = 2;
                while i + (reps + 1) * n <= words.len() && repeats(i, i + reps * n) {
                    reps += 1;
                }
                let literal = protected[i..i + reps * n].iter().any(|word| *word);
                let named = words[i..i + 2 * n]
                    .iter()
                    .all(|word| looks_capitalized(word));
                let completed = words[i..i + (reps - 1) * n].iter().any(|word| {
                    word.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
                        .ends_with(['.', '!', '?'])
                });
                if !literal && !named && !completed {
                    keep[i..i + (reps - 1) * n].fill(false);
                }
                i += reps * n;
            } else {
                i += 1;
            }
        }
    }
    words
        .iter()
        .zip(&keep)
        .filter_map(|(w, keep)| keep.then_some(*w))
        .collect()
}

fn looks_capitalized(word: &str) -> bool {
    word.chars()
        .find(|ch| ch.is_alphabetic())
        .is_some_and(char::is_uppercase)
}

/// Literal quotes may cross dictated lines. Apostrophes inside words and
/// possessive endings do not open quotes.
fn quoted_words(words: &[&str], quote: &mut Option<char>) -> Vec<bool> {
    words
        .iter()
        .map(|word| {
            let mut literal = quote.is_some();
            for (at, ch) in word.char_indices() {
                let previous = word[..at].chars().next_back();
                let next = word[at + ch.len_utf8()..].chars().next();
                if matches!(ch, '\'' | '’')
                    && previous.is_some_and(char::is_alphabetic)
                    && next.is_some_and(char::is_alphabetic)
                {
                    continue;
                }
                if let Some(closing) = *quote {
                    literal = true;
                    if ch == closing {
                        *quote = None;
                    }
                } else if matches!(ch, '"' | '“' | '‘')
                    || (ch == '\'' && !previous.is_some_and(char::is_alphanumeric))
                {
                    literal = true;
                    *quote = Some(match ch {
                        '“' => '”',
                        '‘' => '’',
                        _ => ch,
                    });
                }
            }
            literal
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_keeps_explicit_paragraphs_with_or_without_fillers() {
        for fillers in [true, false] {
            assert_eq!(
                TextCleaner::clean(
                    "First paragraph.\n\nSecond paragraph.\nThird line.",
                    fillers
                ),
                "First paragraph.\n\nSecond paragraph.\nThird line."
            );
        }
        assert_eq!(
            TextCleaner::clean("Um, first paragraph.\n\nUh, second paragraph.", true),
            "first paragraph.\n\nsecond paragraph."
        );
    }

    #[test]
    fn repetitions_do_not_cross_paragraphs_or_remove_meaningful_emphasis() {
        assert_eq!(
            TextCleaner::remove_duplicates("Go\n\nGo again"),
            "Go\n\nGo again"
        );
        assert_eq!(
            TextCleaner::remove_duplicates("It is very very useful. I had had enough."),
            "It is very very useful. I had had enough."
        );
    }

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

    #[test]
    fn terminal_colons_and_semicolons_survive_cleanup() {
        for text in ["Current status:", "First clause;"] {
            assert_eq!(TextCleaner::remove_fillers(text), text);
        }
        assert_eq!(TextCleaner::remove_fillers("Done, um"), "Done");
    }

    #[test]
    fn filler_removal_preserves_uppercase_acronyms() {
        assert_eq!(
            TextCleaner::remove_fillers("I work in the ER, not the ICU."),
            "I work in the ER, not the ICU."
        );
        for text in ["Uh, go now", "Um, go now", "Uhm, go now"] {
            assert_eq!(TextCleaner::remove_fillers(text), "go now");
        }
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

    /// A restarted phrase keeps its final attempt — the one that flows into
    /// the rest of the sentence — and the earlier copy's punctuation goes
    /// with it.
    #[test]
    fn repeated_phrases_keep_the_last_copy() {
        let cases = [
            (
                "i think we we should i think we should move it",
                "i think we should move it",
            ),
            (
                "we could, we could ship on friday",
                "we could ship on friday",
            ),
            ("go to the go to the go to the store", "go to the store"),
        ];
        for (raw, expected) in cases {
            assert_eq!(TextCleaner::remove_duplicates(raw), expected, "{raw:?}");
        }
        // Emphasis is not a stutter.
        assert_eq!(
            TextCleaner::remove_duplicates("very very useful"),
            "very very useful"
        );
        // Repetition across a paragraph break is intentional.
        assert_eq!(
            TextCleaner::remove_duplicates("Go\n\nGo again"),
            "Go\n\nGo again"
        );
    }

    #[test]
    fn repeated_names_quotes_and_completed_sentences_keep_their_content() {
        for text in [
            "I live in New York, New York.",
            "The title is Baden Baden in the travel guide.",
            "The slogan is \"please keep moving keep moving now\".",
            "The slogan is 'please don't stop don't stop now'.",
            "The sign says “please go go now”.",
            "The phrase is \"keep moving\nkeep moving keep moving\".",
            "Go now. Go now.",
            "Please wait! Please wait!",
        ] {
            assert_eq!(TextCleaner::remove_duplicates(text), text, "{text:?}");
        }
    }

    #[test]
    fn ordinary_restarts_still_collapse_around_preserved_quotes() {
        assert_eq!(
            TextCleaner::remove_duplicates("we could we could print \"go now go now\""),
            "we could print \"go now go now\""
        );
        assert_eq!(
            TextCleaner::remove_duplicates("I don't I don't need that"),
            "I don't need that"
        );
    }

    #[test]
    fn removes_erm_and_uhm_fillers() {
        assert_eq!(
            TextCleaner::remove_fillers("erm I think uhm we should go"),
            "I think we should go"
        );
        assert_eq!(
            TextCleaner::remove_fillers("well, erm, maybe"),
            "well, maybe"
        );
        // "mm" stays: it is also a millimetre.
        assert_eq!(
            TextCleaner::remove_fillers("cut it to 5 mm"),
            "cut it to 5 mm"
        );
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
