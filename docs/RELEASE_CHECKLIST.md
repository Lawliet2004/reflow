# Reflow Adaptive Local Inference — Hardware Validation Matrix & Release Sign-Off

> **Status of this document:** updated 2026-08-30 after an end-to-end
> verification pass on the development machine (RTX 2050, 4096 MiB, 6 cores,
> 16 GB RAM). Claims below are backed by named commands and logs. Anything a
> human still has to verify by hand is marked UNVERIFIED rather than assumed.

## 1. Hardware Validation Matrix (Classes A–E)

| Class       | Hardware Configuration                                 | ASR Selection                                                                  | Refinement Selection                         | Evidence                                                                                                                                                                                                                                                                                      | Status                              |
| :---------- | :----------------------------------------------------- | :----------------------------------------------------------------------------- | :------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | :---------------------------------- |
| **Class A** | CPU-only (4 cores, 8 GB RAM)                           | Qwen3-ASR 0.6B (CPU / fp32)                                                    | CPU / none                                   | Unit tests (`asr_selection_tests.rs` — CPU paths) and resolver ladder; **no physical CPU-only machine has been measured**                                                                                                                                                                     | **UNVERIFIED** (code-verified only) |
| **Class B** | RTX 2050 (4 GB VRAM, 6 cores, 16 GB RAM) — dev machine | Qwen3-ASR 1.7B (CUDA / int8), steps down to 0.6B int8 when free VRAM < ~2.1 GB | Qwen3.5 0.8B / 2B, partial offload by budget | **Measured 2026-08-30**: `reflow.log` — `ASR model selection: 1.7b on cuda (int8)`; `diag_both.py 1.7b Qwen3.5-2B gpu` — both resident at 50–64 MiB free, ASR 2682 ms for 7.3s audio (RTF 0.367), LLM 336 ms; `diag_flow.py gpu` — 0.54 s rewrite; WS stream test — RTF 0.259, rewrite 445 ms | **PASS (measured)**                 |
| **Class C** | 6 GB VRAM GPU, 8 cores, 16 GB RAM                      | 0.6B / 1.7B int8                                                               | Qwen3.5 0.8B full GPU offload                | Resolver unit tests only (`profile_tests.rs`, `spill_tests.rs`); **no such machine available to test**                                                                                                                                                                                        | **UNVERIFIED**                      |
| **Class D** | 8 GB VRAM GPU, 12 cores, 32 GB RAM                     | 1.7B (CUDA)                                                                    | Qwen3.5 2B (full GPU offload)                | None — no such machine available to test                                                                                                                                                                                                                                                      | **UNVERIFIED**                      |
| **Class E** | 24 GB workstation (RTX 3090/4090, 64 GB)               | 1.7B (CUDA)                                                                    | Qwen3.5 2B (full GPU offload)                | None — no such machine available to test                                                                                                                                                                                                                                                      | **UNVERIFIED**                      |

### Measured latency on the Class B machine (2026-08-30)

| Path                                        | Measurement (command)                                                       |
| ------------------------------------------- | --------------------------------------------------------------------------- |
| ASR 1.7B int8 CUDA, 7.3s of speech          | 2682 ms, RTF 0.367 (`scripts/diag_both.py 1.7b Qwen3.5-2B-Q4_K_M.gguf gpu`) |
| ASR 1.7B int8 CUDA, 10s streamed WS         | 2614 ms compute, RTF 0.259 (`/v1/stream` websocket test, `ws_diag`)         |
| Polish Qwen3.5-2B Q4_K_M, GPU both-resident | 336 ms (`diag_both.py`)                                                     |
| Polish Qwen3.5-2B Q4_K_M, GPU alone         | 540 ms first completion (`diag_flow.py gpu … --reasoning off`)              |
| Polish Qwen3.5-2B Q4_K_M, CPU               | 1060 ms first completion (`diag_flow.py cpu … --reasoning off`)             |
| llama-server cold start → /health           | 5.0–6.1 s CPU, 3.0 s GPU (`diag_flow.py`)                                   |
| Prompt-cache warm (0.8B)                    | 233–857 ms (`reflow.log` "Refinement prompt cache warmed in …")             |
| Headless API transcribe, 10s WAV            | 2624 ms release→final including 445 ms rewrite (WS final metrics)           |

### Note on Class B VRAM headroom

With the desktop holding ~1900 MiB, the budgeter runs ASR 1.7B int8
(~2570 MiB) only when ≥ ~2.1 GB is free; otherwise it correctly steps down
to 0.6B int8 (~1362 MiB measured) rather than dropping to CPU — a measured
6× latency cliff (17546 ms vs 2887 ms for 7.3s of audio) that
`asr_selection_tests.rs` pins. Both models resident (1.7B + 2B polish)
leave 50–64 MiB free at an _idle_ desktop; under a working desktop expect
the resolver to choose 0.6B + 0.8B or partial 2B offload instead.

## 2. Release Gate Verification Summary

Verified on 2026-08-30 on the dev machine unless noted:

- [x] **Rust test suite**: 344 pass / 0 fail with `cargo test -- --test-threads=1`
      (includes live suites `flow_runtime_live.rs`, `asr_status_live.rs`, which
      skip cleanly when weights are absent).
- [x] **Python ASR runtime selftest**: 60 checks pass
      (`python model-runtime/qwen3_asr_runtime.py --selftest`), including
      fp32-precision acceptance and spill classification.
- [x] **Startup model selection**: measured live — resolves _after_ the
      sidecar's CUDA probe answers (new `probe_cuda` command); previously it
      raced the probe and loaded 0.6B on CPU on CUDA-capable machines
      (regression tests in `asr/sidecar.rs`).
- [x] **Orphaned runtime cleanup**: a force-killed Reflow used to leave
      `llama-server` children holding VRAM; startup now sweeps orphans
      (`rewrite::server::kill_orphaned_llama_servers`).
- [x] **Headless LAN API**: `reflow --api --bind 127.0.0.1:7840` loads the ASR
      model, serves `/v1/health`, pairing, `/v1/transcribe` (verified with
      correct transcript) and the `/v1/stream` websocket (verified end to end
      with a synthetic client). Pairing codes rotate every 4 minutes in
      headless mode and are re-printed.
- [x] **Hands-free dictation**: double-tap Shift+Win holds past the 600s-class
      duration guard (verified live at the 60s configured guard: log line
      "…but this is a hands-free session; continuing."), plain hold is still
      capped, single tap stops cleanly (161.9s session stopped on demand).
- [x] **Deep tier (Qwen3.5-2B)**: installs from `unsloth/Qwen3.5-2B-GGUF`
      (1221.5 MB), launches on CPU (5.0 s) and GPU (3.0 s), polishes correctly
      with `--reasoning off` (without it the model spends its whole budget in
      a reasoning block and returns an empty string — reconfirmed on the 2B:
      `''` with `finish_reason=length`).
- [x] **Overlay window stability**: measured live — one size (600×76) for the
      entire active pipeline (listening/transcribing/inserting), grows to
      600×92 only for the settled result, hides ~1.2 s after feedback.
- [x] **Rust compilation**: `cargo check --lib` clean (warnings none).
- [ ] **Frontend TypeScript / Vitest / build**: run before every release
      (`npx tsc --noEmit`, `npm test -- --run`, `npm run build`) — passing on
      2026-08-29; re-run for the release commit.
- [ ] **Full dictation chain with paste into a real focused window** (hotkey →
      capture → ASR → polish → paste): the injection code paths are live
      (focus-confirmed, two-batch SendInput, deferred clipboard restore) and
      the 2026-08-29 history DB shows real pasted rows with
      `rewriter_used=1`, but the 2026-08-30 pass did not repeat a
      human-at-keyboard dictation. **Requires one manual dictation per
      release.**
- [ ] **HUD visual criteria** (live waveform response, 4-vs-3 stage rail,
      per-stage colors, developer-mode ms readouts): the window-sizing and
      state-machine behaviour is verified (above); the rendered appearance is
      covered by component tests only. **Requires one manual look per
      release.**

## 3. Sign-Off

- **Product**: Reflow (Local-First Adaptive Inference Dictation)
- **Target Platform**: Windows 11 & Linux (x86_64) — _Linux paths are
  unit-tested only; no Linux machine was part of this verification pass._
- **Engine Backends**: Python ASR sidecar + `llama-server` GGUF refinement
- **Last full verification**: 2026-08-30 (Class B machine only)
- **Status**: **Class B verified. Classes A, C, D, E UNVERIFIED — do not
  ship claims about them.** Release readiness for other hardware classes
  requires running `scripts/verify_system.py` and the live test suites on
  each class before their rows can carry a PASS.
