#!/usr/bin/env python3
"""
Reflow Local Qwen3-ASR Streaming Runtime Sidecar.

Real offline inference with HF-native Qwen3-ASR weights
(Qwen/Qwen3-ASR-1.7B-hf or Qwen/Qwen3-ASR-0.6B-hf) on CUDA when available.

JSON-lines IPC over stdin/stdout:
  {"cmd": "ping"}
  {"cmd": "status"}
  {"cmd": "load_model", "model_dir", "device"}
  {"cmd": "install_model", "model_dir"}
  {"cmd": "start_stream", "language", "vocabulary"}
  {"cmd": "push_audio_b64", "audio_b64"}
  {"cmd": "stop_stream", "audio_b64"?}          -> blocks for final transcript
  {"cmd": "cancel_stream"}
  {"cmd": "unload_model"}
"""

import sys
import json
import base64
import os
import threading
import time

import numpy as np

SAMPLE_RATE = 16000
# Hold-to-talk buffers audio while the hotkey is down and transcribes once
# on release — no live partials (those steal the GPU mid-utterance).
LIVE_PARTIALS = False
PARTIAL_MIN_NEW_AUDIO_S = 1.2
PARTIAL_TAIL_S = 20.0
FINAL_MAX_AUDIO_S = 120.0
MIN_VOICED_S = 0.18
MIN_PAD_S = 0.5
PEAK_TARGET = 0.85
SILENCE_RMS = 0.004

def expected_bytes_for(model_dir: str) -> int:
    return 4_076_000_000 if "1.7" in model_dir else 1_565_000_000


MAX_VOCAB_TERMS = 60

LANG_NAMES = {
    "en": "English",
    "hi": "Hindi",
    "zh": "Chinese",
    "yue": "Cantonese",
    "ar": "Arabic",
    "de": "German",
    "fr": "French",
    "es": "Spanish",
    "pt": "Portuguese",
    "id": "Indonesian",
    "it": "Italian",
    "ko": "Korean",
    "ru": "Russian",
    "th": "Thai",
    "vi": "Vietnamese",
    "ja": "Japanese",
    "tr": "Turkish",
    "ms": "Malay",
    "nl": "Dutch",
    "sv": "Swedish",
    "da": "Danish",
    "fi": "Finnish",
    "pl": "Polish",
    "cs": "Czech",
    "fil": "Filipino",
    "fa": "Persian",
    "el": "Greek",
    "hu": "Hungarian",
    "mk": "Macedonian",
    "ro": "Romanian",
}


def preprocess_waveform(samples: np.ndarray) -> tuple[np.ndarray, dict]:
    """Qwen3-ASR-ready mono float32 at 16 kHz.

    Official inference only range-clips; dictation mics are much quieter than
    the file-based eval set, so we also:
      * remove DC
      * drop leading/trailing silence (keep 150 ms pad)
      * peak-normalize quiet speech toward PEAK_TARGET
      * pad to the model's 0.5 s minimum
    """
    stats = {
        "duration_s": 0.0,
        "voiced_s": 0.0,
        "peak": 0.0,
        "rms": 0.0,
        "voiced": False,
        "boost": 1.0,
    }
    if samples is None or samples.size == 0:
        return np.zeros(0, dtype=np.float32), stats

    wav = np.ascontiguousarray(samples, dtype=np.float32).reshape(-1)
    wav = np.nan_to_num(wav, nan=0.0, posinf=0.0, neginf=0.0)
    stats["duration_s"] = float(wav.size) / SAMPLE_RATE

    wav = wav - float(np.mean(wav))
    peak = float(np.max(np.abs(wav))) if wav.size else 0.0
    rms = float(np.sqrt(np.mean(wav * wav))) if wav.size else 0.0
    stats["peak"] = peak
    stats["rms"] = rms

    if peak < 1e-5 or rms < SILENCE_RMS * 0.25:
        return np.zeros(0, dtype=np.float32), stats

    # Frame-level energy to trim silence without clipping the first phoneme.
    frame = 512
    hop = 256
    if wav.size >= frame:
        n = 1 + (wav.size - frame) // hop
        energy = np.empty(n, dtype=np.float32)
        for i in range(n):
            sl = wav[i * hop : i * hop + frame]
            energy[i] = np.sqrt(np.mean(sl * sl))
        peak_e = float(energy.max())
        thresh = max(0.006, peak_e * 0.12)
        voiced_idx = np.where(energy > thresh)[0]
        if voiced_idx.size == 0:
            return np.zeros(0, dtype=np.float32), stats
        pad = int(0.15 * SAMPLE_RATE)
        start = max(0, int(voiced_idx[0]) * hop - pad)
        end = min(wav.size, int(voiced_idx[-1]) * hop + frame + pad)
        wav = wav[start:end]
        stats["voiced_s"] = float(wav.size) / SAMPLE_RATE
    else:
        stats["voiced_s"] = stats["duration_s"]

    if stats["voiced_s"] < MIN_VOICED_S and wav.size < int(MIN_VOICED_S * SAMPLE_RATE):
        return np.zeros(0, dtype=np.float32), stats

    peak = float(np.max(np.abs(wav))) if wav.size else 0.0
    if peak > 1e-6:
        if peak > 1.0:
            wav = wav / peak
            stats["boost"] = 1.0 / peak
        elif peak < 0.35:
            gain = PEAK_TARGET / peak
            wav = wav * gain
            stats["boost"] = gain
        peak = float(np.max(np.abs(wav)))
    wav = np.clip(wav, -1.0, 1.0)

    min_len = int(MIN_PAD_S * SAMPLE_RATE)
    if wav.size < min_len:
        wav = np.pad(wav, (0, min_len - wav.size))

    stats["peak"] = peak
    stats["rms"] = float(np.sqrt(np.mean(wav * wav))) if wav.size else 0.0
    stats["voiced"] = True
    stats["voiced_s"] = float(wav.size) / SAMPLE_RATE
    return wav.astype(np.float32, copy=False), stats


def pcm16_to_float32(pcm16: bytes) -> np.ndarray:
    if not pcm16:
        return np.zeros(0, dtype=np.float32)
    return np.frombuffer(pcm16, dtype=np.int16).astype(np.float32) / 32768.0


def _log_file_path():
    base = os.environ.get("APPDATA") or os.path.expanduser("~/.local/share")
    if os.environ.get("APPDATA"):
        path = os.path.join(base, "reflow", "logs")
    else:
        path = os.path.join(os.path.expanduser("~"), ".local", "share", "reflow", "logs")
    try:
        os.makedirs(path, exist_ok=True)
        return os.path.join(path, "qwen_asr.log")
    except OSError:
        return None


_LOG_PATH = _log_file_path()


def log_err(msg: str):
    line = f"[{time.strftime('%H:%M:%S')}] [Qwen3-ASR Runtime] {msg}"
    sys.stderr.write(line + "\n")
    sys.stderr.flush()
    if _LOG_PATH:
        try:
            with open(_LOG_PATH, "a", encoding="utf-8") as f:
                f.write(line + "\n")
        except OSError:
            pass


def dir_size_bytes(path: str) -> int:
    total = 0
    for root, _dirs, files in os.walk(path):
        for f in files:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total


class RuntimeState:
    """All mutable state, guarded by the GIL + explicit locks where needed."""

    def __init__(self):
        self.loaded = False
        self.load_started_at = None
        self.load_error = None
        self.device = "none"
        self.backend = "not loaded"
        self.model_dir = ""
        self.vram_mb = 0.0

        # install (download) state
        self.is_downloading = False
        self.download_dir = ""
        self.download_error = None
        # Precision to apply when the install finishes and the auto-load kicks in.
        self.pending_precision = "auto"

        # stream state
        self.stream_audio = bytearray()   # full session PCM16
        self.unprocessed = 0              # bytes appended since last partial run
        self.partial_text = ""
        self.detected_language = ""
        self.vocabulary_prompt = None
        self.stream_language = None


STATE = RuntimeState()

# Transcription requests for the single GPU worker: one job at a time,
# newer jobs replace older pending ones (latest audio wins).
_inf_lock = threading.Condition()
_inf_job = None          # dict(audio=bytes, language=str|None, prompt=str|None, final=bool)
_inf_result = None       # dict(text=str, language=str) for the last completed job
_inf_busy = False


class Qwen3AsrModel:
    """Holds the loaded model/processor. Owned by the loader thread."""

    def __init__(self, model, processor, device):
        self.model = model
        self.processor = processor
        self.device = device


_MODEL = None          # type: Qwen3AsrModel | None
_MODEL_LOCK = threading.Lock()


def _torch():
    import torch
    return torch


def pick_device(requested: str) -> str:
    """Map UI backend names onto torch devices."""
    r = (requested or "auto").strip().lower()
    if r in ("cpu",):
        return "cpu"
    want_cuda = r in ("auto", "", "gpu", "cuda")
    force_cuda = r in ("gpu", "cuda")
    try:
        import torch
        if want_cuda and torch.cuda.is_available():
            return "cuda"
    except Exception:
        pass
    if force_cuda:
        raise RuntimeError("CUDA was requested but is not available")
    return "cpu"


def _cuda_runtime_info() -> dict:
    """Return a snapshot of torch's CUDA status for the status payload.

    Used by the Rust sidecar to surface a one-line fix-it hint when the
    user has an NVIDIA card but the installed torch is the CPU-only build.
    The `gpu_hint` is a `pip install` command that pulls the matching
    CUDA-enabled torch wheel for the version of CUDA we detected (or the
    default cu121 if torch's CUDA version is unavailable, e.g. when torch
    is the CPU-only build and we never imported a CUDA torch).
    """
    info = {
        "cuda_available": False,
        "torch_cuda_version": None,
        "gpu_hint": None,
    }
    try:
        import torch  # noqa: F401
    except Exception:
        return info

    try:
        import torch
        info["cuda_available"] = bool(torch.cuda.is_available())
        version = getattr(torch.version, "cuda", None)
        if version:
            info["torch_cuda_version"] = str(version)
    except Exception:
        return info

    if info["cuda_available"]:
        return info

    # GPU is present (caller already detected nvidia-smi) but torch is the
    # CPU-only build. Pick a wheel URL that matches the *driver's* CUDA,
    # or fall back to cu121 which covers most modern NVIDIA drivers.
    cu_tag = "cu121"
    if info["torch_cuda_version"]:
        v = info["torch_cuda_version"].split(".")
        if len(v) >= 2 and v[0].isdigit() and v[1].isdigit():
            cu_tag = f"cu{v[0]}{v[1]}"
    info["gpu_hint"] = (
        "pip install --upgrade torch torchao "
        f"--index-url https://download.pytorch.org/whl/{cu_tag}"
    )
    return info


def weights_bytes(model_dir: str) -> int:
    total = 0
    try:
        for name in os.listdir(model_dir):
            if name.endswith(".safetensors") or name.endswith(".bin"):
                total += os.path.getsize(os.path.join(model_dir, name))
    except OSError:
        pass
    return total


def _try_load(model_dir: str, device: str, precision: str):
    """Load the model once with a concrete strategy. Raises on failure.

    `precision` is one of: "auto" (no override), "int4", "int8", "bf16".
    `int4`/`int8` apply torchao weight-only quantization on CUDA; "bf16" forces
    full BF16 on CUDA / FP32 on CPU.
    """
    import torch
    from transformers import AutoProcessor, AutoModelForMultimodalLM

    quant_kind = None
    if device == "cuda":
        if precision == "int4":
            quant_kind = "int4"
            dtype = torch.bfloat16
        elif precision == "int8":
            quant_kind = "int8"
            dtype = torch.bfloat16
        else:
            # "bf16" or "auto" with no quantization -> full BF16 weights
            dtype = torch.bfloat16
    else:
        # CPU is always full precision. Quantized weights on CPU just cost RAM
        # without a speedup.
        dtype = torch.float32

    kwargs = {"torch_dtype": dtype, "low_cpu_mem_usage": True}
    if quant_kind == "int8" and device == "cuda":
        from transformers import TorchAoConfig
        from torchao.quantization import Int8WeightOnlyConfig

        kwargs["quantization_config"] = TorchAoConfig(Int8WeightOnlyConfig())
    elif quant_kind == "int4" and device == "cuda":
        try:
            from transformers import TorchAoConfig
            from torchao.quantization import Int4WeightOnlyConfig

            kwargs["quantization_config"] = TorchAoConfig(Int4WeightOnlyConfig())
        except Exception as e:
            raise RuntimeError(
                f"Int4 weight-only quantization is not available in this torchao "
                f"install ({e}). Install torchao with int4 support, or pick a "
                f"different precision."
            )

    if device == "cuda":
        torch.backends.cuda.matmul.allow_tf32 = True
        torch.backends.cudnn.allow_tf32 = True
        try:
            torch.set_float32_matmul_precision("high")
        except Exception:
            pass

    with _MODEL_LOCK:
        processor = AutoProcessor.from_pretrained(model_dir)
        # Older Reflow installs omitted chat_template.jinja from the model
        # snapshot. Use Qwen's upstream multimodal template verbatim so both
        # string content and apply_transcription_request's list-form audio/text
        # content render correctly. A plain text-only Qwen template fails here
        # with `can only concatenate str (not "list") to str`.
        if not getattr(processor, "chat_template", None):
            processor.chat_template = """{%- set ns = namespace(system_text='') -%}{%- for m in messages -%}{%- if m.role == 'system' -%}{%- if m.content is string -%}{%- set ns.system_text = ns.system_text + m.content -%}{%- else -%}{%- for c in m.content -%}{%- if c.type == 'text' and (c.text is defined) -%}{%- set ns.system_text = ns.system_text + c.text -%}{%- endif -%}{%- endfor -%}{%- endif -%}{%- endif -%}{%- endfor -%}{%- set ns2 = namespace(audio_tokens='') -%}{%- for m in messages -%}{%- if m.content is not string -%}{%- for c in m.content -%}{%- if c.type == 'audio' or ('audio' in c) or ('audio_url' in c) -%}{%- set ns2.audio_tokens = ns2.audio_tokens + '<|audio_start|><|audio_pad|><|audio_end|>' -%}{%- endif -%}{%- endfor -%}{%- endif -%}{%- endfor -%}{{- '<|im_start|>system\n' + ns.system_text + '<|im_end|>\n' -}}{{- '<|im_start|>user\n' + ns2.audio_tokens + '<|im_end|>\n' -}}{%- for m in messages -%}{%- if m.role == 'assistant' -%}{%- set ns3 = namespace(assistant_text='') -%}{%- if m.content is string -%}{%- set ns3.assistant_text = m.content -%}{%- else -%}{%- for c in m.content -%}{%- if c.type == 'text' and (c.text is defined) -%}{%- set ns3.assistant_text = ns3.assistant_text + c.text -%}{%- endif -%}{%- endfor -%}{%- endif -%}{{- '<|im_start|>assistant\n' -}}{% generation %}{{- ns3.assistant_text + '<|im_end|>\n' -}}{% endgeneration %}{%- endif -%}{%- endfor -%}{%- if add_generation_prompt -%}{{- '<|im_start|>assistant\n' -}}{%- endif -%}"""
            if hasattr(processor, "tokenizer") and processor.tokenizer is not None:
                processor.tokenizer.chat_template = processor.chat_template
        model = AutoModelForMultimodalLM.from_pretrained(model_dir, **kwargs)
        if device != "cpu" or quant_kind is not None:
            # quantized weights may already be placed; .to is a no-op then
            model = model.to(device)
        model.eval()
        global _MODEL
        _MODEL = Qwen3AsrModel(model, processor, device)
        _warmup_model(_MODEL)
    return model


def _warmup_model(wrapper: "Qwen3AsrModel"):
    """Compile CUDA kernels so the first real utterance is not 5–10s."""
    try:
        silence = np.zeros(int(0.5 * SAMPLE_RATE), dtype=np.float32)
        req = {
            "audio": silence,
            "processor_kwargs": {
                "audio_kwargs": {"sampling_rate": SAMPLE_RATE, "padding": "longest"}
            },
        }
        inputs = wrapper.processor.apply_transcription_request(**req)
        torch = _torch()
        device = next(iter(wrapper.model.parameters())).device
        dtype = next(iter(wrapper.model.parameters())).dtype
        inputs = inputs.to(device, dtype)
        with torch.inference_mode():
            wrapper.model.generate(**inputs, max_new_tokens=8, do_sample=False)
        if device.type == "cuda":
            torch.cuda.synchronize()
        log_err("Inference warmup done")
    except Exception as e:
        log_err(f"Warmup skipped: {e}")


def build_attempts(
    precision: str,
    device: str,
    vram_bytes: int,
    weights_bytes_count: int,
) -> list:
    """Pure function: given a (precision, device, VRAM, weight_size) tuple,
    return the list of (mode_label, target_device, quant_kind) attempts the
    load loop will try in order. The first attempt that doesn't raise wins.

    Pulled out of `load_model_blocking` so it can be unit-tested without
    touching torch or the filesystem.
    """
    # Normalize precision so callers passing "AUTO"/"Int8" still match.
    precision = (precision or "auto").strip().lower()
    if precision not in ("auto", "int4", "int8", "bf16"):
        precision = "auto"

    if device != "cuda":
        return [("cpu", "cpu", None)]

    vram = vram_bytes
    w = weights_bytes_count
    attempts: list = []

    if precision == "auto":
        # INT8 halves the weights — accuracy-preserving and the only
        # way the 1.7B model fits on a 4 GB card (e.g. RTX 2050).
        if w * 0.55 + 700 * 1024 * 1024 <= vram:
            attempts.append(("int8", "cuda", "int8"))
        if w * 1.25 <= vram:
            attempts.append(("bf16", "cuda", None))
    elif precision == "int4":
        if w * 0.30 + 700 * 1024 * 1024 <= vram:
            attempts.append(("int4", "cuda", "int4"))
    elif precision == "int8":
        if w * 0.55 + 700 * 1024 * 1024 <= vram:
            attempts.append(("int8", "cuda", "int8"))
    elif precision == "bf16":
        if w * 1.25 <= vram:
            attempts.append(("bf16", "cuda", None))
    # Last-resort: drop to CPU so the user can still dictate.
    attempts.append(("cpu", "cpu", None))
    return attempts


def load_model_blocking(
    model_dir: str,
    requested_device: str = "auto",
    precision: str = "auto",
):
    global _MODEL
    t0 = time.time()
    STATE.loaded = False
    device = pick_device(requested_device)
    # Normalize precision so callers passing "AUTO"/"Int8" still match.
    precision = (precision or "auto").strip().lower()
    if precision not in ("auto", "int4", "int8", "bf16"):
        log_err(f"Unknown precision '{precision}', falling back to auto")
        precision = "auto"

    try:
        if not os.path.isfile(os.path.join(model_dir, "config.json")):
            raise FileNotFoundError(
                f"Model weights not found at {model_dir}. "
                "Download them first (Model settings → Install)."
            )

        if device == "cpu":
            try:
                import torch
                torch.set_num_threads(max(1, os.cpu_count() or 4))
            except Exception:
                pass

        # Build the attempts table using the pure function so the mapping
        # is testable without torch.
        if device == "cuda":
            import torch
            vram = torch.cuda.get_device_properties(0).total_memory
        else:
            vram = 0
        attempts = build_attempts(precision, device, vram, weights_bytes(model_dir))

        last_err = None
        for label, target, quant in attempts:
            try:
                log_err(
                    f"Loading Qwen3-ASR from {model_dir} on {target} "
                    f"({label})"
                )
                _unload_model_blocking()
                _try_load(model_dir, target, quant if quant is not None else "auto")
                STATE.device = target
                STATE.backend = (
                    f"Qwen3-ASR {_model_label(model_dir)} · "
                    f"{target.upper()} {label}"
                )
                STATE.vram_mb = 0.0
                if target == "cuda":
                    try:
                        import torch
                        STATE.vram_mb = torch.cuda.memory_reserved() / (1024 * 1024)
                    except Exception:
                        pass
                STATE.loaded = True
                STATE.load_error = None
                STATE.model_dir = model_dir
                log_err(
                    f"Model ready on {target} ({label}) in {time.time() - t0:.1f}s"
                )
                return {
                    "status": "ok",
                    "loaded": True,
                    "device": target,
                    "backend": STATE.backend,
                }
            except Exception as e:
                last_err = e
                log_err(f"{target} {label} load failed: {e}")

        raise RuntimeError(str(last_err) if last_err else "all load strategies failed")
    except Exception as e:
        STATE.loaded = False
        STATE.load_error = str(e)
        STATE.backend = "load failed"
        STATE.device = "none"
        log_err(f"Model load failed: {e}")
        return {"status": "error", "error": str(e)}


def looks_like_vocab_echo(text: str, prompt) -> bool:
    """True when the transcript is just the vocabulary prompt echoed back
    (Qwen3-ASR does this on silent/unclear audio)."""
    if not prompt or not text:
        return False
    import re

    try:
        terms_raw = prompt.split(":", 1)[1].rstrip(".")
    except IndexError:
        return False
    term_words = {w.lower() for t in terms_raw.split(",") for w in t.split()}
    low = re.sub(r"[^\w\s]", "", text.lower()).strip()
    words = low.split()
    return bool(words) and all(w in term_words for w in words)


def _model_label(model_dir: str) -> str:
    return "1.7B" if "1.7" in model_dir else "0.6B"


def _unload_model_blocking():
    global _MODEL
    with _MODEL_LOCK:
        if _MODEL is not None:
            del _MODEL
            _MODEL = None
    try:
        import gc
        gc.collect()
        import torch
        torch.cuda.empty_cache()
    except Exception:
        pass


def _parse_transcript(processor, generated, raw_fallback: str = ""):
    """Prefer parsed {language, transcription}; never return the prompt echo."""
    text, lang = "", ""
    try:
        parsed = processor.decode(generated, return_format="parsed")
        first = parsed[0] if parsed else {}
        if isinstance(first, dict):
            text = (first.get("transcription") or first.get("text") or "").strip()
            lang = (first.get("language") or "").strip()
            if isinstance(lang, str) and lang.lower() in ("none", "null"):
                lang = ""
        else:
            text = str(first).strip()
    except Exception:
        text = raw_fallback
        try:
            text = processor.decode(generated, return_format="transcription_only")[0]
        except Exception:
            text = processor.batch_decode(generated, skip_special_tokens=True)[0]
        text = (text or "").replace("<asr_text>", "").replace("</asr_text>", "").strip()
        if text.lower().startswith("language "):
            # "language English<rest>" leftover from a raw decode
            parts = text.split("<asr_text>", 1)
            text = parts[-1].strip() if parts else text
    return (text or "").strip(), (lang or "").strip()


def transcribe_blocking(pcm16: bytes, language_name, prompt):
    """Run real ASR on PCM16 mono 16kHz audio. Returns (text, language)."""
    model = _MODEL
    if model is None:
        raise RuntimeError("Model is not loaded")

    raw = pcm16_to_float32(pcm16)
    samples, stats = preprocess_waveform(raw)
    log_err(
        f"audio {stats['duration_s']:.2f}s voiced={stats['voiced_s']:.2f}s "
        f"peak={stats['peak']:.3f} rms={stats['rms']:.4f} boost={stats['boost']:.2f}x"
    )
    if not stats["voiced"] or samples.size == 0:
        log_err("No voiced audio after preprocess; skipping inference")
        return "", ""

    req = {
        "audio": samples,
        "processor_kwargs": {
            "audio_kwargs": {
                "sampling_rate": SAMPLE_RATE,
                # Default is pad-to-30s. That makes a 2s utterance as expensive
                # as a 30s one. Longest/no extra pad keeps encoder work proportional
                # to what was actually spoken.
                "padding": "longest",
                "max_length": None,
            }
        },
    }
    if language_name:
        req["language"] = language_name
    if prompt and stats["rms"] >= 0.02:
        req["prompt"] = prompt

    inputs = model.processor.apply_transcription_request(**req)

    torch = _torch()
    target_device = next(iter(model.model.parameters())).device
    target_dtype = next(iter(model.model.parameters())).dtype
    inputs = inputs.to(target_device, target_dtype)

    input_len = inputs.get("input_ids").shape[1] if "input_ids" in inputs else 0
    # Dictation replies are short. generation_config.json defaults to 512
    # new tokens — int8 decode of 512 tokens is several seconds even when
    # the transcript is a dozen words.
    max_new = min(128, max(24, int(stats["voiced_s"] * 16) + 32))

    t_gen = time.time()
    with torch.inference_mode():
        output_ids = model.model.generate(
            **inputs,
            max_new_tokens=max_new,
            do_sample=False,
            use_cache=True,
        )
    gen_s = time.time() - t_gen
    new_tokens = int(output_ids.shape[1] - input_len)
    log_err(
        f"generate {new_tokens} tokens in {gen_s:.2f}s on {target_device} "
        f"(cap {max_new}, audio {stats['voiced_s']:.2f}s)"
    )

    generated = output_ids[:, input_len:]
    text, lang = _parse_transcript(model.processor, generated)
    if looks_like_vocab_echo(text, prompt) or text.strip().lower().startswith("vocabulary:"):
        log_err(f"Model echoed the vocabulary prompt ({text!r}); treating as no speech")
        return "", lang
    return text, lang


def _inference_worker():
    """Single consumer that performs partial transcriptions as audio arrives."""
    global _inf_job, _inf_result, _inf_busy
    while True:
        with _inf_lock:
            while _inf_job is None:
                _inf_lock.wait()
            job = _inf_job
            _inf_job = None
            _inf_busy = True

        try:
            if job.get("final"):
                # finals are executed on the caller's thread; skip here
                continue
            text, lang = transcribe_blocking(
                job["audio"], job.get("language"), job.get("prompt")
            )
            if text:
                with _inf_lock:
                    _inf_result = {"text": text, "language": lang}
                    STATE.partial_text = text
                    if lang:
                        STATE.detected_language = lang
                    STATE.unprocessed = 0
        except Exception as e:
            log_err(f"partial transcription failed: {e}")
        finally:
            with _inf_lock:
                _inf_busy = False
                _inf_lock.notify_all()


def _maybe_kick_partial(language_name):
    """Start a partial transcription if enough new audio arrived and worker is idle."""
    global _inf_job, _inf_busy
    with _inf_lock:
        if _inf_busy or _MODEL is None:
            return
        new_audio_s = STATE.unprocessed / 2 / SAMPLE_RATE
        if new_audio_s < PARTIAL_MIN_NEW_AUDIO_S:
            return
        total_s = len(STATE.stream_audio) / 2 / SAMPLE_RATE
        tail_bytes = int(min(total_s, PARTIAL_TAIL_S) * 2 * SAMPLE_RATE)
        audio = bytes(STATE.stream_audio[-tail_bytes:])
        _inf_job = {
            "audio": audio,
            "language": language_name,
            "prompt": STATE.vocabulary_prompt,
            "final": False,
        }
        _inf_busy = True
        _inf_lock.notify_all()


def _wait_partial_idle(timeout_s: float) -> bool:
    end = time.time() + timeout_s
    with _inf_lock:
        while _inf_busy:
            remaining = end - time.time()
            if remaining <= 0:
                return False
            _inf_lock.wait(remaining)
    return True


def _wait_model_ready(timeout_s: float = 240.0) -> bool:
    """Block until a load in progress finishes (dictation during startup)."""
    if STATE.loaded:
        return True
    end = time.time() + timeout_s
    while time.time() < end:
        if STATE.loaded:
            return True
        if STATE.load_error and not STATE.loaded:
            # keep waiting a little in case a retry started
            if not _load_thread_alive():
                return False
        time.sleep(0.15)
    return False


def _load_thread_alive() -> bool:
    return any(t.name == "model-loader" and t.is_alive() for t in threading.enumerate())


def start_load(model_dir: str, device: str, precision: str = "auto"):
    if any(t.name == "model-loader" and t.is_alive() for t in threading.enumerate()):
        return {"status": "already-loading"}
    STATE.load_started_at = time.time()
    STATE.load_error = None
    # Must flip loaded off immediately. Leaving it True from a previous
    # load makes the UI think the model is ready and stop polling, while
    # the subtitle still says "loading…".
    STATE.loaded = False
    STATE.device = "none"
    STATE.backend = f"loading {_model_label(model_dir)}…"
    t = threading.Thread(
        target=lambda: load_model_blocking(model_dir, device, precision),
        name="model-loader",
        daemon=True,
    )
    t.start()
    return {"status": "loading"}


def start_install(model_dir: str, repo: str):
    def run():
        try:
            STATE.is_downloading = True
            STATE.download_error = None
            os.makedirs(model_dir, exist_ok=True)
            log_err(f"Downloading {repo} weights to {model_dir}")
            from huggingface_hub import snapshot_download

            snapshot_download(
                repo,
                local_dir=model_dir,
                max_workers=4,
                allow_patterns=[
                    "config.json",
                    "generation_config.json",
                    "tokenizer*",
                    "preprocessor*",
                    "processor*",
                    "*.jinja",
                    "*.safetensors",
                    "*.json",
                ],
            )
            STATE.is_downloading = False
            log_err("Model download complete")
            # auto-load right after install, honoring the precision the user
            # asked for when they kicked off the install.
            start_load(model_dir, "auto", STATE.pending_precision)
        except Exception as e:
            STATE.is_downloading = False
            STATE.download_error = str(e)
            log_err(f"Model download failed: {e}")

    if any(t.name == "model-installer" and t.is_alive() for t in threading.enumerate()):
        return {"status": "already-downloading"}
    t = threading.Thread(target=run, name="model-installer", daemon=True)
    t.start()
    return {"status": "downloading"}


def handle(msg: dict) -> dict:
    global _inf_result
    cmd = msg.get("cmd")

    if cmd == "ping":
        return {"status": "ok", "pong": True}

    if cmd == "status":
        loading = _load_thread_alive()
        cuda_info = _cuda_runtime_info()
        resp = {
            "status": "ok",
            "loaded": bool(STATE.loaded) and not loading,
            "device": STATE.device,
            "backend": STATE.backend,
            "model_dir": STATE.model_dir,
            "vram_mb": round(STATE.vram_mb, 1),
            "is_downloading": STATE.is_downloading,
            "is_loading": loading,
            "error": STATE.load_error or STATE.download_error,
            "cuda_available": cuda_info["cuda_available"],
            "torch_cuda_version": cuda_info["torch_cuda_version"],
            "gpu_hint": cuda_info["gpu_hint"],
        }
        if STATE.is_downloading:
            done = dir_size_bytes(STATE.download_dir) if STATE.download_dir else 0
            expected = expected_bytes_for(STATE.download_dir or "")
            resp["download_progress_pct"] = min(100, int(done * 100 / expected))
        return resp

    if cmd == "load_model":
        model_dir = msg.get("model_dir", "")
        device = msg.get("device", "auto")
        precision = msg.get("precision", "auto")
        if not os.path.isdir(model_dir) or not os.path.isfile(
            os.path.join(model_dir, "config.json")
        ):
            return {
                "status": "error",
                "error": "Model not installed. Open Settings → Model to download it.",
            }
        return start_load(model_dir, device, precision)

    if cmd == "install_model":
        model_dir = msg.get("model_dir", "")
        repo = msg.get("repo", "Qwen/Qwen3-ASR-0.6B-hf")
        precision = msg.get("precision", "auto")
        STATE.download_dir = model_dir
        # Stash the requested precision so the auto-load that follows a
        # successful install honors the user's choice.
        STATE.pending_precision = precision
        if os.path.isfile(os.path.join(model_dir, "config.json")):
            return {"status": "ok", "detail": "already installed"}
        return start_install(model_dir, repo)

    if cmd == "unload_model":
        _unload_model_blocking()
        STATE.loaded = False
        STATE.device = "none"
        STATE.backend = "not loaded"
        return {"status": "ok"}

    if cmd == "start_stream":
        STATE.stream_audio = bytearray()
        STATE.unprocessed = 0
        STATE.partial_text = ""
        STATE.detected_language = ""
        vocab = msg.get("vocabulary") or []
        if vocab:
            terms = ", ".join(str(t) for t in vocab[:MAX_VOCAB_TERMS])
            STATE.vocabulary_prompt = f"Vocabulary: {terms}."
        else:
            STATE.vocabulary_prompt = None
        lang = msg.get("language", "auto")
        STATE.stream_language = LANG_NAMES.get(lang) if lang and lang != "auto" else None
        return {"status": "ok", "streaming": True}

    if cmd == "push_audio_b64":
        audio = base64.b64decode(msg.get("audio_b64", ""))
        STATE.stream_audio.extend(audio)
        STATE.unprocessed += len(audio)
        if LIVE_PARTIALS:
            _maybe_kick_partial(getattr(STATE, "stream_language", None))
        return {"status": "ok", "text": None}

    if cmd == "stop_stream":
        extra = msg.get("audio_b64")
        if extra:
            try:
                STATE.stream_audio.extend(base64.b64decode(extra))
            except Exception as e:
                log_err(f"stop_stream audio_b64 decode failed: {e}")
        if not STATE.stream_audio:
            return {"status": "ok", "text": "", "language": ""}
        if LIVE_PARTIALS:
            _wait_partial_idle(2.0)
        if not _wait_model_ready():
            return {
                "status": "error",
                "error": STATE.load_error or "Model not ready",
                "text": "",
            }
        audio = bytes(STATE.stream_audio[: int(FINAL_MAX_AUDIO_S * 2 * SAMPLE_RATE)])
        try:
            text, lang = transcribe_blocking(
                audio, getattr(STATE, "stream_language", None), STATE.vocabulary_prompt
            )
            if lang:
                STATE.detected_language = lang
            log_err(f"final transcript ({len(text)} chars, lang={lang or '-'}): {text[:180]!r}")
            return {"status": "ok", "text": text, "language": lang}
        except Exception as e:
            log_err(f"final transcription failed: {e}")
            return {"status": "error", "error": str(e), "text": ""}
        finally:
            STATE.stream_audio = bytearray()
            STATE.unprocessed = 0
            STATE.partial_text = ""

    if cmd == "cancel_stream":
        STATE.stream_audio = bytearray()
        STATE.unprocessed = 0
        STATE.partial_text = ""
        return {"status": "ok"}

    return {"status": "error", "error": f"Unknown command: {cmd}"}


def _selftest() -> bool:
    """Audio-preprocess checks that do not load the model."""
    ok = True

    def check(name, cond):
        nonlocal ok
        if not cond:
            print(f"FAIL {name}", file=sys.stderr)
            ok = False
        else:
            print(f"ok   {name}")

    silence = np.zeros(16000, dtype=np.float32)
    out, stats = preprocess_waveform(silence)
    check("silence-dropped", (not stats["voiced"]) and out.size == 0)

    t = np.arange(int(1.2 * SAMPLE_RATE), dtype=np.float32) / SAMPLE_RATE
    quiet = (0.02 * np.sin(2 * np.pi * 220 * t)).astype(np.float32)
    out, stats = preprocess_waveform(quiet)
    check("quiet-speech-kept", stats["voiced"] and out.size > 0)
    check("quiet-speech-boosted", stats["boost"] > 1.5 and stats["peak"] > 0.5)

    loud = np.concatenate(
        [np.zeros(8000, dtype=np.float32), 0.6 * np.sin(2 * np.pi * 330 * t[:8000]), np.zeros(8000, dtype=np.float32)]
    )
    out, stats = preprocess_waveform(loud)
    check("silence-trimmed", stats["voiced"] and out.size < loud.size)

    pcm = (quiet * 32767.0).astype(np.int16).tobytes()
    back = pcm16_to_float32(pcm)
    check("pcm-roundtrip", back.size == quiet.size and abs(float(np.max(np.abs(back))) - 0.02) < 0.002)

    check("vocab-echo", looks_like_vocab_echo("Qwen Tauri", "Vocabulary: Qwen, Tauri, Supabase."))
    check("vocab-real-speech", not looks_like_vocab_echo("Ship the Tauri build today", "Vocabulary: Qwen, Tauri, Supabase."))

    fresh_state = RuntimeState()
    check(
        "stream-language-default",
        hasattr(fresh_state, "stream_language") and fresh_state.stream_language is None,
    )

    # Exercise the optional stop-stream tail without loading model weights.
    # The stubbed transcriber reports the byte count, proving the accumulated
    # stream and trailing payload are both passed to final transcription.
    saved_stream_state = (
        STATE.stream_audio,
        STATE.unprocessed,
        STATE.partial_text,
        STATE.loaded,
        STATE.load_error,
        STATE.detected_language,
        STATE.stream_language,
        STATE.vocabulary_prompt,
    )
    saved_transcriber = transcribe_blocking
    saved_live_partials = LIVE_PARTIALS
    try:
        STATE.stream_audio = bytearray(b"prefix")
        STATE.unprocessed = 0
        STATE.partial_text = ""
        STATE.loaded = True
        STATE.load_error = None
        STATE.detected_language = ""
        STATE.stream_language = None
        STATE.vocabulary_prompt = None
        globals()["transcribe_blocking"] = lambda audio, _language, _prompt: (
            str(len(audio)),
            "en",
        )
        globals()["LIVE_PARTIALS"] = False
        response = handle(
            {
                "cmd": "stop_stream",
                "audio_b64": base64.b64encode(b"tail").decode("ascii"),
            }
        )
        check("stop-stream-appends-audio", response.get("text") == "10")
    finally:
        globals()["transcribe_blocking"] = saved_transcriber
        globals()["LIVE_PARTIALS"] = saved_live_partials
        (
            STATE.stream_audio,
            STATE.unprocessed,
            STATE.partial_text,
            STATE.loaded,
            STATE.load_error,
            STATE.detected_language,
            STATE.stream_language,
            STATE.vocabulary_prompt,
        ) = saved_stream_state

    # build_attempts — pure function that decides the load ladder for each
    # precision. We don't need torch or weights on disk to verify the
    # mapping; VRAM and weight size are injected.
    W17 = 4_076_000_000   # 1.7B weights on disk
    W06 = 1_565_000_000   # 0.6B weights on disk
    GB4 = 4 * 1024 * 1024 * 1024
    GB6 = 6 * 1024 * 1024 * 1024
    GB24 = 24 * 1024 * 1024 * 1024

    # CPU is always CPU.
    check("attempts-cpu", build_attempts("auto", "cpu", 0, W17) == [("cpu", "cpu", None)])

    # 1.7B on a 4 GB card, auto: int8 fits, bf16 doesn't -> int8 + cpu fallback.
    a = build_attempts("auto", "cuda", GB4, W17)
    check("attempts-1.7b-4gb-auto", a == [("int8", "cuda", "int8"), ("cpu", "cpu", None)])

    # 1.7B on a 6 GB card, auto: both int8 and bf16 fit, int8 first.
    a = build_attempts("auto", "cuda", GB6, W17)
    check("attempts-1.7b-6gb-auto", a == [("int8", "cuda", "int8"), ("bf16", "cuda", None), ("cpu", "cpu", None)])

    # User pinned int4: only int4 + cpu fallback (no silent bf16 substitution).
    a = build_attempts("int4", "cuda", GB4, W17)
    check("attempts-1.7b-4gb-int4", a == [("int4", "cuda", "int4"), ("cpu", "cpu", None)])

    # User pinned int4 on a too-small card: cpu only.
    a = build_attempts("int4", "cuda", 1024 * 1024 * 1024, W17)
    check("attempts-int4-too-small", a == [("cpu", "cpu", None)])

    # User pinned int8 on 4 GB: int8 + cpu fallback.
    a = build_attempts("int8", "cuda", GB4, W17)
    check("attempts-1.7b-4gb-int8", a == [("int8", "cuda", "int8"), ("cpu", "cpu", None)])

    # User pinned bf16 on 4 GB: int8 won't be tried, bf16 doesn't fit, cpu only.
    a = build_attempts("bf16", "cuda", GB4, W17)
    check("attempts-1.7b-4gb-bf16", a == [("cpu", "cpu", None)])

    # User pinned bf16 on 6 GB: just bf16 + cpu fallback.
    a = build_attempts("bf16", "cuda", GB6, W17)
    check("attempts-1.7b-6gb-bf16", a == [("bf16", "cuda", None), ("cpu", "cpu", None)])

    # 0.6B auto on a 4 GB card: bf16 fits, int8 also fits, int8 first.
    a = build_attempts("auto", "cuda", GB4, W06)
    check("attempts-0.6b-4gb-auto", a == [("int8", "cuda", "int8"), ("bf16", "cuda", None), ("cpu", "cpu", None)])

    # Garbage / case-dirty precision falls back to auto.
    a = build_attempts("POTATO", "cuda", GB6, W17)
    check("attempts-garbage-precision-falls-back-to-auto", a == build_attempts("auto", "cuda", GB6, W17))
    a = build_attempts("INT8", "cuda", GB6, W17)
    check("attempts-uppercase-normalized", a == build_attempts("int8", "cuda", GB6, W17))

    return ok


def main():
    log_err(f"Qwen3-ASR sidecar started (pid {os.getpid()}). Listening on stdin...")
    threading.Thread(target=_inference_worker, name="asr-worker", daemon=True).start()
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            msg = json.loads(line)
            resp = handle(msg)
        except Exception as e:
            resp = {"status": "error", "error": str(e)}
        try:
            sys.stdout.write(json.dumps(resp) + "\n")
            sys.stdout.flush()
        except Exception:
            break
    log_err("stdin closed; sidecar exiting")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] in ("--selftest", "selftest"):
        sys.exit(0 if _selftest() else 1)
    main()
