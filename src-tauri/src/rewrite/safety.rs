/// Cleanup intensity used by the rewriter safety gate.
///
/// Formatting owns the canonical `CleanupLevel` when present; this local
/// type accepts the same names so shipped tests can pass `"medium"`/`"high"`
/// or this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupLevel {
    Raw,
    Light,
    Medium,
    High,
}

impl CleanupLevel {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "raw" => Self::Raw,
            "high" => Self::High,
            "medium" | "flow" => Self::Medium,
            _ => Self::Light,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Light => "light",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

impl From<&str> for CleanupLevel {
    fn from(value: &str) -> Self {
        Self::parse(value)
    }
}

impl From<String> for CleanupLevel {
    fn from(value: String) -> Self {
        Self::parse(&value)
    }
}

impl From<&String> for CleanupLevel {
    fn from(value: &String) -> Self {
        Self::parse(value)
    }
}

/// `true` when Stage 1's output is already clean enough that polishing cannot
/// meaningfully improve it.
///
/// Stage 2 costs a network round trip plus generation — ~400 ms on CPU with a
/// warm prompt cache — and on a well-formed short sentence it has nothing to do
/// but risk changing it. Skipping is therefore both faster and safer.
///
/// Deliberately conservative: it only skips text that is unambiguously finished
/// prose, and anything uncertain goes to the model. The failure mode to avoid is
/// skipping something that needed cleanup, so every condition here is a positive
/// signal of already-clean text rather than an absence of problems.
pub fn polish_would_add_nothing(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    // Long dictations are where phrasing and sentence splitting actually benefit
    // from a model, so they are never skipped.
    if words.len() > 12 {
        return false;
    }

    // Must start with a capital and end with terminal punctuation: that is what
    // "Stage 1 already finished the job" looks like.
    let first = trimmed.chars().next().unwrap_or(' ');
    if !first.is_uppercase() {
        return false;
    }
    if !trimmed.ends_with(['.', '!', '?']) {
        return false;
    }

    // Any residual disfluency or repair marker means there is still work to do.
    const RESIDUE: &[&str] = &[
        "um",
        "uh",
        "er",
        "ah",
        "hmm",
        "actually",
        "i mean",
        "no wait",
        "sorry",
        "scratch that",
        "you know",
    ];
    let lower = trimmed.to_ascii_lowercase();
    if RESIDUE.iter().any(|marker| {
        lower
            .split(|c: char| !c.is_alphanumeric() && c != ' ')
            .any(|segment| segment.split_whitespace().collect::<Vec<_>>().join(" ") == *marker)
            || lower.contains(&format!(" {marker} "))
    }) {
        return false;
    }

    // Doubled spaces or stranded punctuation are Stage 1 artefacts worth fixing.
    if lower.contains("  ") || lower.contains(" ,") || lower.contains(",,") {
        return false;
    }

    true
}

const MEANINGFUL_REPEATS: &[&str] = &["very", "really", "so", "had", "that", "no"];

/// Reject unsafe LLM rewrites. Returns the trimmed candidate when it is safe.
pub fn accept_rewrite(
    original: &str,
    candidate: &str,
    level: impl Into<CleanupLevel>,
) -> Option<String> {
    let level = level.into();
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }

    let orig_chars = original.chars().count();
    let cand_chars = trimmed.chars().count();
    let max_chars = ((orig_chars as f64) * 2.5) as usize + 20;
    if cand_chars > max_chars {
        return None;
    }

    if looks_like_meta(trimmed) {
        return None;
    }

    // Spoken line/paragraph breaks are intentional, even when the words match.
    if trimmed.matches('\n').count() < original.trim().matches('\n').count() {
        return None;
    }

    if numeric_literals(original) != numeric_literals(trimmed)
        || spoken_quantities(original) != spoken_quantities(trimmed)
        || !preserves_identifiers(original, trimmed)
        || punctuation_arguments(original) != punctuation_arguments(trimmed)
        || !preserves_scripts(original, trimmed)
        || !preserves_enumerated_items(original, trimmed)
        || !preserves_prose_content(original, trimmed)
    {
        return None;
    }

    if has_negation(original) && !has_negation(trimmed) {
        return None;
    }

    if drops_first_person(original, trimmed) {
        return None;
    }

    if reports_the_speaker(trimmed) {
        return None;
    }

    let orig_tokens = tokenize(original);
    let cand_tokens = tokenize(trimmed);
    // A short possessive label may reach the gate without its list marker.
    // Protect the complete name, rather than accepting a one-word deletion
    // because it is small relative to the transcript's edit-distance budget.
    let label = item_words(original);
    if label.len() <= 8
        && label.first().is_some_and(|word| {
            matches!(
                word.as_str(),
                "my" | "your" | "our" | "his" | "her" | "their"
            )
        })
        && drops_label_words(&label, &item_words(trimmed))
    {
        return None;
    }
    // These are the repetitions Stage 1 deliberately retains for grammar or
    // emphasis. A model must not silently turn them into accidental stutters.
    for word in MEANINGFUL_REPEATS {
        if orig_tokens
            .windows(2)
            .any(|pair| pair[0] == *word && pair[1] == *word)
        {
            let before = orig_tokens
                .iter()
                .filter(|token| token.as_str() == *word)
                .count();
            let after = cand_tokens
                .iter()
                .flat_map(|word| word.split('-'))
                .filter(|token| *token == *word)
                .count();
            if after < before {
                return None;
            }
        }
    }
    let max_len = orig_tokens.len().max(cand_tokens.len());
    // Bound the quadratic comparison. Oversized changes retain the original.
    if max_len > 8192 {
        return (orig_tokens == cand_tokens).then(|| trimmed.to_string());
    }
    if max_len > 0 {
        let distance = levenshtein(&orig_tokens, &cand_tokens);
        let ratio = distance as f32 / max_len as f32;
        let limit = match level {
            CleanupLevel::High => 0.75,
            _ => 0.55,
        };
        if ratio > limit {
            return None;
        }
    }

    Some(trimmed.to_string())
}

/// Preserve item names/modifiers without maintaining a compound-noun dictionary.
/// Articles, hesitation fillers and case/punctuation can change; the remaining
/// item words must match in order. This deliberately favours a complete fallback
/// over guessing whether a renamed object is an equivalent one.
pub(crate) fn preserves_item_words(original: &str, candidate: &str) -> bool {
    item_words(original) == item_words(candidate)
        && punctuation_arguments(original) == punctuation_arguments(candidate)
}

// A standalone dot can be a command argument. Lexical tokenization discards
// punctuation, so retain these literal arguments separately even in fragments.
fn punctuation_arguments(text: &str) -> Vec<(usize, &str)> {
    let mut arguments = Vec::new();
    let mut words_before = 0;
    for word in text
        .lines()
        .flat_map(|line| strip_list_marker(line).split_whitespace())
    {
        let argument = word.trim_matches(['`', '"', '\'', ',', ';']);
        if matches!(argument, "." | "..") {
            arguments.push((words_before, argument));
        } else {
            words_before += lexical_words(word).len();
        }
    }
    arguments
}

fn item_words(text: &str) -> Vec<String> {
    let mut words = lexical_words(text);
    // Only a leading article can be incidental. An internal "A" in "my A
    // grade report" or an article inside a quoted title is part of its name.
    if !text.trim_start().starts_with(['"', '“', '‘', '\''])
        && words
            .first()
            .is_some_and(|word| matches!(word.as_str(), "a" | "an" | "the"))
    {
        words.remove(0);
    }
    words
}

fn lexical_words(text: &str) -> Vec<String> {
    tokenize(text)
        .into_iter()
        .flat_map(|word| word.split('-').map(str::to_owned).collect::<Vec<_>>())
        .filter(|word| !matches!(word.as_str(), "um" | "uh" | "er" | "ah" | "hmm"))
        .collect()
}

/// Grammar may change, but edit distance alone cannot justify new objects or
/// names. Confirmed inventories use their stricter item checks; ordinary prose
/// retains content order and scoped prepositions. Grammar corrections do not
/// justify exchanging a source/destination or stemming an unfamiliar name.
fn preserves_prose_content(original: &str, candidate: &str) -> bool {
    if crate::formatting::normalizers::enumeration_items(original).is_some() {
        return true;
    }
    if original.trim_end().ends_with(':') {
        return preserves_group_header(original, candidate);
    }
    preserves_prose_words(original, candidate)
}

fn preserves_prose_words(original: &str, candidate: &str) -> bool {
    // Capitalization can identify a name even where the grammar is ambiguous.
    // Preserve its exact word in both signatures, including case-only edits.
    let protected: std::collections::HashSet<String> = [original, candidate]
        .into_iter()
        .flat_map(str::split_whitespace)
        .filter(|word| word.chars().any(char::is_uppercase))
        .flat_map(tokenize)
        .collect();
    let (after, after_prepositions) = prose_signature(candidate, &protected);
    let preserves = |source: &str| {
        let (before, before_prepositions) = prose_signature(source, &protected);
        if before != after {
            return false;
        }
        // Adding an agent to a passive clause can reverse an active statement's
        // roles despite keeping the same noun order. Cleanup cannot invent "by".
        if !before_prepositions
            .iter()
            .filter(|(_, word)| word == "by")
            .eq(after_prepositions.iter().filter(|(_, word)| word == "by"))
        {
            return false;
        }
        // Grammar can supply a missing preposition, but an existing one must
        // remain at the same position relative to the content it connects.
        let mut remaining = after_prepositions.iter();
        before_prepositions
            .iter()
            .all(|before| remaining.by_ref().any(|after| before == after))
    };
    preserves(original) || phrase_restart_source(original).is_some_and(|source| preserves(&source))
}

/// Source-only allowance for the adjacent phrase restarts Stage 1 already
/// removes. The candidate still has to preserve the entire cleaned signature;
/// all other safeguards compare against the untouched original.
fn phrase_restart_source(original: &str) -> Option<String> {
    // Semicolons separate complete clauses rather than unfinished restarts.
    if original.contains(';') {
        return None;
    }
    let cleaned = crate::formatting::cleaner::TextCleaner::remove_duplicates(original);
    let before = expanded_prose_words(original);
    let after = expanded_prose_words(&cleaned);
    if before.len() <= after.len()
        || MEANINGFUL_REPEATS.iter().any(|word| {
            before
                .iter()
                .filter(|token| token.as_str() == *word)
                .count()
                != after.iter().filter(|token| token.as_str() == *word).count()
        })
    {
        return None;
    }
    // Keep the cleaner's quote/name/sentence-boundary protections, but verify
    // that its punctuation-insensitive matching deleted only exact word copies.
    // Match from the end because the cleaner retains the final phrase attempt.
    let mut kept = vec![false; before.len()];
    let mut cursor = before.len();
    for word in after.iter().rev() {
        while cursor > 0 && before[cursor - 1] != *word {
            cursor -= 1;
        }
        if cursor == 0 {
            return None;
        }
        cursor -= 1;
        kept[cursor] = true;
    }
    let mut start = 0;
    while start < before.len() {
        if kept[start] {
            start += 1;
            continue;
        }
        let end = (start..before.len())
            .find(|index| kept[*index])
            .unwrap_or(before.len());
        let removed = end - start;
        let exact_restart = (2..=6).any(|width| {
            removed.is_multiple_of(width)
                && end + width <= before.len()
                && before[start..end]
                    .chunks(width)
                    .all(|copy| copy == &before[end..end + width])
        });
        if !exact_restart {
            return None;
        }
        start = end;
    }
    Some(cleaned)
}

fn expanded_prose_words(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| lexical_words(strip_list_marker(line)))
        .flat_map(|word| {
            let word = word.replace('’', "'");
            let expanded = match word.as_str() {
                "im" | "i'm" => "i am",
                "i've" => "i have",
                "i'll" => "i will",
                "i'd" => "i would",
                "we're" => "we are",
                "we've" => "we have",
                "we'll" => "we will",
                "we'd" => "we would",
                "you're" => "you are",
                "you've" => "you have",
                "you'll" => "you will",
                "you'd" => "you would",
                "they're" => "they are",
                "they've" => "they have",
                "they'll" => "they will",
                "they'd" => "they would",
                "it's" => "it is",
                "he's" => "he is",
                "he'll" => "he will",
                "he'd" => "he would",
                "she's" => "she is",
                "she'll" => "she will",
                "she'd" => "she would",
                "it'll" => "it will",
                "that's" => "that is",
                "let's" => "let us",
                "won't" => "will not",
                "can't" => "can not",
                _ => "",
            };
            if !expanded.is_empty() {
                expanded
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            } else if let Some(verb) = word.strip_suffix("n't") {
                vec![verb.to_owned(), "not".into()]
            } else {
                vec![word]
            }
        })
        .collect()
}

fn scoped_preposition(word: &str) -> bool {
    matches!(
        word,
        "to" | "of" | "in" | "on" | "at" | "for" | "from" | "with" | "by" | "as"
    )
}

fn grammatical_glue(word: &str) -> bool {
    scoped_preposition(word)
        || matches!(
            word,
            "a" | "an" | "the" | "am" | "is" | "are" | "was" | "were" | "be" | "been" | "being"
        )
}

fn prose_signature(
    text: &str,
    protected: &std::collections::HashSet<String>,
) -> (Vec<String>, Vec<(usize, String)>) {
    let words = expanded_prose_words(text);
    let mut content = Vec::new();
    let mut prepositions = Vec::new();
    for (index, word) in words.iter().enumerate() {
        if scoped_preposition(word) {
            prepositions.push((content.len(), word.clone()));
        } else if matches!(word.as_str(), "have" | "has" | "had") {
            // Possession is content. Only the explicit have + be auxiliary
            // chain can be treated as grammatical glue here.
            if !words
                .get(index + 1)
                .is_some_and(|next| matches!(next.as_str(), "be" | "been" | "being"))
            {
                content.push("have".into());
            }
        } else if matches!(word.as_str(), "do" | "does" | "did") {
            content.push("do".into());
        } else if !grammatical_glue(word) {
            content.push(if protected.contains(word) {
                word.clone()
            } else {
                agreement_form(&words, index)
            });
        }
    }
    (content, prepositions)
}

// Only normalize present-tense agreement in a narrow grammatical frame, never
// arbitrary noun/name suffixes. Unknown contexts and ed/ing forms stay exact.
fn agreement_form(words: &[String], index: usize) -> String {
    let word = &words[index];
    let previous = index.checked_sub(1).map(|i| words[i].as_str());
    let next = words.get(index + 1).map(String::as_str);
    // "to" also introduces recipients. Limit its agreement allowance to an
    // infinitive following a subject pronoun with a pronominal object.
    if previous == Some("to")
        && !(index.checked_sub(2).is_some_and(|i| {
            matches!(
                words[i].as_str(),
                "i" | "you" | "he" | "she" | "we" | "they"
            )
        }) && next.is_some_and(|word| {
            matches!(word, "me" | "you" | "him" | "her" | "it" | "us" | "them")
        }))
    {
        return word.clone();
    }
    if !previous.is_some_and(|word| {
        matches!(
            word,
            "i" | "you"
                | "he"
                | "she"
                | "it"
                | "we"
                | "they"
                | "to"
                | "can"
                | "could"
                | "should"
                | "will"
                | "would"
                | "must"
                | "shall"
                | "may"
                | "might"
        )
    }) || !next.is_some_and(|word| {
        scoped_preposition(word)
            || matches!(
                word,
                "me" | "you"
                    | "him"
                    | "her"
                    | "it"
                    | "us"
                    | "them"
                    | "my"
                    | "your"
                    | "his"
                    | "our"
                    | "their"
                    | "this"
                    | "that"
                    | "these"
                    | "those"
                    | "a"
                    | "an"
                    | "the"
            )
    }) {
        return word.clone();
    }
    if let Some(base) = word.strip_suffix("ies") {
        return format!("{base}y");
    }
    if let Some(base) = word.strip_suffix("es") {
        if ["ss", "sh", "ch", "x", "z", "o"]
            .iter()
            .any(|suffix| base.ends_with(suffix))
        {
            return base.to_owned();
        }
    }
    word.strip_suffix('s')
        .filter(|base| !base.ends_with('s'))
        .unwrap_or(word)
        .to_owned()
}

fn drops_label_words(original: &[String], candidate: &[String]) -> bool {
    if candidate.len() >= original.len() {
        return false;
    }
    let mut remaining = original.iter();
    candidate
        .iter()
        .all(|word| remaining.by_ref().any(|original| original == word))
}

fn preserves_enumerated_items(original: &str, candidate: &str) -> bool {
    use crate::formatting::normalizers::enumeration_groups;
    let Some(before) = enumeration_groups(original) else {
        return true;
    };
    if let Some(after) = enumeration_groups(candidate) {
        return before.len() == after.len()
            && before.iter().zip(after.iter()).all(|(before, after)| {
                before.items.len() == after.items.len()
                    && before
                        .items
                        .iter()
                        .zip(&after.items)
                        .all(|(before, after)| preserves_item_words(before, after))
                    && preserves_group_header(&before.header, &after.header)
                    && lexical_words(&before.tail) == lexical_words(&after.tail)
            });
    }
    // A short spoken enumeration may legitimately become ordinary prose.
    // Compare all prepared content, not just a subsequence: a subsequence would
    // still accept an invented extra object or an "instead of" substitution.
    let Some(content) = crate::formatting::normalizers::spoken_list_content(original) else {
        return false;
    };
    let lines: Vec<&str> = content.lines().collect();
    let last_item = lines
        .iter()
        .rposition(|line| strip_list_marker(line) != line.trim_start());
    let prose_words = |insert_separator: bool| {
        let mut words = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            // Only this structural conjunction can be introduced. The "and"
            // inside "rock and roll records" remains part of the item's name.
            if insert_separator && Some(index) == last_item {
                words.push("and".into());
            }
            words.extend(lexical_words(strip_list_marker(line)));
        }
        words
    };
    let candidate_words = lexical_words(candidate);
    prose_words(false) == candidate_words || prose_words(true) == candidate_words
}

fn preserves_group_header(original: &str, candidate: &str) -> bool {
    if original.trim().is_empty() || candidate.trim().is_empty() {
        return original.trim().is_empty() && candidate.trim().is_empty();
    }
    fn without_connector(text: &str) -> &str {
        let text = text.trim();
        match text.split_once(char::is_whitespace) {
            Some((first, rest)) if first.eq_ignore_ascii_case("and") => rest.trim_start(),
            _ => text,
        }
    }
    preserves_prose_words(without_connector(original), without_connector(candidate))
}

pub(crate) fn looks_like_meta(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    if lower.contains("```") {
        return true;
    }
    if lower.starts_with("here is") || lower.starts_with("here's the") {
        return true;
    }
    if lower.starts_with("cleaned text:") || lower.starts_with("cleaned transcript:") {
        return true;
    }
    let first = lower
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'');
    first == "sure"
}

/// Commands may legitimately change length, numbers and language.
pub fn accept_task_output(original: &str, candidate: &str) -> Option<String> {
    let text = candidate.trim();
    let lower = text.to_ascii_lowercase();
    let refused = [
        "i cannot",
        "i can't",
        "i am unable",
        "i'm unable",
        "as an ai",
        "sorry, i",
        "i’m unable",
        "i can’t",
    ];
    if text.is_empty()
        || looks_like_meta(text)
        || text.chars().count()
            > original
                .chars()
                .count()
                .saturating_mul(8)
                .saturating_add(2400)
                .min(32000)
        || refused.iter().any(|prefix| lower.starts_with(prefix))
        || lower.starts_with("instruction:")
        || lower.starts_with("text:")
    {
        None
    } else {
        Some(text.to_owned())
    }
}

/// `true` when the original speaks in the first person and the candidate has
/// dropped that voice entirely.
///
/// Dictation is the user's own sentence. A rewrite that removes the speaker
/// from it is not a cleanup, it is a paraphrase of someone else's words —
/// exactly what LFM2.5-1.2B produced when it turned "i need the key for
/// opening the door" into "The speaker is requesting the key to open a door."
/// The token-distance check alone waves that through, because the paraphrase
/// happens to share most of its vocabulary with the original.
fn drops_first_person(original: &str, candidate: &str) -> bool {
    const FIRST_PERSON: &[&str] = &[
        "i",
        "i'm",
        "i've",
        "i'll",
        "i'd",
        "me",
        "my",
        "mine",
        "myself",
        "we",
        "we're",
        "we've",
        "we'll",
        "we'd",
        "let's",
        "us",
        "our",
        "ours",
        "ourselves",
        "je",
        "nous",
        "ich",
        "wir",
        "yo",
        "nosotros",
        "eu",
        "nós",
        "ik",
        "wij",
        "io",
        "noi",
        "я",
        "мы",
        "मैं",
        "हम",
        "أنا",
        "نحن",
        "من",
        "ما",
        "saya",
        "kami",
        "aku",
        "ako",
        "ben",
        "biz",
        "我",
        "私",
        "저",
        "나",
        "ฉัน",
    ];
    let is_first_person = |text: &str| {
        tokenize(text)
            .iter()
            .any(|word| FIRST_PERSON.contains(&word.as_str()))
    };
    is_first_person(original) && !is_first_person(candidate)
}

/// `true` when the candidate talks *about* the speaker instead of being their
/// sentence.
fn reports_the_speaker(candidate: &str) -> bool {
    let lower = candidate.trim().to_ascii_lowercase();
    const REPORTING_OPENERS: &[&str] = &[
        "the speaker ",
        "the user ",
        "the person ",
        "speaker is ",
        "speaker wants ",
        "the transcript ",
        "the text ",
    ];
    REPORTING_OPENERS
        .iter()
        .any(|opener| lower.starts_with(opener))
}

fn has_negation(text: &str) -> bool {
    let words = tokenize(text);
    const NEGATIONS: &[&str] = &[
        "not",
        "never",
        "don't",
        "dont",
        "nicht",
        "kein",
        "keine",
        "pas",
        "jamais",
        "non",
        "nunca",
        "no",
        "não",
        "nunca",
        "niet",
        "geen",
        "ikke",
        "inte",
        "ej",
        "ei",
        "en",
        "nie",
        "není",
        "nikdy",
        "нет",
        "не",
        "никогда",
        "δεν",
        "όχι",
        "нема",
        "не",
        "नहीं",
        "मत",
        "tidak",
        "bukan",
        "tak",
        "jangan",
        "hindi",
        "huwag",
        "نہیں",
        "نمی",
        "نه",
        "نیست",
        "لا",
        "ليس",
        "لم",
        "لن",
        "hayır",
        "değil",
    ];
    if words
        .iter()
        .any(|w| NEGATIONS.contains(&w.as_str()) || w.ends_with("n't"))
    {
        return true;
    }
    if [
        "不",
        "没",
        "未",
        "無",
        "唔",
        "冇",
        "ない",
        "ません",
        "안",
        "않",
        "못",
        "ไม่",
        "không",
    ]
    .iter()
    .any(|marker| text.contains(marker))
    {
        return true;
    }
    words
        .windows(2)
        .any(|pair| pair[0] == "do" && pair[1] == "not")
}

fn tokenize(text: &str) -> Vec<String> {
    // Split CJK characters individually so punctuation edits do not replace an
    // entire whitespace-free utterance. Keep words in space-delimited scripts.
    let mut separated = String::with_capacity(text.len());
    for ch in text.chars() {
        if matches!(ch as u32, 0x3400..=0x9fff | 0x3040..=0x30ff | 0xac00..=0xd7af | 0x0e00..=0x0e7f)
        {
            separated.push(' ');
            separated.push(ch);
            separated.push(' ');
        } else {
            separated.push(ch);
        }
    }
    separated
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_lowercase()
                .replace('’', "'")
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn preserves_scripts(original: &str, candidate: &str) -> bool {
    let count = |text: &str| {
        let mut scripts = [0usize; 8];
        for ch in text.chars().filter(|c| c.is_alphabetic()) {
            let script = match ch as u32 {
                0x370..=0x3ff => Some(0),
                0x400..=0x52f => Some(1),
                0x600..=0x8ff => Some(2),
                0x900..=0x97f => Some(3),
                0x3400..=0x9fff => Some(4),
                0x3040..=0x30ff => Some(5),
                0xac00..=0xd7af => Some(6),
                0xe00..=0xe7f => Some(7),
                _ => None,
            };
            if let Some(script) = script {
                scripts[script] += 1;
            }
        }
        scripts
    };
    let original = count(original);
    let candidate = count(candidate);
    original
        .iter()
        .zip(candidate)
        .all(|(before, after)| *before < 2 || after > 0)
}

fn numeric_literals(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            // A line-leading "• "/"1. "/"2) " is list structure, not content:
            // without stripping it, turning spoken "first … second …" into a
            // numbered list reads as two invented numbers.
            strip_list_marker(line)
                .split(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | ':' | '-' | '/' | ','))
                .map(|s| s.trim_matches(|c: char| matches!(c, '.' | ':' | '-' | '/' | ',')))
                .filter(|s| s.chars().any(|c| c.is_numeric()))
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Enumeration labels describe structure; quantities inside items describe
/// meaning. Comparing them in order catches invented/dropped counts and counts
/// swapped between items, which the general token-distance check can miss.
/// Leave number wording intact just as the literal-number guard does for digits.
fn spoken_quantities(text: &str) -> Vec<String> {
    let content = crate::formatting::normalizers::spoken_list_content(text);
    tokenize(content.as_deref().unwrap_or(text))
        .into_iter()
        .flat_map(|word| word.split('-').map(str::to_owned).collect::<Vec<_>>())
        .filter(|word| {
            matches!(
                word.as_str(),
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
            )
        })
        .collect()
}

fn strip_list_marker(line: &str) -> &str {
    let s = line.trim_start();
    if let Some(rest) = s.strip_prefix(['•', '-', '*']) {
        if rest.starts_with(char::is_whitespace) {
            return rest.trim_start();
        }
    }
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && matches!(s.as_bytes().get(digits), Some(b'.') | Some(b')')) {
        let rest = &s[digits + 1..];
        if rest.starts_with(char::is_whitespace) {
            return rest.trim_start();
        }
    }
    s
}

fn preserves_identifiers(original: &str, candidate: &str) -> bool {
    fn token(word: &str) -> &str {
        word.trim_matches(|c: char| {
            matches!(
                c,
                '.' | ',' | ';' | '!' | '?' | '(' | ')' | '"' | '`' | '[' | ']'
            )
        })
    }
    let candidate_tokens: std::collections::HashSet<&str> =
        candidate.split_whitespace().map(token).collect();
    original.split_whitespace().all(|word| {
        let word = token(word);
        let camel_case = word
            .chars()
            .zip(word.chars().skip(1))
            .any(|(before, after)| before.is_lowercase() && after.is_uppercase());
        let technical = word.contains('@')
            || word.contains('/')
            || word.contains('.')
            || word.contains('_')
            || word.contains('\\')
            || (word.starts_with('-') && word.chars().any(|c| c.is_alphabetic()))
            || camel_case
            || word.chars().filter(|c| c.is_uppercase()).count() > 1;
        !technical || candidate_tokens.contains(word)
    })
}

fn levenshtein(a: &[String], b: &[String]) -> usize {
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_grounding_preserves_roles_names_and_command_arguments() {
        for (original, unsafe_edit) in [
            ("Send from Sarah to Pat:", "Send from Pat to Sarah:"),
            ("Send from Sarah to Pat.", "Send to Sarah from Pat."),
            ("I want the painting.", "I want the paint."),
            ("Please call Miles.", "Please call Mile."),
            ("• git add .", "• git add"),
            ("• Bread\n• Muffins", "For Bob:\n• Bread\n• Muffins"),
        ] {
            assert!(
                accept_rewrite(original, unsafe_edit, "high").is_none(),
                "{original:?} -> {unsafe_edit:?}"
            );
        }
    }

    #[test]
    fn explicit_line_and_paragraph_breaks_survive_rewriting() {
        for level in ["light", "medium", "high"] {
            assert!(accept_rewrite(
                "First paragraph.\n\nSecond paragraph.",
                "First paragraph. Second paragraph.",
                level
            )
            .is_none());
            assert!(accept_rewrite(
                "First line.\nSecond line.",
                "First line. Second line.",
                level
            )
            .is_none());
            assert!(accept_rewrite(
                "first paragraph\n\nsecond paragraph",
                "First paragraph.\n\nSecond paragraph.",
                level
            )
            .is_some());
        }
    }

    #[test]
    fn developer_identifiers_paths_and_flags_keep_their_exact_spelling() {
        for (original, candidate) in [
            ("please keep useState", "Please keep usestate."),
            ("please keep getUserById", "Please keep getUserByIdExtra."),
            ("please edit src/api.ts", "Please edit src/other.ts."),
            ("please use --dry-run", "Please use --force."),
        ] {
            assert!(
                accept_rewrite(original, candidate, "medium").is_none(),
                "{original} -> {candidate}"
            );
        }
        assert!(accept_rewrite(
            "please keep useState in src/api.ts with --dry-run",
            "Please keep `useState` in `src/api.ts` with `--dry-run`.",
            "medium"
        )
        .is_some());
    }

    #[test]
    fn rejects_changed_numbers_dates_and_technical_entities() {
        for (original, candidate) in [
            (
                "pay 125.50 on 2026-10-01 please",
                "Pay 125.60 on 2026-10-01, please.",
            ),
            ("meet at 3:00 with the team", "Meet at 4:00 with the team."),
            (
                "send to alice@example.com today",
                "Send to bob@example.com today.",
            ),
            (
                "call get_user_name with the result",
                "Call get_account_name with the result.",
            ),
        ] {
            assert!(
                accept_rewrite(original, candidate, "high").is_none(),
                "{original}"
            );
        }
        assert!(accept_rewrite(
            "pay 125.50 on 2026-10-01 please",
            "Pay 125.50 on 2026-10-01, please.",
            "high"
        )
        .is_some());
    }

    #[test]
    fn spoken_quantities_keep_their_items_and_allow_hyphenation() {
        assert!(accept_rewrite(
            "bring twenty one phones and two chargers",
            "Bring twenty-one phones and two chargers.",
            "medium"
        )
        .is_some());
        assert!(accept_rewrite(
            "first two phones second three chargers",
            "• Three phones\n• Two chargers",
            "high"
        )
        .is_none());
        assert!(accept_rewrite(
            "bring one of the mobiles and two power banks",
            "Bring one of the mobiles and two power banks.",
            "medium"
        )
        .is_some());
    }

    #[test]
    fn grammatical_repetition_and_emphasis_are_not_accidental_stutters() {
        assert!(accept_rewrite(
            "she had had a very very long day when I arrived",
            "She had had a very, very long day when I arrived.",
            "medium"
        )
        .is_some());
        for candidate in [
            "She had a very, very long day when I arrived.",
            "She had had a very long day when I arrived.",
        ] {
            assert!(
                accept_rewrite(
                    "she had had a very very long day when I arrived",
                    candidate,
                    "medium"
                )
                .is_none(),
                "{candidate}"
            );
        }
    }

    #[test]
    fn adjacent_phrase_restarts_accept_cleanup_without_changing_content() {
        for level in ["light", "medium", "high"] {
            for (original, candidate) in [
                (
                    "I was thinking we could we could ship on Thursday.",
                    "I was thinking we could ship on Thursday.",
                ),
                (
                    "I was thinking we could, we could ship on Thursday.",
                    "I was thinking we could ship on Thursday.",
                ),
                (
                    "we should review we should review we should review the release notes",
                    "We should review the release notes.",
                ),
                (
                    "I might send I might send the draft to Sarah.",
                    "I might send the draft to Sarah.",
                ),
                (
                    "Put the draft on the on the desk.",
                    "Put the draft on the desk.",
                ),
            ] {
                assert_eq!(
                    accept_rewrite(original, candidate, level).as_deref(),
                    Some(candidate),
                    "{level}: {original:?} -> {candidate:?}"
                );
                assert!(accept_rewrite(original, original, level).is_some());
            }
        }
    }

    #[test]
    fn phrase_restart_cleanup_keeps_emphasis_names_and_literal_repetition() {
        for (original, candidate) in [
            (
                "This is very good very good news.",
                "This is very good news.",
            ),
            ("I had told I had told Sarah.", "I had told Sarah."),
            (
                "I ordered a New York New York poster.",
                "I ordered a New York poster.",
            ),
            ("I ordered a Bora Bora poster.", "I ordered a Bora poster."),
            (
                "I wrote \"we could we could\" on the note.",
                "I wrote \"we could\" on the note.",
            ),
            (
                "We could ship. We could ship on Thursday.",
                "We could ship on Thursday.",
            ),
            (
                "We could ship; we could ship on Thursday.",
                "We could ship on Thursday.",
            ),
            (
                "Keep the New-York city NewYork city label.",
                "Keep the NewYork city label.",
            ),
            (
                "we could ship on Thursday",
                "We could we could ship on Thursday.",
            ),
        ] {
            assert!(
                accept_rewrite(original, candidate, "high").is_none(),
                "{original:?} -> {candidate:?}"
            );
        }
    }

    #[test]
    fn phrase_restart_cleanup_does_not_relax_raw_meaning_guards() {
        for (original, candidate) in [
            (
                "I was thinking we could we could ship on Thursday.",
                "I was thinking we will ship on Thursday.",
            ),
            (
                "I might send I might send the draft to Sarah.",
                "I send the draft to Sarah.",
            ),
            (
                "I might send I might send the draft to Sarah.",
                "I might send the draft to Pat.",
            ),
            (
                "I will send to Sarah I will send to Sarah from Pat.",
                "I will send from Sarah to Pat.",
            ),
            (
                "I need two keys I need two keys for the door.",
                "I need two keys for the door.",
            ),
            (
                "I need 2 keys I need 2 keys for the door.",
                "I need 2 keys for the door.",
            ),
            (
                "Please call getUserById please call getUserById with the result.",
                "Please call getuserbyid with the result.",
            ),
            (
                "I will not I will not ship the draft.",
                "I will ship the draft.",
            ),
            (
                "I need the key, for opening the door.",
                "I need the key to open the door.",
            ),
        ] {
            assert!(
                accept_rewrite(original, candidate, "high").is_none(),
                "{original:?} -> {candidate:?}"
            );
        }
    }

    #[test]
    fn accepts_cjk_punctuation_and_rejects_multilingual_negation_loss() {
        assert!(
            accept_rewrite("please यह send tomorrow", "Please send tomorrow.", "high").is_none()
        );
        assert!(accept_rewrite(
            "今天我们一起发布这个版本",
            "今天，我们一起发布这个版本。",
            "medium"
        )
        .is_some());
        for (original, candidate) in [
            ("मैं यह नहीं चाहता", "मैं यह चाहता"),
            ("我不想发布这个版本", "我想发布这个版本"),
            ("je ne veux pas publier", "je veux publier"),
            ("ich will das nicht senden", "ich will das senden"),
            ("no quiero publicar esto", "quiero publicar esto"),
            ("لا أريد نشر هذا", "أريد نشر هذا"),
        ] {
            assert!(
                accept_rewrite(original, candidate, "high").is_none(),
                "{original}"
            );
        }
    }

    /// Skipping the LLM has to be safe in one direction only: never skip text
    /// that still needs work. These are the cases where polishing genuinely has
    /// nothing to add, so the ~400 ms round trip is pure cost.
    #[test]
    fn already_clean_short_sentences_skip_the_llm() {
        for text in [
            "Ship it on Thursday.",
            "Can you review the PR?",
            "I need the key to open the door.",
            "Let's go!",
        ] {
            assert!(
                polish_would_add_nothing(text),
                "{text:?} needs no polish but was sent to the model"
            );
        }
    }

    /// Everything uncertain must reach the model. Each of these is a positive
    /// signal that Stage 1 has not finished.
    #[test]
    fn anything_unfinished_still_goes_to_the_llm() {
        for text in [
            // No terminal punctuation.
            "ship it on thursday",
            // Lowercase start.
            "ship it on Thursday.",
            // Residual filler.
            "Ship it on Thursday, um, maybe.",
            // Residual repair marker.
            "Ship it Friday I mean Thursday.",
            // Stranded punctuation from a deletion.
            "Ship it , on Thursday.",
            // Doubled space.
            "Ship it  on Thursday.",
            // Long enough that phrasing and sentence splitting matter.
            "we should probably ship this on thursday because the release notes are not ready and QA has not signed off yet.",
        ] {
            assert!(
                !polish_would_add_nothing(text),
                "{text:?} still needs polish but was skipped"
            );
        }
    }

    #[test]
    fn rejects_empty_meta_length_and_dropped_negation() {
        assert!(accept_rewrite("hello there", "   ", CleanupLevel::Medium).is_none());
        assert!(accept_rewrite(
            "hello",
            "Sure, here is the rewritten text:\n\nHello",
            "medium"
        )
        .is_none());
        assert!(accept_rewrite(
            "hi",
            "This is a much longer rewrite than the original should ever be allowed to become",
            "medium"
        )
        .is_none());
        assert!(accept_rewrite(
            "I do not want this shipped",
            "I want this shipped",
            CleanupLevel::Medium
        )
        .is_none());
        assert!(
            accept_rewrite("I don't want this shipped", "I want this shipped", "high").is_none()
        );
    }

    /// Real LFM2.5-1.2B outputs. Both share most of their vocabulary with the
    /// original, so the token-distance check accepted them, but neither is the
    /// user's sentence any more — the rewriter had turned dictation into a
    /// report about the dictation.
    #[test]
    fn rejects_third_person_reframing_of_the_speaker() {
        assert!(accept_rewrite(
            "i need the key, for opening the door",
            "The speaker is requesting the key to open a door.",
            "high"
        )
        .is_none());
        assert!(accept_rewrite(
            "i think we should ship this on friday",
            "The speaker mentioned considering shipping on Friday.",
            "high"
        )
        .is_none());
    }

    /// Losing the speaker's voice is a meaning change even without a
    /// "The speaker..." opener.
    #[test]
    fn rejects_rewrites_that_drop_the_first_person() {
        assert!(accept_rewrite("i need the key", "The key is needed.", "high").is_none());
        assert!(accept_rewrite("we should ship this", "Shipping is advised.", "high").is_none());
        // Text with no first person to begin with is unaffected.
        assert!(accept_rewrite(
            "ship this on friday please",
            "Ship this on Friday, please.",
            "high"
        )
        .is_some());
        // First person kept: fine.
        assert!(accept_rewrite("i need the key", "I need the key.", "high").is_some());
        // Contractions count as first person.
        assert!(accept_rewrite("im on my way", "I'm on my way.", "high").is_some());
    }

    /// Turning a spoken enumeration into a list introduces marker digits and
    /// bullets that are structure, not content — the number check must ignore
    /// them or every list rewrite would be rejected as an invented number.
    #[test]
    fn list_markers_are_not_counted_as_changed_numbers() {
        assert!(accept_rewrite(
            "first apples second bananas",
            "1. Apples\n2. Bananas",
            "high"
        )
        .is_some());
        assert!(accept_rewrite(
            "first update the readme second fix the bug",
            "• Update the readme\n• Fix the bug",
            "high"
        )
        .is_some());
        // A real number change is still caught.
        assert!(accept_rewrite("pay 125.50 please", "Pay 125.60 please.", "high").is_none());
    }

    /// Email layout can change: the rewrite only
    /// capitalises, punctuates and adds line breaks.
    #[test]
    fn email_layout_rewrite_is_accepted() {
        assert!(accept_rewrite(
            "hi sarah i wanted to follow up on the invoice can you send it by friday thanks john",
            "Hi Sarah,\n\nI wanted to follow up on the invoice. Can you send it by Friday?\n\nThanks,\nJohn",
            "high"
        )
        .is_some());
    }

    #[test]
    fn accepts_light_edits_and_trims() {
        let orig = "i think we should ship this today";
        let cand = "  I think we should ship this today.  ";
        assert_eq!(
            accept_rewrite(orig, cand, CleanupLevel::Medium).as_deref(),
            Some("I think we should ship this today.")
        );
    }

    #[test]
    fn high_cleanup_does_not_permit_unrelated_content_substitution() {
        let orig = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
        let cand = "alpha bravo charlie delta whisky xray yankee zulu lima mike";
        assert!(accept_rewrite(orig, cand, CleanupLevel::Medium).is_none());
        assert!(accept_rewrite(orig, cand, CleanupLevel::High).is_none());
    }
}
