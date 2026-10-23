# Distinguish offline mode from failed policy persistence

> The later [Qwen/readiness and storage checkpoint](runtime-pack-qwen-linux-2026-10-23/README.md) holds newer whole-worktree checks. This policy-message report identifies its earlier source checkpoint.

23 October 2026. The Windows/macOS/Linux implementation goal remains incomplete. This change fixes a first-run/recovery diagnostic without changing download authorization, model selection, settings fields or Tauri IPC.

## Confirmed failure and behavior

After an online policy-file write failed, the existing Rust download gate correctly stayed closed but reported `Offline mode is on`. Users whose download preference was already enabled could therefore be directed to enable it again. The Python setup monitor also replaced every policy error with an airplane-mode explanation.

The existing single atomic gate now represents online, deliberate offline and failed-save states. Admission and diagnostic are read in one snapshot. Offline revokes immediately; online is granted only after the policy write succeeds. Failed persistence and unknown states deny downloads. Successful repair restores the requested online/offline state through the existing configuration path.

Failed saves now report:

> Downloads are unavailable because Reflow could not save its network policy. Check that Reflow's data folder is writable, then retry saving the download setting.

The setup subprocess monitor retains the actual policy reason instead of relabeling it as airplane mode. The existing ASR owned-process revocation/reload path and settings recovery remain responsible for lifecycle handling; no second policy store or orchestration layer was added.

Changed sources:

- `src-tauri/src/network_policy.rs`: one atomic admission/error state; failed-save and successful-repair assertions against real invalid filesystem paths.
- `src-tauri/src/network_policy/download.rs`: extend the stalled-body fixture with a real policy-save failure while bytes are in flight; assert the actionable error, retained partial bytes, unchanged incumbent and no activated destination.
- `src-tauri/src/asr/python_setup.rs`: preserve the policy error in setup monitoring.

The failed-save diagnostic regression failed before the fix. Focused policy/transfer tests pass, including offline/cancellation revocation and resumable-byte preservation. The fixtures use isolated directories, a local gate and loopback HTTP; they do not mutate the production global gate, download models or prove the complete packaged settings journey.

## Verification scope

Final locked Rust checks pass **982 tests, 27 ignored** across 45 targets; library **638 passed/16 ignored**. All-target Clippy with `-D warnings` passes. The separate real cached Zipformer dependency-pack run passes: **16.728 seconds** for installation, **36.14 seconds** for the complete cancellation/corruption/missing-cache/offline-repair sequence, zero model downloads/network-journal entries, simulated AppImage and invalid inherited/original Python homes. Formatting and scoped source/check hashes are recorded in the adjacent provenance. These are local observations; this fixture does not decode speech or prove a clean AppImage installation. The [paragraph-preservation refinement evidence](refinement-2026-10-23/paragraph-preservation/README.md) still describes the unchanged production cleanup pipeline. An additional [100-case controlled instruction comparison](refinement-2026-10-23/rejected-punctuation/README.md) was rejected; no prompt change was adopted.

Native installer/device/process-tree acceptance, the complete runtime-pack matrix, broader crash/disk faults, 15 allowed Rust advisory warnings, final visual verification and eight substantive writing-quality cases remain open in [the active tracker](../implementation-progress.md). No release was published.
