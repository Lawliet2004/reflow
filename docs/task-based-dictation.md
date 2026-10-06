# Task-based dictation

## Goal

Let a new user choose the writing task before tuning models. Natural cleanup is the everyday recommendation: repair speech without making it sound like someone else.

## Implementation plan

1. Preserve explicit line and paragraph breaks through rule cleanup; retain meaningful repeated words.
2. Give light, natural and polished cleanup different rewrite instructions. Add a developer-prompt context that preserves requirements and identifiers without answering the prompt.
3. Add a shared task picker on Home, Writing settings, first-run setup and the mode editor. Reuse existing settings, tiers and local models; do not replace saved custom preferences.
4. Explain speech recognition (ASR) separately from text cleanup (LLM). Show task examples, model recommendations, installation state and the no-LLM fallback. Never download or change the speech model merely because a task was selected.
5. Verify formatting, task selection, prompt contracts, fallback behavior, existing tests, build, lint and browser layout.

## Task contract

| Task             | Cleanup          | Context          | Suggested LLM |
| ---------------- | ---------------- | ---------------- | ------------- |
| Quick dictation  | Light rules      | Normal           | None          |
| Natural cleanup  | Medium, faithful | Normal           | Qwen3.5 0.8B  |
| Developer prompt | Medium, faithful | Developer prompt | Qwen3.5 2B    |
| Polished writing | High, faithful   | Notes            | Qwen3.5 2B    |
| Original words   | Raw              | Normal           | None          |

Task selection applies cleanup, context and tone together. Model size remains independently selectable. Literal code dictation remains a separate Coding context with no AI rewrite. App and shortcut modes may override the default task.

## Recognition guidance

Start with Qwen3-ASR 0.6B for everyday dictation. Try 1.7B when names, technical terms or mixed-language speech are being misheard and memory permits. Zipformer 20M is a compact English CPU streaming option; Phonon-2 is another English-only alternative. These English adapters do not support recognition hotword bias. Dictionary hints improve Qwen recognition; preferred-spelling replacements also run after cleanup. Actual latency and accuracy depend on hardware and recordings.

## Fidelity and failure behavior

Keep meaning, voice, negations, numbers, technical identifiers and input language. Resolve explicit corrections and accidental repeated phrases; preserve emphasis. Improve awkward wording only when context supports it. Do not invent facts, missing requirements or an ambiguous intended word. Writing may create paragraphs and lists, but never expand a transcript into a new essay.

The existing safety gate and all-or-nothing fallback remain active. Missing models, timeouts, truncation or rejected rewrites return the complete rule-cleaned transcript and surface a reason. Explicit line/paragraph breaks are hard boundaries: cleanup edits each section separately, retaining its separators. Oversized sections split at conservative sentence boundaries under the existing total deadline, including when context is attached. Paths, decimals, common abbreviations and initials stay together; a section without a safe boundary falls back. This can limit corrections spanning segments; excessive context or too many sections can still cause a complete fallback.

## Compatibility

Reuse persisted cleanup/style/tier fields; add only the developer-prompt context value. Fresh installs start with Natural cleanup and a faithful voice. Existing preferences and custom modes stay intact until a task is explicitly selected; legacy configurations missing the newer fields retain their compatibility defaults. Selecting a Home task returns the default mode to Dictation and clears its custom instructions, translation and captured context so the visible choice is applied. Other modes retain their preferences. No new model dependencies or database migrations.

## Model references

The recognition guidance follows the official [Qwen3-ASR model card](https://huggingface.co/Qwen/Qwen3-ASR-0.6B). The local cleanup choices are [Qwen3.5 0.8B](https://huggingface.co/Qwen/Qwen3.5-0.8B) and [Qwen3.5 2B](https://huggingface.co/Qwen/Qwen3.5-2B). Task recommendations are application defaults to try, not measured accuracy guarantees.

## Validation

- Frontend: 221 tests across 42 files passed. ESLint, TypeScript and the Vite production build passed. Vite reported its bundle-size warning for the 500.5 kB main JavaScript bundle.
- Backend: the serialized unit and integration suite passed. After the final sentence-boundary guard, the affected suite passed again: 384 library tests and 7 live runtime tests, with 2 existing library tests ignored. The optional Phonon audio-fixture test remains ignored. The ASR loaded-status convergence check passed separately and was excluded from repeated full runs.
- Live cleanup used the installed Qwen3.5 0.8B runtime. Natural cleanup removed a repeated phrase; the developer case retained its identifiers, path, command flag and negation through an accepted edit or complete fallback. Qwen3.5 2B was not installed and was not tested live.
- Real browser checks covered task selection, model guidance, explicit browser-preview fallback, and layouts at 430 px and 1100 px. No console errors or horizontal overflow were found in the new task controls. Browser previews do not perform local inference.
- A separate read-only code review checked fidelity, context handling, compatibility and segment boundaries. Tests and illustrative examples do not establish a broad model-quality benchmark.
