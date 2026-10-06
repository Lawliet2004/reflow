# Phonon-2 in Reflow

Select **Settings → Performance → Phonon-2 · Fast English**. Selecting the card downloads missing weights or reloads installed weights. Phonon uses the Python runtime; it cannot use Reflow's experimental native Qwen runtime.

The download is 163,515,201 bytes (about 164 MB). This is the compressed weight archive, not total disk or RAM use. Extraction retains the archive and creates an approximately 178 MB container. Python dependencies and CUDA's expanded weights consume additional space and memory.

## Setup and supported execution

Install Reflow's normal [pinned requirements](../model-runtime/requirements.txt). For an existing working Qwen installation, add the Phonon dependencies to the same interpreter:

```bash
python -m pip install -r model-runtime/requirements-phonon.txt
```

- **CPU:** Fermion's packed int8 encoder and TDT decoder. The wheel chooses its available CPU kernel tier; this machine used AVX2. Older CPUs may use slower kernels or the package's reference fallback.
- **NVIDIA CUDA:** local weights expand to fp32 in the Transformers Parakeet graph. Reflow uses a greedy TDT decoder adapted from Fermion's reference loop, separating token and duration logits. A compatible CUDA PyTorch installation and sufficient free VRAM are required. Automatic selection falls back to CPU when CUDA is unavailable or the estimated expanded weights do not fit.
- **Language:** English. Auto-detect accepts English and reports `en`; it does not add multilingual recognition. Explicit requests for another language fail before recording begins. Choose Qwen for other languages.

Reflow retains cancellation, silence-aligned segmentation, imports, subtitle exports and dictionary replacements. Phonon does not receive recognition hotword bias. Audio is buffered and decoded when recording ends, as with Reflow's other speech models. Faster-than-real-time processing does not imply live partial transcripts.

The desktop CUDA adapter is distinct from Fermion's Docker CUDA runtime. AMD/Intel GPU, Vulkan and Apple MLX execution are not integrated for this model. The live checks below cover Windows; Linux execution is supported by the pinned runtime but has not been exercised in this verification.

## Verified behavior

On 2026-10-04, the pinned archive was downloaded through Reflow's network-policy guard and its SHA-256 was verified. A local synthetic 6.7736-second WAV spoke:

> The quick brown fox jumps over the lazy dog. Please add this speech model to the application.

Both devices returned every reference word correctly, with no truncation warnings. Hardware: AMD Ryzen 5 7535HS, NVIDIA RTX 2050 (4 GB), Python 3.11.9, PyTorch 2.13.0+cu130 and Transformers 5.15.1.

| Device      | Actual format | Load after imports | Transcription | Audio / processing time |
| ----------- | ------------- | ------------------ | ------------- | ----------------------- |
| CPU, AVX2   | packed int8   | 12.17 s            | 0.532 s       | 12.7×                   |
| NVIDIA CUDA | fp32          | 31.76 s            | 0.754 s       | 9.0×                    |

These are one short synthetic sample, not an accuracy corpus or an older-laptop guarantee. Timing includes preprocessing and segmentation, excludes startup and model loading, and does not include optional writing-model cleanup. CPU was faster for this sample.

The separate Rust sidecar test also passed on both devices: 0.542 s CPU and 0.928 s CUDA, with the same exact reference words. It additionally verified PCM transfer, device/precision reporting and rejection of Hindi requests.

On 2026-10-05, the same-session unload/reload test exposed a closed-pipe bug in model switching. Loads, downloads and CUDA probes now recreate a retired Python process. The UI shows a spinner only while the backend reports an active load, keeping Reload available for an installed but unloaded model. The repaired CPU → unload → CUDA test passed with exact reference transcripts on both devices.

Reproduce with your own English WAV and expected reference:

```bash
python scripts/verify_phonon.py --audio speech.wav --device both --reference "The exact spoken words" --output results.json
```

Add `--install` to download missing weights. The script respects Reflow's offline policy and network journal. CPU-only systems can use `--device cpu`. The opt-in `phonon_live` Rust integration test exercises the actual sidecar IPC and PCM transfer; set `REFLOW_PHONON_AUDIO` and `REFLOW_PHONON_REFERENCE`, then run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test phonon_live -- --ignored --nocapture
```

## Provenance and attribution

Weights: [Fermion Research Phonon-2](https://huggingface.co/FermionResearch/Phonon-2/tree/ca1bef26bcd8ef4a7e16d0636d8a77bb25e298ee), revision `ca1bef26bcd8ef4a7e16d0636d8a77bb25e298ee`. The archive SHA-256 is `98125795b6dda72f5c6eee9ba33d19815df65dcb18b50a357bf9f73c9935309e`; the reconstructed container SHA-256 is `4b6bfa3a12cc3c4e0a54f2ab3ec4ca7a842b09e5c7ecfc8e7ca0ac6cc8c11468`. Both are checked before loading. Extraction also checks each member's size and checksum and rejects unsafe paths.

Phonon-2, copyright 2026 Fermion Research, derives from [NVIDIA Parakeet TDT 0.6B v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), copyright NVIDIA Corporation. Fermion retrained encoder linear weights to a five-value alphabet and quantized remaining parameters to 6-bit tables. Model weights remain licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Reflow changes runtime integration and execution, not the weights. The downloader retains the upstream `NOTICE` and weight/code license files beside the archive.

Runtime: [Fermion's official source](https://github.com/fermionresearch/phonon), installed as `fermion-research==0.2.7`. The CUDA decoder and extraction adapter use portions of its Apache-2.0 implementation; the [Apache license](../model-runtime/licenses/Fermion-Apache-2.0.txt) is included in source and bundled resources. This adapter relies on private Fermion interfaces, so upgrades require updating the pin and rerunning CPU/CUDA verification. No remote Python code executes and inference makes no Hub calls.
