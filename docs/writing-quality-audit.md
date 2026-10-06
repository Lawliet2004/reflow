# Writing quality audit

Audited on 2026-10-06. Scope: the live dictation text-postprocessing path,
including deterministic formatting, the local cleanup prompt, model request
segmentation, validation, vocabulary restoration, and complete fallback.

## Hard-coding audit

The runtime cleanup prompt contains general editing instructions and the current
dictation. It contains no shopping answers, bakery examples, or demonstration
conversations. Those examples exist in regression tests; they are excluded from
production by Rust's test configuration. UI before/after examples are display
content, not model inputs. Each cleanup request builds its own messages.

There are still explicit, general language rules: spoken punctuation names,
English fillers, enumeration markers, email boundary words, canonical technical
spellings, and content-validation rules. User-configured vocabulary and aliases
also perform literal replacement. It would be inaccurate to describe the
implementation as containing no fixed rules at all.

The old Decisive rule changed wishes into commitments and dropped uncertainty
before validation. It has been removed. Decisive now requests a clear tone while
preserving intent and uncertainty, and its fallback retains the original wish.

## Requested writing features

| Feature                 | Implementation and verification                                                                                                                                                                                                                | Remaining limit                                                                                                                                                                 |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Paragraphs              | Explicit spoken line/paragraph breaks survive formatting and model segmentation. Valid model-added paragraphs are accepted.                                                                                                                    | The installed model did not separate topics in the automatic-paragraph probe.                                                                                                   |
| Reviews                 | The prompt requests connected prose and preserves opinions, qualifications, and ratings. Tests accept valid review paragraphing and reject opinion/negation changes.                                                                           | A run-on review remained uncorrected after its model proposal was rejected. There is no generated review template or invented recommendation.                                   |
| Bullet points           | Clear enumerations become bullets in writing/email mode. An explicit bullet request overrides the normal default. Tests preserve item names, quantities, and separate groups.                                                                  | Ambiguous prose stays prose; unsupported enumeration wording can fall back.                                                                                                     |
| Numbered points         | Clear normal-mode enumerations use numbering. Explicit numbering overrides writing-mode bullets, and separate groups restart numbering. A model cannot number an existing heading.                                                             | A spoken count mismatch is preserved rather than inventing a missing item or guessing a new count.                                                                              |
| Email writing           | Clear existing greetings and terminal sign-offs are separated using copied words and newlines. Email layout instructions apply even to short Light-mode text. Existing paragraphs/lists remain intact.                                         | Boundary rules recognize common English email phrases. Ambiguous or other-language layouts depend on the model. Subjects, recipients, signatures, and facts are never invented. |
| Grammar                 | The prompt requests grammar repair in the input language. Tests and local inference verify basic agreement and regular-verb fixes.                                                                                                             | Irregular verbs, complex inflections, contractions, and legitimate paraphrases may be rejected by conservative validation.                                                      |
| Vocabulary              | ASR hotwords, enabled custom aliases, preferred spellings, and final canonical-case restoration are integrated. Alias chains are not replayed after model cleanup. Paths and email identifiers are protected.                                  | This is spelling/term preservation, not automatic synonym enrichment. Missing ASR words cannot be recovered reliably without guessing.                                          |
| Sentences               | Sentence casing and punctuation preserve dotted addresses, decimals, paths, abbreviations, and multiline quotations. The prompt asks to split run-on sentences.                                                                                | The small model does not consistently repair run-ons, and punctuation inference is heuristic.                                                                                   |
| Comma and semicolon     | Spoken comma/semicolon commands work, including “semi colon” and an ASR-appended period. Quoted command names and ordinary noun phrases are protected. Model-inserted commas were verified.                                                    | Ambiguous unquoted punctuation words remain heuristic. Automatic semicolon choice is model-dependent.                                                                           |
| Overall writing         | Cleanup intensity, writing mode, and voice register have separate instructions. No specimen answer is cached. Segmented requests edit only their current fragment, and the assembled candidate is validated again.                             | This is faithful editing of dictated content, not free-form composition or a guarantee of publication-ready writing.                                                            |
| User's intended meaning | Guards cover content order, names, item wording, quantities, source/destination roles, possession, negation, uncertainty, scripts, identifiers, and group boundaries. Tests reject possession-to-identity and actor-reversing passive changes. | These are conservative lexical safeguards, not general semantic understanding or a proof of equivalence.                                                                        |

## Evaluation

`writing_structure_quality.rs` exercises these categories through the actual
postprocessing path using controlled model responses where necessary. It tests
both valid corrections and rejected changes. Model stubs prove pipeline behavior;
they do not prove a model can produce those corrections.

`writing_structure_live.rs` uses the installed Qwen3.5 0.8B Q4 model and local
llama-server. Its probes report `quality_met`, `model_used`, and
`safety_accepted` separately. Complete fallback can preserve meaning and layout
without fixing grammar; it is never counted as successful refinement merely
because validation passed. Exact inputs, prepared text, final outputs, and
fallback errors are saved in [writing-quality-live.json](writing-quality-live.json).

Final verification: **526 backend tests passed**, with two existing environment
tests ignored. This includes 21 writing-category regressions. The final suite
ran with one test thread after a parallel run intermittently failed the unrelated
shared CUDA-capability state test; that test also passed in isolation. No
capability code was changed.

Strict Clippy passed for the production library and the four dictation/writing
test targets. Rust formatting and scoped whitespace checks passed. No model
download, cloud-model integration, or frontend change was needed for this audit.

The local model met **12 of 15 targeted quality probes**; all 15 final outputs
passed the existing content safeguards. Automatic topic paragraphing, the run-on
review, and the irregular-verb correction remained below the probe's quality
criterion. The separate existing dictation corpus also passed its content and
structure checks, including independent recordings and a fresh-server comparison.
The direct model diagnostic still produced incorrect inventory wording; the
production pipeline retained complete, correctly formatted fallback.

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib --test writing_structure_quality --test dictation_polish_quality --test task_prompts --test polish_toggle_tests --test integration_tests -- --test-threads=1
$env:REFLOW_WRITING_AUDIT_PATH = Join-Path (Get-Location) 'docs/writing-quality-live.json'
cargo test --manifest-path src-tauri/Cargo.toml --test writing_structure_live --test dictation_polish_live -- --ignored --nocapture
```

These are targeted text-postprocessing checks, not audio recognition benchmarks,
multilingual quality certification, or a measured comparison against Wispr Flow.
Raw mode intentionally bypasses cleanup. The deterministic language rules and
grammar allowances remain predominantly English; one mixed Hindi-English probe
does not establish general multilingual quality. Meaningful foreign-language
words that resemble English fillers remain a known limitation of filler removal.
File-transcription jobs have a separate ASR result path; this audit does not claim
that the live cleanup pipeline is applied to those jobs.

The implementation supports all requested categories to varying degrees. The
installed small model and conservative validator still leave genuine writing
quality gaps. Improving those requires a representative, human-reviewed
evaluation set and model comparison; adding more fixed sample answers is not a
substitute. See [dictation-polishing.md](dictation-polishing.md) for the prior
public-source research and model/fine-tuning considerations.
