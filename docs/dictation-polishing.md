# Faithful dictation polishing

Reflow treats cleanup as editing the speaker's text. A request remains a request;
a question remains a question; listed objects remain objects. Cleanup must not
answer the speaker, turn objects into actions, or invent quantities. The shared
LLM system prompt explicitly asks for grammatically correct, well-structured
writing in the input's own language and preserves each part of mixed-language
speech. Writing-task instructions select prose, paragraphs or lists.

## Reported regression

Input:

> I want you to bring me um uh uh three things that is one the mobile, second the power bank, third the earbuds.

The installed Qwen3.5 0.8B model reproduced this incorrect output before the fix:

> I want you to bring me three things: one of the mobile, one of the power bank, and one of the earbuds.

Normal dictation now prepares this inventory before model refinement:

```text
I want you to bring me three things:

1. The mobile
2. The power bank
3. The earbuds
```

Writing modes prepare a colon lead-in and three bullets. Ordinal/list markers are
structure; quantities within items, such as "two markers", remain content.

The follow-up comparison exposed two different stages. The matching local
history entry's ASR text was already:

> I want you to bring me these four things. First my mobile, second my earbuds, third my power.

The word "bank" was absent before deterministic formatting and model polishing.
No audio was retained for this recording, so its recognition failure cannot be
reproduced from the history. Cleanup preserves "My power" rather than guessing
an omitted noun. When ASR supplies "my power bank", the new preparation is:

```text
I want you to bring me these four things:

1. My mobile
2. My earbuds
3. My power bank
```

The stated count remains "four" but only the three named items are emitted.
Neither formatting nor an LLM fills in an unknown fourth object. The recorded
configuration uses Qwen3.5 0.8B with high cleanup in normal dictation; both
medium and high normal dictation now retain this numbered structure.

## Removing example-dependent behavior

The initial cleanup prompt included fixed demonstration turns, including the
earlier mobile/power-bank/earbuds dictation. These were real runtime prompt
content, not just test fixtures, and could bias a small model toward their
answers. They have been removed entirely. Each cleanup request now consists
of general system rules and the current dictation, with optional explicitly
enabled reference context limited to clarifying spelling. No demonstration
transcripts or answers enter the production prompt or cached prefix.

The subsequent bakery/electronics comparison was inspected in matching local
history. The bakery recording retained its prepared transcript after model
cleanup was rejected. The separate mobile recording already contained those
objects in its ASR text; this history does not show the bakery input being
replaced by the mobile example. The complex input exposed unsupported list
groups, which the formatter now recognizes using generic grammar boundaries.

An explicit ordinal reset after a completed group and a new scoped clause
starts a separate list. Both groups preserve their own header, complete items,
quantities and following requests. Numbering restarts within each normal-mode
group. The bakery example keeps its two cakes separate from its electronics
items. The ambiguous phrase "one new smartphone and laptop" remains intact,
without assigning an unstated quantity to the laptop. The same logic handles
unrelated locations and objects; quoted connectors and bracketed expressions
are protected. Unclear group boundaries retain the whole transcript.

## Evidence from other implementations

Wispr Flow and Superwhisper are proprietary. We reviewed their public behavior
and configuration documentation, not private code, prompts or training data.
This implementation aims at their documented editing behavior; it does not
establish equivalent quality.

- [Wispr Flow: Smart Formatting and Backtrack](https://docs.wisprflow.ai/articles/5373093536-How-do-I-use-Smart-Formatting-%26-Backtrack)
  describes filler removal, punctuation, grammar, list cues and clear corrections.
- [Wispr Flow: Context Awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness)
  describes nearby text, application context and vocabulary.
- [Superwhisper: Built-in Modes](https://superwhisper.com/docs/modes/built-in)
  separates transcription, message cleanup, note structure and contextual processing.
  Its [language model documentation](https://superwhisper.com/docs/models/language)
  confirms the text-processing stage follows transcription. Its
  [custom-mode guidance](https://superwhisper.com/docs/modes/custom) recommends
  explicit instructions and examples. The exact built-in prompts can be inspected
  in recording history according to its
  [customization documentation](https://superwhisper.com/docs/modes/customizing-modes);
  those recording-history prompts were not available for this review.

The relevant ASR-to-cleanup-to-delivery pipelines were traced in two open-source
projects, at these pinned revisions:

- OpenWhispr `f770e9211719a6e28d0578b480d8a23dea79d7ff`:
  [cleanup contract](https://github.com/OpenWhispr/openwhispr/blob/f770e9211719a6e28d0578b480d8a23dea79d7ff/src/locales/en/prompts.json),
  [prompt assembly](https://github.com/OpenWhispr/openwhispr/blob/f770e9211719a6e28d0578b480d8a23dea79d7ff/src/config/prompts/index.ts),
  [inference](https://github.com/OpenWhispr/openwhispr/blob/f770e9211719a6e28d0578b480d8a23dea79d7ff/src/services/ReasoningService.ts),
  [output validation](https://github.com/OpenWhispr/openwhispr/blob/f770e9211719a6e28d0578b480d8a23dea79d7ff/src/utils/cleanupOutput.ts),
  [delivery orchestration](https://github.com/OpenWhispr/openwhispr/blob/f770e9211719a6e28d0578b480d8a23dea79d7ff/src/helpers/audioManager.js).
  Cleanup and assistant behavior are separate; examples and a delimited transcript
  clarify the edit task; invalid or unavailable cleanup retains the transcript.
- VoiceInk `c09cc1f677f40f2ee665843a61f07670210f012e`:
  [transcription pipeline](https://github.com/Beingpax/VoiceInk/blob/c09cc1f677f40f2ee665843a61f07670210f012e/VoiceInk/Features/Recording/Workflows/TranscriptionPipeline.swift),
  [shared editing rules](https://github.com/Beingpax/VoiceInk/blob/c09cc1f677f40f2ee665843a61f07670210f012e/VoiceInk/Core/Enhancement/AIPrompts.swift),
  [worked examples](https://github.com/Beingpax/VoiceInk/blob/c09cc1f677f40f2ee665843a61f07670210f012e/VoiceInk/Features/Enhancement/Templates/PromptTemplates.swift),
  [enhancement workflow](https://github.com/Beingpax/VoiceInk/blob/c09cc1f677f40f2ee665843a61f07670210f012e/VoiceInk/Features/Enhancement/Workflows/AIEnhancementService.swift).
  Its default examples explicitly distinguish list positions from quantities,
  including a list item containing "two markers". Its contract preserves tone,
  certainty, names and values with minimal edits. These patterns informed Reflow's
  rules and independently written test cases; no third-party source or model weights
  were copied into Reflow.

## Algorithm and scope

1. Existing deterministic cleanup removes fillers and accidental restarts,
   resolves clear value corrections, and applies spelling/replacement rules.
2. A shared conservative parser recognizes sequential list markers, including
   mixed "one / second / third" after a counted lead-in. It avoids quoted markers,
   separate paragraphs, ordinary quantity prose and descriptive ordinals.
   Counted/explicit inventories in normal mode use numbered lines; writing
   modes use bullets. Comma inventories require an explicit lead-in and a colon
   or "that is" separator. Quoted/nested commas and numeric commas stay inside
   their item. Ambiguous comma prose is left unchanged.
3. The local model receives a compact, cacheable editing contract and the current
   transcript only. Rules preserve requests, listed objects, meaningful repetition,
   uncertainty, language, names, identifiers and quantities.
4. Existing list fragments get a one-line editing instruction; the validator
   rejects fragment expansion that could duplicate whole lists. The safety gate
   checks literal numbers, spoken cardinal quantities in order, meaningful
   grammatical/emphatic repetition, identifiers, scripts, first person, negation,
   complete item wording/order/count, following requests, structure, response leakage and
   edit distance. Confirmed enumeration markers are excluded from quantity checks.
   An invalid response returns the complete deterministic result with the existing
   cleanup error; it is never partially injected.

No extra model, dependency, cloud call or automatic download is introduced.
The prompt's stable prefix remains bounded by the existing 2,000-character test.
Raw mode still bypasses cleanup; coding mode retains its existing code handling.

These safeguards are conservative heuristics, not proof of semantic equivalence.
English spoken-number checks do not cover all languages or numeric expressions.
As with the existing literal-number check, changing number representation between
words and digits can fall back to the prepared transcript. Quantities tied to
different objects are not fully semantically verified. Model quality remains a
separate factor; this work does not change the configured model.

Confirmed list items have a conservative lexical check: preserve their complete
wording, allowing case, punctuation, compound hyphenation, hesitation removal and
incidental leading articles. This protects unfamiliar multiword names without a
dictionary of compounds. It can reject legitimate rephrasing or inflection
changes inside a full-clause item; the prepared transcript is retained in that
case. Short possessive labels also reject simple word deletion, while ordinary
sentence grammar substitutions remain possible. Ordinary prose additionally
retains its content words in order. Existing prepositions remain attached to
the same content positions, protecting source and destination roles. Articles,
auxiliaries, common contractions and narrowly contextual present-tense agreement
may change. Broad noun/name suffix stemming is prohibited, and capitalized words
retain their exact spelling apart from casing. A list without a header cannot
gain an invented recipient or scope. Standalone `.` and `..` command arguments
retain their positions as well as their wording. These are conservative lexical
heuristics with English grammar allowances, not general semantic verification.
Legitimate paraphrases, complex inflections and some corrections in other
languages may fall back.

## Model choice and fine-tuning

The first comparison should use Reflow's existing Qwen3.5 2B option against the
installed 0.8B, with the same representative cleanup corpus and runtime settings.
The [official Qwen card](https://huggingface.co/Qwen/Qwen3.5-2B) provides general
instruction-following benchmarks and an Apache-2.0 base for an owned fine-tune;
those benchmarks do not establish Reflow cleanup accuracy. The catalog's 2B
Q4_K_M download is approximately 1.28 GB; it is not installed in this workspace.

[VoiceInk Refine V1](https://huggingface.co/beingpax/VoiceInk-Refine-V1) is a
task-specific Qwen3.5-2B fine-tune for ASR cleanup. Its public card provides no
comparative or multilingual evaluation. Its
[license](https://huggingface.co/beingpax/VoiceInk-Refine-V1/blob/main/LICENSE.md)
requires permission for use in another application, so its weights are not
integrated into Reflow.

If the stronger baseline still fails, train an owned supervised LoRA adapter on
actual ASR outputs paired with human-checked, faithful cleanup. Include names,
negation, quantities, uncertainty, meaningful repetition, lists, long dictation
and Hindi-English code switching. Split the evaluation set by speaker/session
and exclude near duplicates. Measure lost/added meaning, grammar, structure,
fallback rate and latency separately. A fine-tuned text model cannot reliably
recover words absent from ASR without guessing.

The [Unsloth Qwen3.5 training guide](https://unsloth.ai/docs/models/qwen3.5/fine-tune)
currently advises bf16 LoRA rather than 4-bit QLoRA training and supports GGUF
export. Validate the final quantized model using the exact chat template and
Reflow runtime; keep the content safeguards and complete fallback.

## Verification

The reported faulty rewrite failed regression tests before the fix, and the
installed local model reproduced it through the real postprocessing path.
The deterministic/stub tests exercise valid prose and bullets, retained item
quantities, unsafe candidate fallback, negation, identifiers and explicit breaks.
An opt-in local-model corpus also checks corrections, grammatical repetition,
uncertainty, dictated commands, mixed-language text and writing-mode output.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib --test dictation_polish_quality --test task_prompts --test polish_toggle_tests --test integration_tests
cargo test --manifest-path src-tauri/Cargo.toml --test dictation_polish_live -- --ignored --nocapture
```

The local-model gate covers a raw-transcript diagnostic and the real production
path: fourteen medium examples, two high/normal inventories, a writing-mode
list, four consecutive unrelated recordings on one warm server, and the bakery
input on a newly started server. The bakery and stationery/hardware recordings
each retain two independent groups with numbering restarted. The bakery output
is compared between a reused and a new server; earlier mobile and stationery
items cannot leak into it.

Verified on 2026-10-06: 526 backend tests passed, with two existing environment
tests ignored. The final suite ran serially to avoid an intermittent unrelated
CUDA-capability state-test failure. All 23 local-model cases passed their content/structure or safe
fallback checks; this total includes the diagnostic and fresh-server comparison.

The broader [writing-quality audit](writing-quality-audit.md) checks paragraphs,
reviews, requested list styles, email layout, grammar, vocabulary, punctuation,
and meaning retention separately. Its installed-model run met 12 of 15 quality
criteria while all 15 outputs passed the content safeguards. Automatic topic
paragraphing, a run-on review, and an irregular-verb correction remained
uncorrected. Passing safety/fallback checks is not evidence of perfect grammar
or Wispr Flow parity.

The reported inventories produce complete numbered lists through the full
pipeline; writing mode produces exactly three bullets. The high/normal cases
test both complete "power bank" input and the actual history's ambiguous
"power" input. Complete formatted fallback is a valid production result when
the model's proposal is unsafe. It is reported with the existing error and is
never presented in these tests as successful model refinement. The 0.8B model
still proposes incorrect raw enumeration wording and unsafe changes on some
repetition, language and inventory cases. The validator retains the full
prepared result. This remains a model limitation after removing examples.

Rust formatting and whitespace checks passed. Strict Clippy passed for the
production library and both new cleanup test targets. The all-target check was
blocked by existing issues outside this change: `field_reassign_with_default` in
`tests/automatic_dictionary.rs:183` and `items_after_test_module` in
`src/calibration/mod.rs:454`. Those files were already changed before this task
and were preserved.

The live test requires the installed Qwen3.5 0.8B GGUF and llama-server. It asserts
their presence rather than silently passing when unavailable. This corpus checks
text postprocessing; it does not measure audio recognition accuracy or compare
outputs against private Wispr Flow or Superwhisper installations.
