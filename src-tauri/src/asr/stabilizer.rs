#[derive(Debug, Default, Clone)]
pub struct TranscriptStabilizer {
    committed_prefix: String,
    mutable_suffix: String,
    history: Vec<String>,
}

impl TranscriptStabilizer {
    pub fn new(_stability_threshold: usize) -> Self {
        Self {
            committed_prefix: String::new(),
            mutable_suffix: String::new(),
            history: Vec::new(),
        }
    }

    /// Process a new incoming partial transcript from the streaming ASR model
    pub fn update(&mut self, new_text: &str) -> (&str, &str, String) {
        let trimmed = new_text.trim();
        if trimmed.is_empty() {
            return (
                &self.committed_prefix,
                &self.mutable_suffix,
                self.full_transcript(),
            );
        }

        self.history.push(trimmed.to_string());
        if self.history.len() > 10 {
            self.history.remove(0);
        }

        let words: Vec<&str> = trimmed.split_whitespace().collect();

        // If we have enough words, commit all words except the trailing 2-3 words
        if words.len() > 3 {
            let commit_count = words.len() - 2;
            let committed_words = &words[..commit_count];
            let mutable_words = &words[commit_count..];

            self.committed_prefix = committed_words.join(" ");
            self.mutable_suffix = mutable_words.join(" ");
        } else {
            self.committed_prefix.clear();
            self.mutable_suffix = trimmed.to_string();
        }

        (
            &self.committed_prefix,
            &self.mutable_suffix,
            self.full_transcript(),
        )
    }

    /// Finalize stream: commit everything into a unified string
    pub fn finalize(&mut self, final_text: &str) -> String {
        let trimmed = final_text.trim();
        let result = if !trimmed.is_empty() {
            trimmed.to_string()
        } else {
            self.full_transcript()
        };

        self.reset();
        result
    }

    pub fn full_transcript(&self) -> String {
        if self.committed_prefix.is_empty() {
            self.mutable_suffix.clone()
        } else if self.mutable_suffix.is_empty() {
            self.committed_prefix.clone()
        } else {
            format!("{} {}", self.committed_prefix, self.mutable_suffix)
        }
    }

    pub fn reset(&mut self) {
        self.committed_prefix.clear();
        self.mutable_suffix.clear();
        self.history.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stabilizer_progression() {
        let mut stabilizer = TranscriptStabilizer::new(2);

        let (c, m, _) = stabilizer.update("hello");
        assert_eq!(c, "");
        assert_eq!(m, "hello");

        let (c, m, _) = stabilizer.update("hello world this is a test");
        assert_eq!(c, "hello world this is");
        assert_eq!(m, "a test");

        let final_text = stabilizer.finalize("hello world this is a test today");
        assert_eq!(final_text, "hello world this is a test today");
    }
}
