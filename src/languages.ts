import languages from "../model-runtime/languages.json";

// Qwen3-ASR's canonical forceable languages. Both shipped Qwen3.5 tiers
// provide multilingual text cleanup; that does not establish equal accuracy.
export const DICTATION_LANGUAGES: readonly { code: string; name: string }[] = languages;

export function isDictationLanguage(code: string): boolean {
  return code === "auto" || DICTATION_LANGUAGES.some((language) => language.code === code);
}
