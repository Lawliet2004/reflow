use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Deserialize)]
struct Language {
    code: String,
    name: String,
}

/// Shared with both language selectors and the Python sidecar. The names
/// match Qwen3-ASR's supported forced-language prompt tags.
pub fn language_name(code: &str) -> Result<Option<&'static str>, String> {
    if code == "auto" {
        return Ok(None);
    }
    static LANGUAGES: OnceLock<Vec<Language>> = OnceLock::new();
    let languages = LANGUAGES.get_or_init(|| {
        serde_json::from_str(include_str!("../../../model-runtime/languages.json"))
            .expect("compiled language catalogue must be valid")
    });
    languages.iter().find(|l| l.code == code).map(|l| Some(l.name.as_str()))
        .ok_or_else(|| format!("Unsupported dictation language '{code}'. Choose a supported language or Auto-detect."))
}

pub fn resolve_language_name(input: &str) -> Result<String, String> {
    let languages: Vec<Language> =
        serde_json::from_str(include_str!("../../../model-runtime/languages.json"))
            .map_err(|e| e.to_string())?;
    languages
        .into_iter()
        .find(|l| {
            l.code.eq_ignore_ascii_case(input.trim()) || l.name.eq_ignore_ascii_case(input.trim())
        })
        .map(|l| l.name)
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
}
