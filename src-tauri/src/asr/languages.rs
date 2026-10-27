use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Language {
    code: String,
    name: String,
}

fn languages() -> &'static [Language] {
    static LANGUAGES: OnceLock<Vec<Language>> = OnceLock::new();
    LANGUAGES.get_or_init(|| {
        serde_json::from_str(include_str!("../../../model-runtime/languages.json"))
            .expect("compiled language catalogue must be valid")
    })
}

/// Shared with both language selectors and the Python sidecar. The names
/// match Qwen3-ASR's supported forced-language prompt tags.
pub fn language_name(code: &str) -> Result<Option<&'static str>, String> {
    if code == "auto" {
        return Ok(None);
    }
    languages().iter().find(|l| l.code == code).map(|l| Some(l.name.as_str()))
        .ok_or_else(|| format!("Unsupported dictation language '{code}'. Choose a supported language or Auto-detect."))
}

pub fn language_name_for_model(
    code: &str,
    model: Option<&str>,
) -> Result<Option<&'static str>, String> {
    if matches!(model, Some("phonon-2" | "zipformer-20m")) && !matches!(code, "auto" | "en") {
        return Err("This speech model supports English only. Choose English or Auto-detect, or select a multilingual speech model.".into());
    }
    if model != Some("indic-transcribe-8bit") || code == "auto" {
        return language_name(code);
    }
    static ADDITIONAL: OnceLock<serde_json::Value> = OnceLock::new();
    let models = ADDITIONAL.get_or_init(|| {
        serde_json::from_str(include_str!(
            "../../../model-runtime/additional-models.json"
        ))
        .expect("compiled additional model metadata")
    });
    models["indic-transcribe-8bit"]["languages"].as_array().into_iter().flatten()
        .find(|language| language["code"].as_str() == Some(code))
        .and_then(|language| language["name"].as_str()).map(Some)
        .ok_or_else(|| format!("Indic Transcribe does not support language '{code}'. Choose a supported language or Auto-detect."))
}

pub fn resolve_language_name(input: &str) -> Result<String, String> {
    let input = input.trim();
    languages()
        .iter()
        .find(|l| l.code.eq_ignore_ascii_case(input) || l.name.eq_ignore_ascii_case(input))
        .map(|l| l.name.clone())
        .ok_or_else(|| format!("Unsupported translation language '{input}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_language_codes_map_to_canonical_prompt_names() {
        let languages: Vec<Language> =
            serde_json::from_str(include_str!("../../../model-runtime/languages.json")).unwrap();
        assert_eq!(languages.len(), 30);
        for language in languages {
            assert_eq!(
                language_name(&language.code).unwrap(),
                Some(language.name.as_str())
            );
        }
        assert_eq!(language_name("auto").unwrap(), None);
        assert!(language_name("bn").is_err());
        assert!(language_name("English").is_err());
    }

    #[test]
    fn resolves_codes_and_names_case_insensitively() {
        assert_eq!(resolve_language_name(" EN ").unwrap(), "English");
        assert_eq!(resolve_language_name("german").unwrap(), "German");
        assert!(resolve_language_name("tlh").is_err());
    }

    #[test]
    fn english_only_models_reject_incompatible_stream_languages() {
        for model in ["phonon-2", "zipformer-20m"] {
            assert!(language_name_for_model("hi", Some(model))
                .unwrap_err()
                .contains("English only"));
            assert_eq!(
                language_name_for_model("en", Some(model)).unwrap(),
                Some("English")
            );
            assert_eq!(language_name_for_model("auto", Some(model)).unwrap(), None);
        }
    }

    #[test]
    fn indic_languages_do_not_expand_qwen_prompt_tags() {
        assert_eq!(
            language_name_for_model("bn", Some("indic-transcribe-8bit")).unwrap(),
            Some("Bengali")
        );
        assert!(language_name_for_model("bn", Some("0.6b")).is_err());
        assert!(language_name_for_model("ja", Some("indic-transcribe-8bit")).is_err());
    }
}
