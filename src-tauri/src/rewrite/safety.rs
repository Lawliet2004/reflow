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

    if numeric_literals(original) != numeric_literals(trimmed)
        || !preserves_identifiers(original, trimmed)
        || !preserves_scripts(original, trimmed)
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
    text.split(|c: char| !c.is_alphanumeric() && !matches!(c, '.' | ':' | '-' | '/' | ','))
        .map(|s| s.trim_matches(|c: char| matches!(c, '.' | ':' | '-' | '/' | ',')))
        .filter(|s| s.chars().any(|c| c.is_numeric()))
        .map(str::to_lowercase)
        .collect()
}

fn preserves_identifiers(original: &str, candidate: &str) -> bool {
    original.split_whitespace().all(|word| {
        let word =
            word.trim_matches(|c: char| matches!(c, '.' | ',' | ';' | '!' | '?' | '(' | ')' | '"'));
        let technical = word.contains('@')
            || word.contains("://")
            || word.contains('_')
            || word.contains('\\')
            || word.chars().filter(|c| c.is_uppercase()).count() > 1;
        !technical || candidate.to_lowercase().contains(&word.to_lowercase())
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
    fn high_allows_more_token_change_than_medium() {
        let orig = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
        let cand = "alpha bravo charlie delta whisky xray yankee zulu lima mike";
        assert!(accept_rewrite(orig, cand, CleanupLevel::Medium).is_none());
        assert!(accept_rewrite(orig, cand, CleanupLevel::High).is_some());
    }
}
