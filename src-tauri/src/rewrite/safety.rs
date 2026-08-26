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

fn has_negation(text: &str) -> bool {
    let words = tokenize(text);
    if words.iter().any(|w| {
        w == "not" || w == "never" || w == "don't" || w == "dont" || w.ends_with("n't")
    }) {
        return true;
    }
    words.windows(2).any(|pair| pair[0] == "do" && pair[1] == "not")
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
            curr[j] = (prev[j] + 1)
                .min(curr[j - 1] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(accept_rewrite(
            "I don't want this shipped",
            "I want this shipped",
            "high"
        )
        .is_none());
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
