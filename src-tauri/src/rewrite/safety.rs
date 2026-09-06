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

fn looks_like_meta(text: &str) -> bool {
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
    if words
        .iter()
        .any(|w| w == "not" || w == "never" || w == "don't" || w == "dont" || w.ends_with("n't"))
    {
        return true;
    }
    words
        .windows(2)
        .any(|pair| pair[0] == "do" && pair[1] == "not")
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
                .to_ascii_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
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
        let cand = "alpha bravo charlie delta w1 w2 w3 w4 w5 w6";
        assert!(accept_rewrite(orig, cand, CleanupLevel::Medium).is_none());
        assert!(accept_rewrite(orig, cand, CleanupLevel::High).is_some());
    }
}
