use super::engine::ASREngine;

pub struct MockASREngine {
    is_loaded: bool,
    language: String,
    current_stream_text: Vec<String>,
    sample_count: usize,
    sample_phrases: Vec<&'static str>,
    phrase_index: usize,
}

impl MockASREngine {
    pub fn new() -> Self {
        Self {
            is_loaded: true,
            language: "en".to_string(),
            current_stream_text: Vec::new(),
            sample_count: 0,
            sample_phrases: vec![
                "I need to finish this API implementation tomorrow because the deployment is scheduled for Monday.",
                "Let's check the database migration and verify all table indexes before pushing to production.",
                "Aaj humein frontend aur backend integration ko test karna hai.",
                "Ajke amader office jete hobe project submit korar jonno.",
                "Reflow local dictation using Qwen3-ASR with zero cloud dependency.",
            ],
            phrase_index: 0,
        }
    }
}

impl ASREngine for MockASREngine {
    fn initialize(&mut self) -> Result<(), String> {
        self.is_loaded = true;
        Ok(())
    }

    fn load_model_with_precision(
        &mut self,
        _model_dir: &str,
        _backend: &str,
        _precision: &str,
    ) -> Result<(), String> {
        self.is_loaded = true;
        Ok(())
    }

    fn unload_model(&mut self) -> Result<(), String> {
        self.is_loaded = false;
        Ok(())
    }

    fn is_model_loaded(&self) -> bool {
        self.is_loaded
    }

    fn start_stream(&mut self, language: &str, _vocabulary: &[String]) -> Result<(), String> {
        self.language = if language == "auto" { "en".into() } else { language.to_string() };
        self.sample_count = 0;
        self.current_stream_text.clear();
        Ok(())
    }

    fn push_audio(&mut self, samples_16k_mono: &[f32]) -> Result<Option<String>, String> {
        self.sample_count += samples_16k_mono.len();

        let phrase = self.sample_phrases[self.phrase_index % self.sample_phrases.len()];
        let words: Vec<&str> = phrase.split_whitespace().collect();

        // Reveal words proportionally to the audio received (every 3200 samples = 200ms)
        let words_to_show = ((self.sample_count / 3200) + 1).min(words.len());
        let current_text = words[..words_to_show].join(" ");

        Ok(Some(current_text))
    }

    fn get_partial_transcript(&mut self) -> Result<String, String> {
        let phrase = self.sample_phrases[self.phrase_index % self.sample_phrases.len()];
        let words: Vec<&str> = phrase.split_whitespace().collect();
        let words_to_show = ((self.sample_count / 3200) + 1).min(words.len());
        Ok(words[..words_to_show].join(" "))
    }

    fn stop_stream(&mut self) -> Result<String, String> {
        let phrase = self.sample_phrases[self.phrase_index % self.sample_phrases.len()];
        self.phrase_index += 1;
        self.sample_count = 0;
        Ok(phrase.to_string())
    }

    fn cancel_stream(&mut self) -> Result<(), String> {
        self.sample_count = 0;
        self.current_stream_text.clear();
        Ok(())
    }

    fn get_detected_language(&self) -> String {
        self.language.clone()
    }

    fn get_backend_name(&self) -> String {
        "Mock Engine (Test Runtime)".to_string()
    }
}
