//! Context budgeting and chunking for long dictations.
//!
//! One request must fit `prompt + input + output <= n_ctx` with margin. Short
//! dictations go through whole; long ones are split at sentence or line
//! boundaries into similar-sized chunks. A spoken list ("first ... second ...",
//! "bullet") is never split, so numbering and its lead-in stay together.

use super::client::is_sentence_boundary;
use super::script::estimate_tokens;

/// Chunk when the input exceeds about this many words. Tune with the eval.
pub const MAX_CHUNK_WORDS: usize = 300;
/// Tokens kept free of `n_ctx` for estimation error and template overhead.
pub const CONTEXT_MARGIN_TOKENS: usize = 256;
/// The completion budget is `OUTPUT_TOKEN_FACTOR x input tokens + slack`.
pub const OUTPUT_TOKEN_FACTOR: f32 = 1.3;
pub const OUTPUT_TOKEN_SLACK: usize = 32;
/// A chunk is never planned smaller than this many input tokens.
const MIN_CHUNK_INPUT_TOKENS: usize = 64;
/// A list lead-in is the sentence before the first cue, if this short.
const LEAD_IN_MAX_WORDS: usize = 10;
/// Cue sentences further apart than this many sentences are separate lists.
const LIST_GAP_SENTENCES: usize = 2;

const ORDINALS: &[&str] = &[
    "first", "second", "third", "fourth", "fifth", "sixth", "firstly", "secondly", "thirdly",
];
const NUMBER_WORDS: &[&str] = &[
    "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];
const BULLET_CUES: &[&str] = &["bullet", "bullets"];

/// Completion budget for an input of `input_tokens`.
pub fn completion_tokens(input_tokens: usize) -> usize {
    (input_tokens as f32 * OUTPUT_TOKEN_FACTOR).ceil() as usize + OUTPUT_TOKEN_SLACK
}

/// Largest input one request may carry: prompt, input and a full-size answer
/// must all fit `n_ctx` with `CONTEXT_MARGIN_TOKENS` to spare.
pub fn max_chunk_input_tokens(n_ctx: u32, prompt_tokens: usize) -> usize {
    let available =
        (n_ctx as usize).saturating_sub(CONTEXT_MARGIN_TOKENS + prompt_tokens + OUTPUT_TOKEN_SLACK);
    ((available as f32 / (1.0 + OUTPUT_TOKEN_FACTOR)) as usize).max(MIN_CHUNK_INPUT_TOKENS)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    /// What joins this chunk to the previous output: "", " ", "\n" or "\n\n".
    pub separator_before: &'static str,
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn gap_separator(gap: &str) -> &'static str {
    if gap.is_empty() {
        ""
    } else if gap.matches('\n').count() >= 2 {
        "\n\n"
    } else if gap.contains('\n') {
        "\n"
    } else {
        " "
    }
}

/// Explicit discourse transitions give unpunctuated ASR text a safe topic
/// boundary without guessing where an ordinary clause ends.
fn topic_boundaries(text: &str) -> Vec<usize> {
    static TRANSITIONS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let transitions = TRANSITIONS.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(?:(?:okay\s+)?switching\s+topics|moving\s+on\s+to|on\s+another\s+topic|next\s+topic|last\s+thing)\b")
            .expect("fixed topic transitions")
    });
    transitions
        .find_iter(text)
        .filter(|found| !crate::formatting::literals::overlaps_literal(text, found.range()))
        .map(|found| found.start())
        .filter(|start| *start > 0 && text[..*start].ends_with(char::is_whitespace))
        .collect()
}

/// Trimmed byte ranges of each sentence or line.
fn sentences(text: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut push = |start: usize, end: usize| {
        let slice = &text[start..end];
        let trimmed = slice.trim();
        if !trimmed.is_empty() {
            let lead = slice.len() - slice.trim_start().len();
            ranges.push((start + lead, start + lead + trimmed.len()));
        }
    };
    let topics = topic_boundaries(text);
    let mut start = 0;
    for (index, ch) in text.char_indices() {
        let end = index + ch.len_utf8();
        if topics.contains(&index) {
            push(start, index);
            start = index;
        }
        if ch == '\n' {
            push(start, index);
            start = end;
        } else if is_sentence_boundary(text, index, ch) {
            // A numbered marker belongs to its item, rather than forming a
            // sentence of its own (which could detach the final item).
            let prefix = text[start..index].trim();
            if ch == '.' && !prefix.is_empty() && prefix.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            push(start, end);
            start = end;
        }
    }
    push(start, text.len());
    ranges
}

fn cues(sentence: &str) -> (Vec<String>, bool) {
    let words: Vec<String> = sentence
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut found = Vec::new();
    let line = sentence.trim_start();
    let mut bullet = ["- ", "* ", "• "]
        .iter()
        .any(|marker| line.starts_with(marker));
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && matches!(line.as_bytes().get(digits), Some(b'.' | b')')) {
        found.push(format!("item {}", &line[..digits]));
    } else if words
        .first()
        .is_some_and(|word| NUMBER_WORDS.contains(&word.as_str()))
    {
        found.push(format!("item {}", words[0]));
    }
    for (i, word) in words.iter().enumerate() {
        if ORDINALS.contains(&word.as_str()) {
            found.push(word.clone());
        } else if BULLET_CUES.contains(&word.as_str()) {
            bullet = true;
        } else if matches!(word.as_str(), "step" | "number") {
            // "step one", "number two", "step 3" count; "phone number 555" or
            // "serial number 7" do not.
            let counts = words.get(i + 1).is_some_and(|next| {
                NUMBER_WORDS.contains(&next.as_str())
                    || (word == "step" && next.chars().all(|c| c.is_ascii_digit()))
            });
            if counts {
                found.push(format!("{word} {}", words[i + 1]));
            }
        }
    }
    (found, bullet)
}

/// Sentence-index ranges (inclusive) that make up one spoken list.
fn list_regions(text: &str, sentences: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let cue_info: Vec<(Vec<String>, bool)> =
        sentences.iter().map(|(s, e)| cues(&text[*s..*e])).collect();
    let cue_sentences: Vec<usize> = (0..sentences.len())
        .filter(|i| !cue_info[*i].0.is_empty() || cue_info[*i].1)
        .collect();
    let mut regions = Vec::new();
    let mut group_start = 0;
    while group_start < cue_sentences.len() {
        let mut group_end = group_start;
        while group_end + 1 < cue_sentences.len()
            && cue_sentences[group_end + 1] - cue_sentences[group_end] <= LIST_GAP_SENTENCES
        {
            group_end += 1;
        }
        let members = &cue_sentences[group_start..=group_end];
        let mut distinct: Vec<&String> = members.iter().flat_map(|i| &cue_info[*i].0).collect();
        distinct.sort();
        distinct.dedup();
        let has_bullet = members.iter().any(|i| cue_info[*i].1);
        if distinct.len() >= 2 || has_bullet {
            let first = members[0];
            let last = *members.last().unwrap();
            let lead_in = first > 0 && {
                let (s, e) = sentences[first - 1];
                word_count(&text[s..e]) <= LEAD_IN_MAX_WORDS
            };
            regions.push((if lead_in { first - 1 } else { first }, last));
        }
        group_start = group_end + 1;
    }
    regions
}

/// Split `text` into chunks of at most `max_words` words and `max_tokens`
/// estimated tokens, where the text allows it. A sentence or spoken list
/// larger than the limit stays whole; the request budget check then decides.
pub fn split(text: &str, max_words: usize, max_tokens: usize) -> Vec<Chunk> {
    let text = text.trim();
    let total_words = word_count(text);
    let total_tokens = estimate_tokens(text);
    if total_words <= max_words && total_tokens <= max_tokens {
        return vec![Chunk {
            text: text.to_owned(),
            separator_before: "",
        }];
    }

    let topics = topic_boundaries(text);
    let sentence_ranges = sentences(text);
    // Merge each spoken list into one atom so no boundary falls inside it.
    let regions = list_regions(text, &sentence_ranges);
    let mut atoms: Vec<(usize, usize)> = Vec::new();
    let mut index = 0;
    while index < sentence_ranges.len() {
        if let Some((from, to)) = regions.iter().find(|(from, _)| *from == index) {
            atoms.push((sentence_ranges[*from].0, sentence_ranges[*to].1));
            index = to + 1;
        } else {
            atoms.push(sentence_ranges[index]);
            index += 1;
        }
    }

    let parts = total_words
        .div_ceil(max_words.max(1))
        .max(total_tokens.div_ceil(max_tokens.max(1)))
        .max(1);

    // Close a chunk when the next sentence would overshoot an even share by
    // more than stopping now undershoots it, or would break a hard limit. The
    // share is recomputed from what remains, so the last chunk is not a scrap.
    let stats: Vec<(usize, usize, usize, usize)> = atoms
        .iter()
        .map(|&(start, end)| {
            let atom = &text[start..end];
            (start, end, word_count(atom), estimate_tokens(atom))
        })
        .collect();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    let (mut current_start, mut current_end) = (atoms[0].0, atoms[0].0);
    let (mut words, mut tokens) = (0usize, 0usize);
    // Shares are taken from the per-sentence estimates, which are what the
    // running sums add up, not from the estimate of the whole text.
    let mut left_words: usize = stats.iter().map(|s| s.2).sum();
    let mut left_tokens: usize = stats.iter().map(|s| s.3).sum();
    let mut parts_left = parts;
    for (start, end, atom_words, atom_tokens) in stats {
        let started = current_end > current_start;
        let closer = |have: usize, add: usize, share: usize| {
            have + add > share && have + add - share > share.saturating_sub(have)
        };
        if started {
            let hard = words + atom_words > max_words || tokens + atom_tokens > max_tokens;
            let soft = parts_left > 1
                && (closer(words, atom_words, left_words.div_ceil(parts_left))
                    || closer(tokens, atom_tokens, left_tokens.div_ceil(parts_left)));
            if hard || soft || topics.contains(&start) {
                groups.push((current_start, current_end));
                current_start = start;
                left_words = left_words.saturating_sub(words);
                left_tokens = left_tokens.saturating_sub(tokens);
                parts_left = parts_left.saturating_sub(1).max(1);
                words = 0;
                tokens = 0;
            }
        }
        current_end = end;
        words += atom_words;
        tokens += atom_tokens;
    }
    groups.push((current_start, current_end));

    let mut chunks = Vec::with_capacity(groups.len());
    let mut previous_end = None;
    for (start, end) in groups {
        let separator_before = previous_end.map_or("", |prev| {
            if topics.contains(&start) {
                "\n\n"
            } else {
                gap_separator(&text[prev..start])
            }
        });
        chunks.push(Chunk {
            text: text[start..end].to_owned(),
            separator_before,
        });
        previous_end = Some(end);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sentence(i: usize) -> String {
        format!(
            "Sentence number {i} talks about the project plan and the budget for the quarter ahead."
        )
    }

    fn paragraphs(count: usize) -> String {
        (0..count).map(sentence).collect::<Vec<_>>().join(" ")
    }

    fn rejoin(chunks: &[Chunk]) -> String {
        chunks.iter().fold(String::new(), |mut out, chunk| {
            out.push_str(chunk.separator_before);
            out.push_str(&chunk.text);
            out
        })
    }

    #[test]
    fn deterministic_list_markers_stay_together_across_chunk_limits() {
        for list in [
            "1. Call the bank.\n2. Email the landlord.\n3. Book the tickets.",
            "- Call the bank.\n- Email the landlord.\n- Book the tickets.",
            "one call the bank. two email the landlord. three book the tickets.",
        ] {
            let chunks = split(list, 5, 2000);
            assert_eq!(chunks.len(), 1, "{list}");
            assert_eq!(chunks[0].text, list);
        }
    }

    #[test]
    fn explicit_topic_changes_split_long_unpunctuated_dictation() {
        let topic = "we need to review the project status today ".repeat(15);
        let text = format!("{topic}okay switching topics {topic}last thing {topic}");
        let chunks = split(&text, MAX_CHUNK_WORDS, 2000);
        assert_eq!(chunks.len(), 3);
        assert!(chunks
            .iter()
            .all(|chunk| word_count(&chunk.text) <= MAX_CHUNK_WORDS));
        assert_eq!(chunks[1].separator_before, "\n\n");
        assert_eq!(chunks[2].separator_before, "\n\n");
        assert_eq!(
            rejoin(&chunks).split_whitespace().collect::<Vec<_>>(),
            text.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn short_input_is_one_untouched_chunk() {
        let chunks = split("Send it to Priya.", MAX_CHUNK_WORDS, 1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "Send it to Priya.");
        assert_eq!(chunks[0].separator_before, "");
    }

    #[test]
    fn long_input_splits_at_sentence_boundaries_without_losing_text() {
        let text = paragraphs(40); // ~600 words
        let chunks = split(&text, MAX_CHUNK_WORDS, 10_000);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(
                word_count(&chunk.text) <= MAX_CHUNK_WORDS,
                "{}",
                word_count(&chunk.text)
            );
            assert!(
                chunk.text.ends_with('.'),
                "split mid-sentence: {}",
                chunk.text
            );
            assert!(chunk.text.starts_with("Sentence"));
        }
        assert_eq!(rejoin(&chunks), text);
        // Balanced rather than 300 + remainder.
        let sizes: Vec<usize> = chunks.iter().map(|c| word_count(&c.text)).collect();
        assert!(
            sizes.iter().max().unwrap() - sizes.iter().min().unwrap() < 60,
            "{sizes:?}"
        );
    }

    #[test]
    fn line_breaks_are_boundaries_and_reported_as_separators() {
        let text = format!("{}\n\n{}", paragraphs(20), paragraphs(20));
        let chunks = split(&text, MAX_CHUNK_WORDS, 10_000);
        assert!(chunks.iter().any(|c| c.separator_before == "\n\n"));
        assert_eq!(rejoin(&chunks), text);
    }

    #[test]
    fn a_spoken_list_is_never_split() {
        let list =
            "Things to do first call the bank. Second email the landlord. Third book the tickets.";
        let text = format!("{} {list} {}", paragraphs(14), paragraphs(14));
        let chunks = split(&text, 120, 10_000);
        assert!(chunks.len() >= 2);
        let holder: Vec<_> = chunks
            .iter()
            .filter(|c| c.text.contains("first call"))
            .collect();
        assert_eq!(holder.len(), 1);
        assert!(holder[0].text.contains("Things to do first call the bank."));
        assert!(holder[0].text.contains("Third book the tickets."));
        assert_eq!(rejoin(&chunks), text);
    }

    #[test]
    fn bullet_cue_keeps_its_lead_in_and_items_together() {
        let list = "Groceries. Bullet milk. Bullet eggs. Bullet bread. Bullet coffee.";
        let text = format!("{} {list} {}", paragraphs(12), paragraphs(12));
        let chunks = split(&text, 100, 10_000);
        let holder: Vec<_> = chunks
            .iter()
            .filter(|c| c.text.contains("Bullet milk"))
            .collect();
        assert_eq!(holder.len(), 1);
        assert!(holder[0].text.contains("Groceries. Bullet milk."));
        assert!(holder[0].text.contains("Bullet coffee."));
    }

    #[test]
    fn a_lone_ordinal_is_not_a_list() {
        let text = format!(
            "{} It was the first time we met. {}",
            paragraphs(10),
            paragraphs(10)
        );
        let chunks = split(&text, 100, 10_000);
        assert!(chunks.len() >= 2);
        assert_eq!(rejoin(&chunks), text);
    }

    #[test]
    fn indic_text_chunks_on_tokens_even_under_the_word_limit() {
        let sentence = "हमें कल सुबह नई तैनाती की जाँच करनी है और टीम को बताना है।";
        let text = vec![sentence; 20].join(" "); // 280 words, ~700 estimated tokens
        assert!(word_count(&text) < MAX_CHUNK_WORDS);
        let max_tokens = max_chunk_input_tokens(2048, 1000);
        assert!(
            estimate_tokens(&text) > max_tokens,
            "{}",
            estimate_tokens(&text)
        );
        let chunks = split(&text, MAX_CHUNK_WORDS, max_tokens);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            assert!(estimate_tokens(&chunk.text) <= max_tokens);
        }
    }

    #[test]
    fn budget_arithmetic_fits_the_context_with_margin() {
        let input = max_chunk_input_tokens(4096, 1000);
        let total = 1000 + input + completion_tokens(input);
        assert!(total + CONTEXT_MARGIN_TOKENS <= 4096, "{total}");
        assert_eq!(completion_tokens(100), 130 + OUTPUT_TOKEN_SLACK);
        assert_eq!(max_chunk_input_tokens(512, 1000), MIN_CHUNK_INPUT_TOKENS);
    }
}
