# Zipformer 20M INT8 in Reflow

Select **Settings → Performance → Zipformer 20M INT8**. Selecting the card downloads missing weights or reloads installed weights. Zipformer uses the Python sidecar with the sherpa-onnx runtime; it cannot use Reflow's experimental native Qwen runtime.

The download is 43,649,301 bytes (about 44 MB): the three pinned int8 ONNX files (`encoder`, `decoder`, `joiner`) plus `tokens.txt` from `csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17` at revision `d42f2d9f7ca24806fb667456a18a9f1b60f70d16`. The fp32 siblings in the same repository are not downloaded. Each file is SHA-256 verified at install and again at load.

## Setup and supported execution

Install Reflow's normal [pinned requirements](../model-runtime/requirements.txt). For an existing working installation, add only the Zipformer dependency to the same interpreter:

```bash
python -m pip install -r model-runtime/requirements-zipformer.txt
```

- **Device:** CPU only. sherpa-onnx runs the int8 encoder with two inference threads.
- **Decoding:** modified beam search (4 active paths) with sherpa-onnx endpoint detection enabled.
- **Language:** English. Choosing another dictation language fails before recording begins. Choose a Qwen model for other languages.

## Streaming behavior

Unlike Reflow's buffered engines, Zipformer decodes as audio arrives. Each pushed chunk feeds the sherpa-onnx stream directly; endpoint-finalized utterances are committed and the stream state is reset, so feature memory stays bounded over long dictations and only the decoder tail is drained at stop. The final transcript is the concatenation of committed segments plus the in-flight segment.

Live partial text is reported through the usual `push_audio` return path: the Rust side issues a `get_partial` probe after each frame batch, and the sidecar answers it on the reader thread. Dictionary replacements still apply to the final text; recognition hotword bias is not wired for this model.

Tests that do not download weights:

```bash
python model-runtime/test_zipformer.py
```

An end-to-end check with real audio uses the shared driver — point it at the installed model directory:

```bash
REFLOW_MODEL_DIR=<app models dir>/zipformer-20m python scripts/test_sidecar.py speech.wav
```
