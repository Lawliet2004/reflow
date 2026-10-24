import languages from "../model-runtime/languages.json";
import additional from "../model-runtime/additional-models.json";

// Qwen3-ASR's canonical forceable languages. Both shipped Qwen3.5 tiers
// provide multilingual text cleanup; that does not establish equal accuracy.
export const DICTATION_LANGUAGES: readonly { code: string; name: string }[] = languages;

export function dictationLanguagesForModel(model?: string) {
  return model === "indic-transcribe-8bit"
    ? additional["indic-transcribe-8bit"].languages
    : DICTATION_LANGUAGES;
}
