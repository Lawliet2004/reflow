//! Script detection and a script-aware token estimate.
//!
//! The cleanup pipeline needs two cheap, offline answers about a transcript:
//! which writing system dominates it (so a translation can be detected) and
//! roughly how many model tokens it costs (so a request can be sized against
//! `n_ctx`). Latin text costs ~1.4 tokens per word; Devanagari and Bengali cost
//! several times more per word because the tokenizer falls back to fragments.

/// Tokens per Latin word. Deliberately above the ~1.3 average so a request that
/// looks like it fits really does.
pub const LATIN_TOKENS_PER_WORD: f32 = 1.4;
/// Tokens per Devanagari/Bengali character. A typical Hindi word is ~5
/// characters, so this is ~4-5 tokens per word against 1.4 for English. That is
/// deliberately pessimistic: overestimating only costs an extra chunk.
pub const INDIC_TOKENS_PER_CHAR: f32 = 1.0;
/// Tokens per CJK character.
pub const CJK_TOKENS_PER_CHAR: f32 = 1.5;
/// Tokens per character of any other non-ASCII script (Arabic, Cyrillic, ...).
pub const OTHER_TOKENS_PER_CHAR: f32 = 0.6;
/// A long unbroken Latin string (a path or URL) still costs about 1 token per
/// this many characters, however few "words" it contains.
const LATIN_CHARS_PER_TOKEN_FLOOR: f32 = 3.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Latin,
    Devanagari,
    Bengali,
}

pub fn is_devanagari(ch: char) -> bool {
    ('\u{0900}'..='\u{097F}').contains(&ch)
}

pub fn is_bengali(ch: char) -> bool {
    ('\u{0980}'..='\u{09FF}').contains(&ch)
}

fn is_cjk(ch: char) -> bool {
    matches!(ch, '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{AC00}'..='\u{D7AF}')
}

/// Letter counts per supported script. Digits and punctuation do not count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScriptCounts {
    pub latin: usize,
    pub devanagari: usize,
    pub bengali: usize,
}

impl ScriptCounts {
    pub fn of(text: &str) -> Self {
        let mut counts = Self::default();
        for ch in text.chars() {
            if is_devanagari(ch) && ch != '\u{0964}' && ch != '\u{0965}' {
                counts.devanagari += 1;
            } else if is_bengali(ch) && ch != '\u{09F7}' {
                counts.bengali += 1;
            } else if ch.is_alphabetic()
                && (ch.is_ascii() || ('\u{00C0}'..='\u{024F}').contains(&ch))
            {
                counts.latin += 1;
            }
        }
        counts
    }

    pub fn total(&self) -> usize {
        self.latin + self.devanagari + self.bengali
    }

    pub fn count(&self, script: Script) -> usize {
        match script {
            Script::Latin => self.latin,
            Script::Devanagari => self.devanagari,
            Script::Bengali => self.bengali,
        }
    }

    /// The script with the most letters, or `None` for text with no letters.
    pub fn dominant(&self) -> Option<Script> {
        if self.total() == 0 {
            return None;
        }
        [Script::Latin, Script::Devanagari, Script::Bengali]
            .into_iter()
            .max_by_key(|script| self.count(*script))
    }

    pub fn share(&self, script: Script) -> f32 {
        if self.total() == 0 {
            0.0
        } else {
            self.count(script) as f32 / self.total() as f32
        }
    }
}

/// Conservative token estimate for `text`, aware of Indic scripts.
pub fn estimate_tokens(text: &str) -> usize {
    let mut tokens = 0.0f32;
    for word in text.split_whitespace() {
        let mut latin_chars = 0usize;
        let mut other = 0.0f32;
        for ch in word.chars() {
            if is_devanagari(ch) || is_bengali(ch) {
                other += INDIC_TOKENS_PER_CHAR;
            } else if is_cjk(ch) {
                other += CJK_TOKENS_PER_CHAR;
            } else if ch.is_ascii() {
                latin_chars += 1;
            } else {
                other += OTHER_TOKENS_PER_CHAR;
            }
        }
        if latin_chars > 0 {
            tokens += LATIN_TOKENS_PER_WORD.max(latin_chars as f32 / LATIN_CHARS_PER_TOKEN_FLOOR);
        }
        tokens += other;
    }
    tokens.ceil() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dominant_script_follows_the_letters_not_the_punctuation() {
        assert_eq!(
            ScriptCounts::of("Send it to Priya 5:30 PM.").dominant(),
            Some(Script::Latin)
        );
        assert_eq!(
            ScriptCounts::of("कल हमें API को fix करना है।").dominant(),
            Some(Script::Devanagari)
        );
        assert_eq!(
            ScriptCounts::of("আমাদের কাল API fix করতে হবে").dominant(),
            Some(Script::Bengali)
        );
        assert_eq!(ScriptCounts::of("12:30 — !").dominant(), None);
    }

    #[test]
    fn indic_words_cost_several_times_an_english_word() {
        let english = estimate_tokens("we need to fix the deployment today");
        let hindi = estimate_tokens("हमें आज तैनाती को ठीक करना होगा");
        assert!((9..=13).contains(&english), "{english}");

        let per_english_word = english as f32 / 7.0;
        let per_hindi_word = hindi as f32 / 7.0;
        assert!(per_hindi_word > 1.8 * per_english_word);
    }

    #[test]
    fn a_long_unbroken_string_is_not_one_token() {
        assert!(estimate_tokens(&"a".repeat(1000)) >= 250);
    }
}
