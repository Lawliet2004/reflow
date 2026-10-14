# Reflow Linux smoothness and feature refactor

The user reports choppy scrolling/page changes and shortcut preferences that
do not match KDE's active assignment. They request SWE-2 Max implementation
workers, with the primary agent judging and validating changes.

## Working plan

1. Preserve the current dirty checkout in a separate source snapshot; inspect
   existing behavior and establish measurements before editing.
2. Delegate three independent areas to Devin CLI using `swe-2-max`: frontend
   rendering/scrolling/navigation; Linux shortcut assignment and recovery;
   hardware-aware speech/LLM behavior. Keep shared interfaces and integration
   under primary-agent review to prevent overlapping changes.
3. Compare public Wispr Flow behavior and OpenWhispr source/documentation for
   concrete mechanisms. Retain Reflow's design and features, honor licenses,
   and distinguish observed public behavior from unknown internal mechanisms.
4. Review each proposed simplification against existing cancellation, audio,
   privacy/offline, device preference, language, accessibility and cross-platform
   contracts. Remove complexity only when its purpose is understood.
5. Run focused regressions, required frontend/Rust/Python checks and production
   builds appropriate to the final changes. Compare the same workload/settings
   before and after; separate development overhead from release behavior.
6. Restart the app and complete user-assisted native checks for smooth scrolling,
   page switching, configurable shortcuts, dictation and relevant LLM/features.
   Report external hardware prerequisites and untested platforms accurately.

The original-source snapshot is `/tmp/reflow-smoothness-baseline-20261013`.
No unrelated user work may be reset, stashed, replaced, committed or published.
Saved choices and model weights are retained. Diagnostic logs will be kept
until the user explicitly confirms the app is smooth and working properly;
cleanup is deferred until that confirmation and will target this task's logs.

## Implementation status

The reviewed implementation, automated validation and optimized Tauri build are complete. Evidence and remaining live acceptance checks are in [linux-smoothness-verification.md](linux-smoothness-verification.md). Diagnostic cleanup remains deferred until the user confirms smoothness and functionality.
