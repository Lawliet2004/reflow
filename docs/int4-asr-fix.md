# 4-bit ASR loading fix — 1 October 2026

The installed Qwen3-ASR 1.7B checkpoint failed during INT4 conversion and fell
back to full-precision CPU. The failure was reproduced with the installed
PyTorch 2.13.0+cu130, Transformers 5.15.1, torchao 0.18.0 and RTX 2050 (4 GB).

## Causes and changes

- `Int4WeightOnlyConfig()` defaults to PLAIN packing. In the installed torchao
  0.18 implementation, that path raises `ImportError: Requires mslk >= 1.0.0`.
  Select version 2 `tile_packed_to_4d` explicitly to use PyTorch's CUDA tinygemm
  kernels. A real linear-layer probe confirmed execution on this GPU.
- Moving the completed model to CUDA was too late: Transformers converted
  weights on CPU. The tiled packing operator has no CPU implementation in this
  Windows build. Pass `device_map={"": "cuda"}` for INT4 so weights are placed
  on CUDA before conversion. This also avoids loading the whole BF16 model on
  the GPU first. Add the tested `accelerate==1.14.0` dependency required by
  Transformers for `device_map`; it is already installed on this machine.
- The loader retried every conversion or memory error with eager attention.
  Retry only a ValueError identifying unsupported SDPA attention; other errors
  now reach the normal load-failure handling immediately.
- Warmup swallowed every inference failure and allowed the loader to report
  ready. Reject a model when all warmup shapes fail. A successful shape still
  permits loading if another shape fails.

These choices follow the [torchao INT4 configuration API](https://docs.pytorch.org/ao/stable/api_reference/generated/torchao.quantization.Int4WeightOnlyConfig.html)
and [Transformers torchao integration](https://huggingface.co/docs/transformers/main/quantization/torchao),
and were checked against the locally installed library source. The tiled CUDA
path requires compute capability 8.0 or newer; this RTX 2050 supports it.

## Verification

`python -m unittest discover -s model-runtime -p test_runtime.py -v` passes all
10 tests, including six added regressions. The packing, device placement,
unrelated-error retry, and entirely failed warmup cases failed before their
respective fixes. `python model-runtime/qwen3_asr_runtime.py --selftest` passes
all 70 checks.

The real sidecar was exercised using `load_model` → status polling →
`start_stream` → chunked `push_audio_b64` → `stop_stream`, with CUDA and INT4
required in the resulting status. Installed 1.7B weights were used without
downloads or settings changes.

| Measurement                                          | Result                                          |
| ---------------------------------------------------- | ----------------------------------------------- |
| Reported backend                                     | Qwen3-ASR 1.7B · CUDA int4                      |
| Model load including warmup, after imports           | 9.23 seconds                                    |
| Cold process through ready status, including imports | 26.19 seconds                                   |
| PyTorch reserved GPU memory                          | 1,822 MiB                                       |
| Spill check                                          | Passed; no detected spill                       |
| Test audio                                           | 10.55-second existing English corpus clip       |
| Three final transcription durations                  | 0.890, 0.891, 0.891 seconds                     |
| Accuracy on this clip                                | Exact reference words in all three runs (WER 0) |

This verifies the installed model on this GPU and English clip. It does not
establish accuracy on other languages or recordings. Raw reproduction,
kernel-probe and IPC evidence is retained locally under
`.codex-runtime-evidence/repro-int4.log`, `probe-int4.log`, `int4-live.log` and
`int4-live-result.json` (ignored by Git).

The existing debug bundle's `_up_/model-runtime/qwen3_asr_runtime.py` resource
was refreshed from the fixed source and its SHA-256 verified. Future Tauri
builds bundle this source through the existing resource configuration. A
separately installed release requires rebuilding/reinstalling to receive the
updated resource; no release installer was produced in this task.

Global `pip check` reports existing issues in unrelated `coderag` and `opspilot`
packages. All six pinned ASR dependencies match the installed versions, and
the real ASR test passes.

## Both model sizes verified

The follow-up also exercised the installed 0.6B checkpoint with CUDA/INT4
required in ready status. All three runs matched the same English reference
exactly, with no reported spill. PyTorch reserved 990 MiB; final transcription
took 0.922, 0.672 and 0.656 seconds for the 10.55-second clip. Model loading
including warmup took 10.15 seconds after imports. Evidence is in
`.codex-runtime-evidence/int4-06-live-result.json` and `int4-06-live.log`.

The 0.6B model was still blocked by the Rust manifest and settings page. INT4
is now listed as supported for both Python model sizes; the settings button is
enabled for 0.6B, and switching between model sizes preserves the user's INT4
choice. Automatic precision selection retains its existing BF16/INT8 ladder.

The backend selection regression failed with `asr_precision_unsupported` before
the manifest fix, and both UI regressions failed before removing the restriction
and reset. After the fixes, all 84 frontend tests and 53 backend selection,
profile and runtime-policy tests pass, along with the frontend production build
and lint checks for the changed components. The real browser preview confirms
4-bit is enabled with 0.6B selected and reports no console errors. Preview does
not perform ASR; actual inference was verified separately through real IPC.
