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
import queue
import struct
import subprocess
import threading
import time

import numpy as np

_CANCEL_EVENT = threading.Event()


class CancelStoppingCriteria:
    """StoppingCriteria that aborts model.generate() as soon as _CANCEL_EVENT is set."""

    def __init__(self, event: threading.Event):
        self.event = event

    def __call__(self, input_ids=None, scores=None, **kwargs) -> bool:
        return self.event.is_set()


def _read_exact(stream, n: int) -> bytes:
    """Read exactly n bytes from a binary stream or raise EOFError."""
    buf = bytearray()
    while len(buf) < n:
        chunk = stream.read(n - len(buf))
        if not chunk:
            raise EOFError("Unexpected EOF on binary stream")
        buf.extend(chunk)
    return bytes(buf)


def unpack_binary_audio_frame(header: bytes) -> tuple:
    """Unpack 12-byte binary audio frame header: (session_id: u32, seq_id: u32, data_len: u32)"""
    return struct.unpack("<III", header)


SAMPLE_RATE = 16000
# Hold-to-talk buffers audio while the hotkey is down and transcribes once
# on release — no live partials (those steal the GPU mid-utterance).
LIVE_PARTIALS = False
PARTIAL_MIN_NEW_AUDIO_S = 1.2
PARTIAL_TAIL_S = 20.0
# Ceiling on the audio handed to a single `generate()` call.
#
# One hour. Raised from 120s, which silently discarded everything past two
# minutes — a ten minute dictation came back as its first fifth.
#
# Raising this is only safe because the caller's transcription timeout now scales
# with the audio length (see `transcribe_timeout` in asr/sidecar.rs). Against the
# old fixed 180s budget, anything over ~450s of audio blew the timeout, and a
# `stop_stream` timeout is not a harmless probe: it kills the sidecar and counts a
# crash strike. A bigger cap without a bigger budget would have turned a
# truncated transcript into no transcript at all.
#
# Still bounded on purpose. A single pass over an hour of audio is ~24 minutes of
# compute at the measured 0.4x realtime, and longer on CPU. Long-form dictation
# really wants segmented transcription on silence boundaries; this only stops the
# audio being thrown away, and any overflow is reported rather than hidden.
FINAL_MAX_AUDIO_S = 3600.0
MIN_VOICED_S = 0.18
MIN_PAD_S = 0.5
PEAK_TARGET = 0.85
SILENCE_RMS = 0.004

# Task 16: pad audio up to a fixed set of durations instead of to whatever the
# utterance happened to be. CUDA picks and tunes kernels per input shape, so an
# unbucketed pipeline pays an autotune penalty on the first utterance of every
# new length — which, with millisecond-resolution lengths, is every utterance.
AUDIO_BUCKET_SECONDS = (2.0, 4.0, 8.0, 16.0, 30.0)

# Buckets warmed at load time on CUDA. Covers the range real dictation lands in;
# the 30 s bucket is left to be tuned lazily because warming it would add
# noticeable startup time for an uncommon length.
WARMUP_BUCKET_SECONDS_CUDA = (0.5, 2.0, 4.0, 8.0, 16.0)

# CPU has no kernel autotune, so one small shape is enough to page the weights
# in and JIT any lazily-initialised code.
WARMUP_BUCKET_SECONDS_CPU = (0.5,)

def expected_bytes_for(model_dir: str, model_id=None, expected_bytes=None) -> int:
    if expected_bytes is not None:
        return int(expected_bytes)
    if model_id is not None:
        return 4_076_000_000 if model_id == "1.7b" else 1_565_000_000
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


# Silence-trim analysis window. 512 samples at 16 kHz is 32 ms, hopping 16 ms,
# so roughly 62 frames per second of audio.
FRAME_WIN = 512
FRAME_HOP = 256


def frame_energies(wav: np.ndarray) -> np.ndarray:
    """Per-frame RMS energy, vectorized.

    The original implementation ran a Python `for` over every frame — about 62
    iterations per second of audio, on the full utterance, on every single
    transcription. A strided view plus one reduction does the same arithmetic in
    a single pass with no interpreter overhead.
    """
    if wav.size < FRAME_WIN:
        return np.zeros(0, dtype=np.float32)
    frames = np.lib.stride_tricks.sliding_window_view(wav, FRAME_WIN)[::FRAME_HOP]
    # float64 accumulation keeps the result numerically identical to the
    # per-frame `np.mean` the loop used.
    return np.sqrt(np.mean(frames.astype(np.float64) ** 2, axis=1)).astype(np.float32)


def max_new_tokens_for(voiced_seconds: float) -> int:
    """Decode-token cap for `voiced_seconds` of speech.

    `generation_config.json` defaults to 512, and an INT8 decode of 512 tokens
    takes seconds even when the transcript is a dozen words, so a cap is
    necessary. Sizing it matters in both directions: too high wastes nothing but
    removes the guard, too low silently truncates the transcript.

    English dictation runs about 2.5 words per second, and Qwen's tokenizer
    averages a little over one token per word, so ~4 tokens per second of speech
    is the natural rate. The 3x headroom absorbs fast speakers, dense technical
    vocabulary and the language tag, and the floor covers very short utterances
    where the fixed overhead dominates.
    """
    seconds = max(0.0, float(voiced_seconds))
    estimate = int(seconds * 12.0) + 32
    return max(32, min(384, estimate))


def bucket_for_seconds(seconds: float) -> float | None:
    """Smallest bucket that fits `seconds`, or None if it exceeds them all.

    Returning None means "do not pad": an utterance longer than the largest
    bucket would otherwise be padded to a shape that costs more compute than
    the audio needs.
    """
    for bucket in AUDIO_BUCKET_SECONDS:
        if seconds <= bucket:
            return bucket
    return None


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
    frame = FRAME_WIN
    hop = FRAME_HOP
    if wav.size >= frame:
        energy = frame_energies(wav)
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


# Download progress is read on the `status` fast path, but walking a multi-GB
# model directory is not free. Memoize it briefly so a burst of status polls
# costs one walk rather than one walk each.
_DIR_SIZE_TTL_S = 1.0
_dir_size_cache: dict = {}


def dir_size_bytes_cached(path: str) -> int:
    now = time.time()
    hit = _dir_size_cache.get(path)
    if hit is not None and now - hit[0] < _DIR_SIZE_TTL_S:
        return hit[1]
    size = dir_size_bytes(path)
    _dir_size_cache[path] = (now, size)
    return size


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
        # Precision label the current model actually loaded with ("bf16",
        # "int8", "int4", "cpu"), not the one that was requested.
        self.precision = ""
        # Seconds the last successful load took, for the benchmark cache.
        self.load_seconds = 0.0
        # Real-time factor measured during warmup, or None.
        self.warmup_rtf = None
        # Result of the last VRAM spill check. See `evaluate_spill`.
        self.spill = {"spilled": False, "reasons": [], "checked": False}
        # One of LOAD_FAILURE_KINDS when the last load failed, else None.
        # Distinct from `load_error` so the UI can branch without parsing prose.
        self.load_failure_kind = None
        # True from the moment `load_model` is acknowledged until the loader
        # thread is actually running.
        #
        # `load_model` is acked immediately and the main thread then spends
        # 9-52s in `_warm_imports()` before it can even enqueue the load. Any
        # `status` answered in that window used to report
        # "loaded=False, is_loading=False, backend=not loaded" — indistinguishable
        # from a failed load. The desktop app gates recording on that answer, so
        # the hotkey went dead and the UI claimed the model was not loaded while
        # it was in fact loading normally.
        self.load_pending = False

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


# --------------------------------------------------------------------------
# CUDA capability snapshot.
#
# This is reported on every `status` reply, but it must never be *computed* on
# the `status` path. `import torch` is a 5-10s operation on a cold Windows
# filesystem (measured: 6.4s), and Python's per-module import lock means a
# second `import torch` blocks until the loader thread's import finishes. The
# Rust side budgets 'status' tightly, so computing this inline guaranteed a
# timeout on the very first poll after spawn — which tore the sidecar down
# before the model could load at all.
#
# Instead: probe once on a dedicated background thread and serve every `status`
# reply from an immutable cached dict. The accessor never imports anything.
# --------------------------------------------------------------------------

_CUDA_INFO_PENDING = {
    "cuda_available": False,
    "torch_cuda_version": None,
    "gpu_hint": None,
    # Distinguishes "torch says no CUDA" from "we have not looked yet", so the
    # UI never shows a 'your torch is CPU-only' hint it might have to retract.
    "probe_pending": True,
}

_CUDA_INFO: dict | None = None
_CUDA_INFO_LOCK = threading.Lock()


def _compute_cuda_runtime_info() -> dict:
    """Actually import torch and inspect CUDA. Slow; background thread only.

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
        "probe_pending": False,
    }
    try:
        import torch
    except Exception:
        return info

    try:
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


def _cuda_runtime_info() -> dict:
    """Non-blocking accessor. Returns the cached probe, or a pending stub.

    Guaranteed to do no I/O and no imports, so it is safe on the `status`
    fast path.
    """
    info = _CUDA_INFO
    return info if info is not None else _CUDA_INFO_PENDING


# --------------------------------------------------------------------------
# VRAM spill detection (Task 7)
#
# On Windows WDDM an over-budget CUDA allocation does not raise OOM. The driver
# silently backs the excess with shared system memory and the model keeps
# working — roughly ten times slower. "The load succeeded" is therefore not
# evidence that the configuration is viable, and a benchmark-driven architecture
# that trusts it will validate and then ship its own worst configuration.
#
# Two independent signals are used, because neither is reliable alone:
#
#   1. Memory accounting. Compare what torch says it reserved against how much
#      device memory actually disappeared according to the driver. A large gap
#      means part of torch's reservation is not in VRAM.
#   2. Throughput. Spilled memory is slow. A warmup real-time factor far above
#      the expected baseline indicates spill even when the accounting looks
#      clean, which happens when the driver reports shared memory as device
#      memory.
# --------------------------------------------------------------------------

# Allocator overhead and measurement noise between two nvidia-smi samples.
# Below this a gap is not evidence of anything.
SPILL_TOLERANCE_MB = 192.0

# Measured warmup RTF beyond `expected * this` is treated as spill.
SPILL_RTF_MULTIPLIER = float(os.environ.get("REFLOW_SPILL_RTF_MULTIPLIER", "3.0"))

# Free VRAM below this after a load is itself suspicious: the allocator had to
# be scraping the bottom of the device to fit.
SPILL_FREE_FLOOR_MB = 64.0


def _nvidia_smi(args: list) -> str | None:
    """Run nvidia-smi and return stdout, or None if it is unavailable."""
    try:
        creationflags = 0x08000000 if os.name == "nt" else 0  # CREATE_NO_WINDOW
        out = subprocess.run(
            ["nvidia-smi", *args],
            capture_output=True,
            text=True,
            timeout=5,
            creationflags=creationflags,
        )
    except Exception:
        return None
    if out.returncode != 0:
        return None
    text = (out.stdout or "").strip()
    return text or None


def _device_vram_mb() -> dict | None:
    """Total / used / free VRAM for device 0, as the driver reports it."""
    text = _nvidia_smi(
        [
            "--query-gpu=memory.total,memory.used,memory.free",
            "--format=csv,noheader,nounits",
            "--id=0",
        ]
    )
    if not text:
        return None
    fields = [f.strip() for f in text.splitlines()[0].split(",")]
    if len(fields) < 3:
        return None
    try:
        return {
            "total": float(fields[0]),
            "used": float(fields[1]),
            "free": float(fields[2]),
        }
    except ValueError:
        return None


def _process_vram_mb() -> float | None:
    """Device memory the driver attributes to *this* process.

    Returns None when the driver will not say. WDDM frequently reports
    `[Not Supported]` here, which is exactly why the free-VRAM delta and the
    throughput check both exist.
    """
    text = _nvidia_smi(
        [
            "--query-compute-apps=pid,used_gpu_memory",
            "--format=csv,noheader,nounits",
        ]
    )
    if not text:
        return None
    me = os.getpid()
    for line in text.splitlines():
        fields = [f.strip() for f in line.split(",")]
        if len(fields) < 2:
            continue
        try:
            pid = int(fields[0])
        except ValueError:
            continue
        if pid != me:
            continue
        if not fields[1] or fields[1].startswith("["):
            return None
        try:
            return float(fields[1])
        except ValueError:
            return None
    return None


def evaluate_spill(
    torch_reserved_mb: float,
    free_before_mb: float | None,
    free_after_mb: float | None,
    process_vram_mb: float | None,
    warmup_rtf: float | None = None,
    expected_rtf: float | None = None,
    tolerance_mb: float = SPILL_TOLERANCE_MB,
    rtf_multiplier: float = SPILL_RTF_MULTIPLIER,
) -> dict:
    """Pure spill predicate. No I/O, so it is unit-testable.

    Returns a dict with `spilled`, `reasons` and the inputs, so the UI and the
    benchmark cache can both explain the verdict rather than just asserting it.
    """
    reasons: list = []

    # Signal 1a: the driver attributes far less device memory to us than torch
    # believes it reserved.
    if process_vram_mb is not None and torch_reserved_mb > 0:
        gap = torch_reserved_mb - process_vram_mb
        if gap > tolerance_mb:
            reasons.append(
                f"torch reserved {torch_reserved_mb:.0f} MB but the driver "
                f"attributes only {process_vram_mb:.0f} MB of VRAM to this "
                f"process ({gap:.0f} MB unaccounted)"
            )

    # Signal 1b: device free memory did not drop by as much as torch reserved.
    if free_before_mb is not None and free_after_mb is not None and torch_reserved_mb > 0:
        consumed = free_before_mb - free_after_mb
        gap = torch_reserved_mb - consumed
        if gap > tolerance_mb:
            reasons.append(
                f"torch reserved {torch_reserved_mb:.0f} MB but device free VRAM "
                f"fell by only {consumed:.0f} MB ({gap:.0f} MB unaccounted)"
            )

    # Signal 1c: the device is effectively full. Combined with any reservation
    # this means the next allocation will spill even if this one did not.
    if free_after_mb is not None and free_after_mb < SPILL_FREE_FLOOR_MB:
        reasons.append(
            f"only {free_after_mb:.0f} MB of VRAM remains free after load, "
            f"below the {SPILL_FREE_FLOOR_MB:.0f} MB floor"
        )

    # Signal 2: throughput. This catches the case where the driver reports
    # shared system memory as if it were device memory.
    if warmup_rtf is not None and expected_rtf is not None and expected_rtf > 0:
        limit = expected_rtf * rtf_multiplier
        if warmup_rtf > limit:
            reasons.append(
                f"warmup real-time factor {warmup_rtf:.2f} exceeds "
                f"{limit:.2f} ({rtf_multiplier:.1f}x the {expected_rtf:.2f} "
                f"baseline), which is the signature of memory spilled to "
                f"shared system RAM"
            )

    return {
        "spilled": bool(reasons),
        "reasons": reasons,
        "torch_reserved_mb": round(torch_reserved_mb, 1),
        "process_vram_mb": None if process_vram_mb is None else round(process_vram_mb, 1),
        "free_before_mb": None if free_before_mb is None else round(free_before_mb, 1),
        "free_after_mb": None if free_after_mb is None else round(free_after_mb, 1),
        "warmup_rtf": None if warmup_rtf is None else round(warmup_rtf, 3),
        "expected_rtf": None if expected_rtf is None else round(expected_rtf, 3),
    }


def _measure_spill(
    device: str,
    free_before_mb: float | None,
    warmup_rtf: float | None,
    expected_rtf: float | None,
) -> dict:
    """Collect the live figures and run [`evaluate_spill`]."""
    if device != "cuda":
        return {"spilled": False, "reasons": [], "checked": False}
    torch_reserved_mb = 0.0
    try:
        import torch

        torch_reserved_mb = torch.cuda.memory_reserved() / (1024 * 1024)
    except Exception:
        pass
    after = _device_vram_mb()
    report = evaluate_spill(
        torch_reserved_mb=torch_reserved_mb,
        free_before_mb=free_before_mb,
        free_after_mb=None if after is None else after["free"],
        process_vram_mb=_process_vram_mb(),
        warmup_rtf=warmup_rtf,
        expected_rtf=expected_rtf,
    )
    report["checked"] = True
    return report


# Expected warmup RTF per precision on a modest discrete GPU. Deliberately
# generous: this is a spill tripwire, not a performance target. Replaced by
# measured values once the Task 17 benchmark has run.
EXPECTED_WARMUP_RTF = {
    "bf16": 0.35,
    "int8": 0.50,
    "int4": 0.60,
}


class SpillDetected(RuntimeError):
    """The model loaded but part of it is not in device memory.

    A distinct type so it can be classified separately from OOM: an OOM means
    "this will not fit", spill means "this fits only on paper and will run about
    ten times slower". They need different remediation and, critically, spill
    must invalidate a benchmark rather than being recorded as a success.
    """


# Load-failure taxonomy shared with the Rust side. Each kind implies different
# remediation, so collapsing them into one error string loses the only
# information the user could act on.
LOAD_FAILURE_KINDS = (
    "spill",
    "oom",
    "unsupported_precision",
    "weights_missing",
    "timeout",
    "crash",
    "unknown",
)


def classify_load_failure(exc: BaseException) -> str:
    """Map an exception onto [`LOAD_FAILURE_KINDS`]."""
    if isinstance(exc, SpillDetected):
        return "spill"
    if isinstance(exc, FileNotFoundError):
        return "weights_missing"
    if isinstance(exc, TimeoutError):
        return "timeout"
    text = f"{type(exc).__name__}: {exc}".lower()
    if "out of memory" in text or "cuda_error_out_of_memory" in text:
        return "oom"
    if (
        "quantization" in text
        or "torchao" in text
        or "int4" in text
        or "no kernel image" in text
        or "not available in this torchao" in text
    ):
        return "unsupported_precision"
    if "weights not found" in text or "no such file" in text:
        return "weights_missing"
    if "timed out" in text or "timeout" in text:
        return "timeout"
    if "cuda error" in text or "illegal memory access" in text:
        return "crash"
    return "unknown"


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

    `precision` is one of: "auto" (no override), "int4", "int8", "bf16",
    "fp32" (CPU only; the CPU branch always runs fp32 regardless).
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

    # Task 16: pin the attention implementation rather than relying on whatever
    # Transformers happens to default to for this architecture and version.
    # SDPA has a fused CUDA path and a reasonable CPU path; letting the default
    # silently select "eager" is a large, invisible regression.
    kwargs["attn_implementation"] = "sdpa"

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
        try:
            model = AutoModelForMultimodalLM.from_pretrained(model_dir, **kwargs)
        except Exception as e:
            if kwargs.get("attn_implementation") == "sdpa":
                log_err(f"Loading with sdpa attention failed ({e}); falling back to eager attention")
                kwargs["attn_implementation"] = "eager"
                model = AutoModelForMultimodalLM.from_pretrained(model_dir, **kwargs)
            else:
                raise
        if device != "cpu" or quant_kind is not None:
            # quantized weights may already be placed; .to is a no-op then
            model = model.to(device)
        model.eval()
        global _MODEL
        _MODEL = Qwen3AsrModel(model, processor, device)
        warmup_rtf = _warmup_model(_MODEL)
    return warmup_rtf


def _warmup_model(wrapper: "Qwen3AsrModel") -> float | None:
    """Compile CUDA kernels so the first real utterance is not 5–10s.

    Returns the measured real-time factor of the longest warmup shape, or None
    if warmup could not run. The caller feeds it to the spill check: a warmup
    that is an order of magnitude slower than expected is the clearest evidence
    that memory landed in shared system RAM.
    """
    torch = _torch()
    slowest_rtf = None
    try:
        device = next(iter(wrapper.model.parameters())).device
        dtype = next(iter(wrapper.model.parameters())).dtype
    except Exception as e:
        log_err(f"Warmup skipped: {e}")
        return None

    buckets = (
        WARMUP_BUCKET_SECONDS_CUDA
        if device.type == "cuda"
        else WARMUP_BUCKET_SECONDS_CPU
    )
    for seconds in buckets:
        try:
            silence = np.zeros(int(seconds * SAMPLE_RATE), dtype=np.float32)
            req = {
                "audio": silence,
                "processor_kwargs": {
                    "audio_kwargs": {
                        "sampling_rate": SAMPLE_RATE,
                        # Pad to the bucket so the kernels compiled here are the
                        # ones a real utterance of this length will use.
                        "padding": "max_length",
                        "max_length": int(seconds * SAMPLE_RATE),
                    }
                },
            }
            inputs = wrapper.processor.apply_transcription_request(**req)
            inputs = inputs.to(device, dtype)
            started = time.time()
            with torch.inference_mode():
                wrapper.model.generate(**inputs, max_new_tokens=8, do_sample=False)
            if device.type == "cuda":
                torch.cuda.synchronize()
            elapsed = time.time() - started
            rtf = elapsed / seconds
            log_err(f"Warmup {seconds:g}s bucket: {elapsed:.2f}s (rtf {rtf:.3f})")
            # Report the largest bucket's RTF: it is the least dominated by
            # fixed per-call overhead and so the most comparable to a baseline.
            slowest_rtf = rtf
        except Exception as e:
            log_err(f"Warmup {seconds:g}s bucket skipped: {e}")
    if slowest_rtf is not None:
        log_err("Inference warmup done")
    return slowest_rtf


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
    # "fp32" is accepted for the CPU path: the Rust resolver's CPU selection
    # is Fp32 (the CPU ladder never quantizes), and rejecting it here made
    # every CPU load log "Unknown precision 'fp32'" and fall back to auto.
    precision = (precision or "auto").strip().lower()
    if precision not in ("auto", "int4", "int8", "bf16", "fp32"):
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
    elif precision in ("bf16", "fp32"):
        # fp32 has no CUDA rung of its own: the CUDA path runs BF16
        # full-precision weights (see `_try_load`). Treat an fp32 request
        # like bf16 rather than dropping to CPU, so a settings combination
        # of fp32 + GPU does not silently lose the GPU.
        if w * 1.25 <= vram:
            attempts.append(("bf16", "cuda", None))
    # Last-resort: drop to CPU so the user can still dictate.
    attempts.append(("cpu", "cpu", None))
    return attempts


def load_model_blocking(
    model_dir: str,
    requested_device: str = "auto",
    precision: str = "auto",
    model_id=None,
):
    global _MODEL
    t0 = time.time()
    STATE.loaded = False
    device = pick_device(requested_device)
    # Normalize precision so callers passing "AUTO"/"Int8" still match.
    # "fp32" is accepted: it is what the Rust resolver's CPU selection sends
    # (the CPU ladder never quantizes), and rejecting it logged
    # "Unknown precision 'fp32'" on every CPU load before silently
    # switching to auto.
    precision = (precision or "auto").strip().lower()
    if precision not in ("auto", "int4", "int8", "bf16", "fp32"):
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
        #
        # Budget against *measured free* VRAM, not the device's nominal total.
        # Deciding that a model fits because the card is nominally 4 GB, while a
        # browser is holding 2 GB of it, is precisely how a load ends up spilled
        # into shared system memory.
        vram = 0
        if device == "cuda":
            live = _device_vram_mb()
            if live is not None:
                vram = int(live["free"] * 1024 * 1024)
                log_err(
                    f"Budgeting against {live['free']:.0f} MB free VRAM "
                    f"(device total {live['total']:.0f} MB)"
                )
            else:
                # No nvidia-smi. Fall back to the nominal total but say so, so a
                # subsequent spill verdict is interpretable.
                import torch

                vram = torch.cuda.get_device_properties(0).total_memory
                log_err(
                    "nvidia-smi unavailable; budgeting against nominal total VRAM, "
                    "which may over-admit"
                )
        attempts = build_attempts(precision, device, vram, weights_bytes(model_dir))

        last_err = None
        for label, target, quant in attempts:
            try:
                log_err(f"Loading Qwen3-ASR from {model_dir} on {target} ({label})")
                _unload_model_blocking()
                # Sample free VRAM *before* the load so the spill check has a
                # baseline. Reading it after and subtracting an estimate is
                # exactly the mistake this whole task exists to avoid.
                before = _device_vram_mb() if target == "cuda" else None
                free_before = None if before is None else before["free"]

                warmup_rtf = _try_load(
                    model_dir, target, quant if quant is not None else "auto"
                )

                # A CUDA load that "succeeded" is not yet known to be viable.
                spill = _measure_spill(
                    device=target,
                    free_before_mb=free_before,
                    warmup_rtf=warmup_rtf,
                    expected_rtf=EXPECTED_WARMUP_RTF.get(label),
                )
                if spill.get("spilled"):
                    detail = "; ".join(spill.get("reasons", [])) or "unspecified"
                    STATE.spill = spill
                    # Refuse the configuration rather than shipping something
                    # that runs an order of magnitude slower in silence.
                    raise SpillDetected(
                        f"{target} {label} spilled VRAM into shared system "
                        f"memory: {detail}"
                    )

                STATE.spill = spill
                STATE.device = target
                STATE.precision = label
                STATE.backend = (
                    f"Qwen3-ASR {_model_label(model_dir, model_id)} · {target.upper()} {label}"
                )
                STATE.vram_mb = 0.0
                STATE.warmup_rtf = warmup_rtf
                if target == "cuda":
                    try:
                        import torch

                        STATE.vram_mb = torch.cuda.memory_reserved() / (1024 * 1024)
                    except Exception:
                        pass
                STATE.loaded = True
                STATE.load_error = None
                STATE.load_failure_kind = None
                STATE.model_dir = model_dir
                STATE.load_seconds = time.time() - t0
                log_err(f"Model ready on {target} ({label}) in {STATE.load_seconds:.1f}s")
                return {
                    "status": "ok",
                    "loaded": True,
                    "device": target,
                    "backend": STATE.backend,
                }
            except Exception as e:
                last_err = e
                kind = classify_load_failure(e)
                log_err(f"{target} {label} load failed [{kind}]: {e}")
                # Free whatever the failed attempt left behind, or the next
                # attempt inherits its fragmentation and spills too.
                _unload_model_blocking()

        raise RuntimeError(str(last_err) if last_err else "all load strategies failed")
    except Exception as e:
        STATE.loaded = False
        STATE.load_error = str(e)
        STATE.load_failure_kind = classify_load_failure(e)
        STATE.backend = "load failed"
        STATE.device = "none"
        log_err(f"Model load failed [{STATE.load_failure_kind}]: {e}")
        return {
            "status": "error",
            "error": str(e),
            "failure_kind": STATE.load_failure_kind,
            "spill": STATE.spill,
        }


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


def _model_label(model_dir: str, model_id=None) -> str:
    if model_id is not None:
        return model_id.upper()
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

    # Task 16: pad to a fixed bucket rather than to the exact length.
    #
    # `padding: "longest"` keeps encoder work proportional to what was spoken,
    # which is why it replaced the 30 s default — but it also means every
    # utterance presents CUDA with a shape it has never seen, so the first
    # inference at each new length pays a kernel-autotune penalty. Since audio
    # lengths are effectively continuous, that is every utterance. Bucketing
    # trades a little padding for a warm kernel cache: the buckets are all
    # pre-compiled at load time.
    bucket_s = bucket_for_seconds(stats["voiced_s"])
    if bucket_s is None:
        # Longer than the largest bucket. Padding up would cost more compute
        # than the audio itself needs, so fall back to exact-length padding and
        # accept the autotune cost for an uncommon case.
        audio_kwargs = {
            "sampling_rate": SAMPLE_RATE,
            "padding": "longest",
            "max_length": None,
        }
    else:
        audio_kwargs = {
            "sampling_rate": SAMPLE_RATE,
            "padding": "max_length",
            "max_length": int(bucket_s * SAMPLE_RATE),
        }

    req = {
        "audio": samples,
        "processor_kwargs": {"audio_kwargs": audio_kwargs},
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
    max_new = max_new_tokens_for(stats["voiced_s"])

    _CANCEL_EVENT.clear()
    stopping_criteria = [CancelStoppingCriteria(_CANCEL_EVENT)]
    try:
        from transformers.generation.stopping_criteria import StoppingCriteriaList

        stopping_criteria = StoppingCriteriaList(stopping_criteria)
    except Exception:
        pass

    t_gen = time.time()
    with torch.inference_mode():
        output_ids = model.model.generate(
            **inputs,
            max_new_tokens=max_new,
            do_sample=False,
            use_cache=True,
            stopping_criteria=stopping_criteria,
        )
    gen_s = time.time() - t_gen

    if _CANCEL_EVENT.is_set():
        log_err("Inference cancelled mid-generation; discarding result")
        return "", ""

    new_tokens = int(output_ids.shape[1] - input_len)
    log_err(
        f"generate {new_tokens} tokens in {gen_s:.2f}s on {target_device} "
        f"(cap {max_new}, audio {stats['voiced_s']:.2f}s, "
        f"bucket {bucket_s if bucket_s else 'exact'}, rtf "
        f"{gen_s / max(stats['voiced_s'], 1e-6):.3f})"
    )
    if new_tokens >= max_new:
        # The cap, not an end-of-sequence token, ended generation. The
        # transcript is very likely truncated mid-sentence.
        log_err(
            f"WARNING: generation hit the {max_new}-token cap for "
            f"{stats['voiced_s']:.1f}s of audio; the transcript may be cut off"
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


# Loading native extensions (scipy/BLAS in particular) from a worker thread
# while the main thread is blocked in a CRT stdio call can deadlock the
# process: the worker wedges inside DllMain-time module init contending a
# CRT critical section owned by the main thread, and neither thread ever
# runs again. Observed on Windows with torch's Intel OpenMP resident and
# numpy/scipy's OpenBLAS builds both present. Doing every DLL-bearing
# import of the load path here, once, on the main thread (while main is
# demonstrably not blocked in the reader) makes the model-loader thread's
# later imports pure sys.modules cache hits, so no module init ever runs on
# a worker during a load.
_warm_imports_done = False
_warm_imports_lock = threading.Lock()


def _warm_imports():
    """Pre-import the load path's native extensions on the main thread."""
    global _warm_imports_done, _CUDA_INFO
    if _warm_imports_done:
        return
    with _warm_imports_lock:
        if _warm_imports_done:
            return
        started = time.time()
        log_err("Warming imports on the main thread...")
        import torch  # noqa: F401

        from transformers import (  # noqa: F401
            AutoModelForMultimodalLM,
            AutoProcessor,
            TorchAoConfig,
        )
        try:
            from torchao.quantization import (  # noqa: F401
                Int4WeightOnlyConfig,
                Int8WeightOnlyConfig,
            )
        except Exception:
            pass
        _warm_imports_done = True
        log_err(f"Warm imports complete in {time.time() - started:.1f}s")
        # The CUDA capability snapshot used to be computed on its own
        # background thread, but that put a torch import on a worker thread -
        # the same native-init-under-contention hazard this warmup exists to
        # remove. Compute it here instead; `status` serves it from the cache
        # and reports probe_pending until this runs.
        try:
            computed = _compute_cuda_runtime_info()
        except Exception as e:  # pragma: no cover - defensive
            log_err(f"CUDA probe failed: {e}")
            computed = dict(_CUDA_INFO_PENDING, probe_pending=False)
        with _CUDA_INFO_LOCK:
            _CUDA_INFO = computed
        log_err(
            "CUDA probe: available={} torch_cuda={}".format(
                computed.get("cuda_available"), computed.get("torch_cuda_version")
            )
        )


def start_load(model_dir: str, device: str, precision: str = "auto", model_id=None, expected_bytes=None):
    if any(t.name == "model-loader" and t.is_alive() for t in threading.enumerate()):
        STATE.load_pending = False
        return {"status": "already-loading"}
    STATE.load_started_at = time.time()
    STATE.load_error = None
    # Must flip loaded off immediately. Leaving it True from a previous
    # load makes the UI think the model is ready and stop polling, while
    # the subtitle still says "loading…".
    STATE.loaded = False
    STATE.device = "none"
    STATE.backend = f"loading {_model_label(model_dir, model_id)}…"
    t = threading.Thread(
        target=lambda: load_model_blocking(model_dir, device, precision, model_id),
        name="model-loader",
        daemon=True,
    )
    t.start()
    # The thread is alive from here on, so `_load_thread_alive()` takes over
    # from the pending flag with no gap in which `status` could report idle.
    STATE.load_pending = False
    return {"status": "loading"}


def start_install(model_dir: str, repo: str, model_id=None, expected_bytes=None):
    STATE.download_model_id = model_id
    STATE.download_expected_bytes = expected_bytes
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
            start_load(model_dir, "auto", STATE.pending_precision, model_id, expected_bytes)
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
        # A load that has been acked but whose worker has not started yet is
        # still a load in progress. Reporting otherwise makes the caller treat
        # a warming sidecar as a broken one.
        loading = _load_thread_alive() or bool(STATE.load_pending)
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
            # True while the warm imports (and therefore the CUDA probe) are
            # still running: `cuda_available=false` from a pending probe means
            # "not looked yet", which a load decision must wait on rather
            # than act on. Absent on older readers, which see `false` and keep
            # their pre-existing behaviour.
            "cuda_probe_pending": bool(cuda_info.get("probe_pending")),
            # Task 7: spill is a distinct outcome from OOM and from success.
            "spill_detected": bool(STATE.spill.get("spilled")),
            "spill_reasons": list(STATE.spill.get("reasons", [])),
            "spill_checked": bool(STATE.spill.get("checked")),
            "failure_kind": STATE.load_failure_kind,
            # Measured figures for the benchmark cache and the readiness panel.
            "precision": STATE.precision,
            "load_seconds": round(STATE.load_seconds, 2),
            "warmup_rtf": STATE.warmup_rtf,
        }
        if STATE.is_downloading:
            done = dir_size_bytes_cached(STATE.download_dir) if STATE.download_dir else 0
            expected = expected_bytes_for(STATE.download_dir or "", getattr(STATE, "download_model_id", None), getattr(STATE, "download_expected_bytes", None))
            resp["download_progress_pct"] = min(100, int(done * 100 / expected))
        return resp

    if cmd == "load_model":
        model_dir = msg.get("model_dir", "")
        device = msg.get("device", "auto")
        precision = msg.get("precision", "auto")
        if not os.path.isdir(model_dir) or not os.path.isfile(
            os.path.join(model_dir, "config.json")
        ):
            STATE.load_pending = False
            return {
                "status": "error",
                "error": "Model not installed. Open Settings → Model to download it.",
            }
        return start_load(model_dir, device, precision, msg.get("model_id"), msg.get("expected_bytes"))

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
        return start_install(model_dir, repo, msg.get("model_id"), msg.get("expected_bytes"))

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
        # Cap the audio handed to a single generate() call, but never silently.
        # Dropping the end of what someone said and returning a confident-looking
        # transcript is worse than telling them it was cut.
        limit_bytes = int(FINAL_MAX_AUDIO_S * 2 * SAMPLE_RATE)
        captured_s = len(STATE.stream_audio) / 2 / SAMPLE_RATE
        truncated_s = 0.0
        if len(STATE.stream_audio) > limit_bytes:
            truncated_s = captured_s - FINAL_MAX_AUDIO_S
            log_err(
                f"WARNING: dictation was {captured_s:.1f}s, longer than the "
                f"{FINAL_MAX_AUDIO_S:.0f}s single-pass limit; the last "
                f"{truncated_s:.1f}s will not be transcribed"
            )
        audio = bytes(STATE.stream_audio[:limit_bytes])
        try:
            text, lang = transcribe_blocking(
                audio, getattr(STATE, "stream_language", None), STATE.vocabulary_prompt
            )
            if lang:
                STATE.detected_language = lang
            log_err(f"final transcript ({len(text)} chars, lang={lang or '-'}): {text[:180]!r}")
            response = {"status": "ok", "text": text, "language": lang}
            if truncated_s > 0.0:
                # Surfaced to the user by the Rust side rather than buried in a
                # log nobody reads.
                response["warning"] = (
                    f"Only the first {FINAL_MAX_AUDIO_S:.0f}s of this "
                    f"{captured_s:.0f}s dictation was transcribed."
                )
                response["truncated_seconds"] = round(truncated_s, 1)
                response["captured_seconds"] = round(captured_s, 1)
            return response
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

    cancel_evt = threading.Event()
    criteria = CancelStoppingCriteria(cancel_evt)
    check("cancel-criteria-init", not criteria())
    cancel_evt.set()
    check("cancel-criteria-tripped", criteria())

    frame_header = struct.pack("<III", 42, 1, 6)
    sess_id, seq_id, d_len = unpack_binary_audio_frame(frame_header)
    check("binary-frame-unpack", sess_id == 42 and seq_id == 1 and d_len == 6)

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

    # "fp32" is what the Rust resolver's CPU selection sends; it must be
    # accepted (mapped to the plain CPU rung) rather than reset to auto.
    a = build_attempts("fp32", "cpu", 0, W17)
    check("attempts-fp32-cpu-accepted", a == [("cpu", "cpu", None)])
    # On CUDA, fp32 resolves to the BF16 rung (the CUDA path's full-
    # precision option), like bf16.
    a = build_attempts("fp32", "cuda", GB6, W17)
    check(
        "attempts-fp32-cuda-matches-bf16",
        a == build_attempts("bf16", "cuda", GB6, W17),
    )

    # ---------------------------------------------------------------
    # Task 7: VRAM spill detection. `evaluate_spill` is pure, so every
    # case below is checked without a GPU present.
    # ---------------------------------------------------------------

    # A clean BF16 load on a 4 GB card: torch's reservation is accounted for
    # both by the driver's per-process figure and by the drop in free VRAM.
    clean = evaluate_spill(
        torch_reserved_mb=2200.0,
        free_before_mb=3800.0,
        free_after_mb=1560.0,
        process_vram_mb=2240.0,
        warmup_rtf=0.30,
        expected_rtf=0.35,
    )
    check("spill-clean-load", not clean["spilled"] and clean["reasons"] == [])

    # The signature case: 1.7B BF16 on a 4 GB card. torch believes it reserved
    # 3.9 GB but the driver only attributes 2.9 GB of VRAM to us, and free VRAM
    # barely moved. The load "succeeded".
    spilled = evaluate_spill(
        torch_reserved_mb=3900.0,
        free_before_mb=3800.0,
        free_after_mb=900.0,
        process_vram_mb=2900.0,
        warmup_rtf=0.40,
        expected_rtf=0.35,
    )
    check("spill-detected-by-accounting", spilled["spilled"])
    check("spill-reasons-explain-why", len(spilled["reasons"]) >= 1)

    # Throughput-only detection: the driver happily reports shared memory as
    # device memory, so the accounting looks clean, but inference is 10x slow.
    slow = evaluate_spill(
        torch_reserved_mb=2200.0,
        free_before_mb=3800.0,
        free_after_mb=1600.0,
        process_vram_mb=2200.0,
        warmup_rtf=3.6,
        expected_rtf=0.35,
    )
    check("spill-detected-by-throughput", slow["spilled"])
    check(
        "spill-throughput-reason-mentions-rtf",
        any("real-time factor" in r for r in slow["reasons"]),
    )

    # A slow-but-not-absurd RTF is not spill. A 2x miss is within the noise of
    # a busy laptop and must not block an otherwise valid configuration.
    borderline = evaluate_spill(
        torch_reserved_mb=2200.0,
        free_before_mb=3800.0,
        free_after_mb=1600.0,
        process_vram_mb=2200.0,
        warmup_rtf=0.70,
        expected_rtf=0.35,
    )
    check("spill-borderline-rtf-allowed", not borderline["spilled"])

    # An almost-full device is flagged even when the accounting balances: the
    # next allocation has nowhere to go.
    full = evaluate_spill(
        torch_reserved_mb=3700.0,
        free_before_mb=3800.0,
        free_after_mb=20.0,
        process_vram_mb=3700.0,
    )
    check("spill-detected-when-device-is-full", full["spilled"])

    # Unknown driver figures must not manufacture a verdict either way. WDDM
    # commonly refuses to report per-process memory.
    unknown = evaluate_spill(
        torch_reserved_mb=2200.0,
        free_before_mb=None,
        free_after_mb=None,
        process_vram_mb=None,
    )
    check("spill-unknown-is-not-a-false-positive", not unknown["spilled"])

    # Allocator overhead below the tolerance is not evidence of anything.
    noise = evaluate_spill(
        torch_reserved_mb=2200.0,
        free_before_mb=3800.0,
        free_after_mb=1700.0,
        process_vram_mb=2100.0,
    )
    check("spill-tolerance-absorbs-allocator-overhead", not noise["spilled"])

    # The report carries its inputs so the UI and the benchmark cache can both
    # explain the verdict instead of merely asserting it.
    check(
        "spill-report-carries-inputs",
        spilled["torch_reserved_mb"] == 3900.0
        and spilled["process_vram_mb"] == 2900.0
        and spilled["free_after_mb"] == 900.0,
    )

    # ---------------------------------------------------------------
    # Load-failure classification.
    # ---------------------------------------------------------------
    check(
        "classify-spill",
        classify_load_failure(SpillDetected("spilled into shared memory")) == "spill",
    )
    check(
        "classify-oom",
        classify_load_failure(RuntimeError("CUDA out of memory. Tried to allocate 2.00 GiB"))
        == "oom",
    )
    check(
        "classify-unsupported-precision",
        classify_load_failure(
            RuntimeError("Int4 weight-only quantization is not available in this torchao")
        )
        == "unsupported_precision",
    )
    check(
        "classify-weights-missing",
        classify_load_failure(FileNotFoundError("Model weights not found at C:/models"))
        == "weights_missing",
    )
    check("classify-timeout", classify_load_failure(TimeoutError("timed out")) == "timeout")
    check(
        "classify-crash",
        classify_load_failure(RuntimeError("CUDA error: an illegal memory access")) == "crash",
    )
    check("classify-unknown", classify_load_failure(ValueError("something else")) == "unknown")
    check(
        "classify-kinds-are-declared",
        all(
            classify_load_failure(e) in LOAD_FAILURE_KINDS
            for e in (
                SpillDetected("x"),
                RuntimeError("CUDA out of memory"),
                FileNotFoundError("x"),
                TimeoutError("x"),
                ValueError("x"),
            )
        ),
    )

    # ---------------------------------------------------------------
    # Task 16: shape bucketing.
    # ---------------------------------------------------------------
    check(
        "buckets-are-ascending",
        list(AUDIO_BUCKET_SECONDS) == sorted(AUDIO_BUCKET_SECONDS),
    )
    check(
        "bucket-covers-typical-utterance",
        bucket_for_seconds(5.0) == 8.0 and bucket_for_seconds(1.1) == 2.0,
    )
    check(
        "bucket-exact-boundary-does-not-round-up",
        bucket_for_seconds(4.0) == 4.0,
    )
    check(
        "bucket-beyond-largest-is-unpadded",
        bucket_for_seconds(45.0) is None,
    )
    check("bucket-zero-uses-smallest", bucket_for_seconds(0.0) == AUDIO_BUCKET_SECONDS[0])
    check(
        "warmup-covers-every-bucket-it-can",
        set(WARMUP_BUCKET_SECONDS_CUDA) >= set(AUDIO_BUCKET_SECONDS[:-1]),
    )

    # ---------------------------------------------------------------
    # Task 16: the decode cap must scale with speech, not truncate it.
    # ---------------------------------------------------------------
    check("explicit-model-label", _model_label("/parent/1.7/wrong", "0.6b") == "0.6B")
    check("explicit-model-bytes", expected_bytes_for("/1.7", "0.6b", 123) == 123)
    check("fallback-model-label", _model_label("/models/1.7b") == "1.7B")
    check("max-new-floor", max_new_tokens_for(0.0) == 32)
    check("max-new-scales", max_new_tokens_for(30.0) > max_new_tokens_for(5.0))
    check("max-new-ceiling", max_new_tokens_for(600.0) <= 384)
    # A 5 s utterance is ~12 words, ~14 tokens. The cap must clear that
    # comfortably or transcripts get cut off.
    check("max-new-5s-has-headroom", max_new_tokens_for(5.0) >= 60)
    # A 2 minute dictation is ~300 words. The old formula capped at 128 tokens,
    # which truncated it.
    check("max-new-120s-fits-long-prose", max_new_tokens_for(120.0) >= 340)
    check("max-new-monotonic", all(
        max_new_tokens_for(s) <= max_new_tokens_for(s + 1.0) for s in range(0, 200)
    ))

    # ---------------------------------------------------------------
    # Task 16: vectorized frame energy must match the original loop.
    # ---------------------------------------------------------------
    rng = np.random.default_rng(7)
    for n in (0, 1, 100, 4001, SAMPLE_RATE * 3 + 7):
        sig = rng.standard_normal(n).astype(np.float32) * 0.1
        check(
            f"frame-energy-matches-reference-n{n}",
            _frame_energies_match_reference(sig),
        )

    return ok


def _frame_energies_match_reference(samples: np.ndarray) -> bool:
    """Compare the vectorized frame-energy helper against a literal loop."""
    hop = FRAME_HOP
    win = FRAME_WIN
    reference = []
    i = 0
    while i + win <= samples.size:
        frame = samples[i : i + win]
        reference.append(float(np.sqrt(np.mean(frame * frame))))
        i += hop
    fast = frame_energies(samples)
    if len(fast) != len(reference):
        return False
    return all(abs(a - b) < 1e-5 for a, b in zip(fast, reference))


_cmd_queue = queue.Queue()
_write_lock = threading.Lock()


def _write_response(resp: dict):
    with _write_lock:
        sys.stdout.write(json.dumps(resp) + "\n")
        sys.stdout.flush()


def _respond(msg: dict, resp: dict):
    """Write a reply, echoing the request's `id` so the caller can match it.

    Replies are no longer guaranteed to come back in request order: `status`,
    `ping` and `cancel_stream` are answered immediately on the reader thread
    while a slow command may still be in flight on the worker. Echoing the id
    is what makes that safe — the caller matches replies to requests instead of
    assuming strict FIFO, and a late reply after a timeout is discarded rather
    than misread as the answer to the next request.
    """
    request_id = msg.get("id")
    if request_id is not None:
        resp = dict(resp)
        resp["id"] = request_id
    _write_response(resp)


def _command_worker():
    while True:
        msg = _cmd_queue.get()
        if msg is None:
            break
        try:
            resp = handle(msg)
        except Exception as e:
            resp = {"status": "error", "error": str(e)}
        if not msg.pop("_ack_early", False):
            # load_model was already acked by the main thread before the
            # import warmup; sending this reply too would duplicate it.
            _respond(msg, resp)
        _cmd_queue.task_done()


def main():
    log_err(f"Qwen3-ASR sidecar started (pid {os.getpid()}). Listening on stdin...")
    threading.Thread(target=_inference_worker, name="asr-worker", daemon=True).start()
    threading.Thread(target=_command_worker, name="cmd-worker", daemon=True).start()

    def warm_imports_safe():
        try:
            _warm_imports()
        except Exception as e:
            # Let the loader surface the real import error through the normal
            # load-failure path rather than killing the reader loop here.
            log_err(f"Warm import failed: {e}")

    stream = sys.stdin.buffer
    # The load path's native extensions are imported HERE, on the main thread,
    # once, on the first load/install command (see _warm_imports). No worker
    # thread in this process ever runs a cold import, which is what deadlocks
    # module init against this thread's blocked CRT stdio state.
    while True:
        try:
            b = stream.read(1)
            if not b:
                break
        except Exception:
            break

        if b in (b"\r", b"\n", b" ", b"\t"):
            continue

        if b == b"\x01":
            # Direct binary audio transport frame:
            # 0x01 <session_u32> <seq_u32> <len_u32> <raw_pcm_bytes>
            try:
                header = _read_exact(stream, 12)
                _session_id, _seq_id, data_len = unpack_binary_audio_frame(header)
                pcm_bytes = _read_exact(stream, data_len)
                STATE.stream_audio.extend(pcm_bytes)
                STATE.unprocessed += len(pcm_bytes)
                if LIVE_PARTIALS:
                    _maybe_kick_partial(getattr(STATE, "stream_language", None))
            except Exception as e:
                log_err(f"Binary audio frame read error: {e}")
                break
        else:
            # JSON-lines control message
            try:
                rest = stream.readline()
                line = (b + rest).decode("utf-8", errors="replace").strip()
                if not line:
                    continue
                msg = json.loads(line)
            except Exception as e:
                _write_response({"status": "error", "error": f"Invalid JSON: {e}"})
                continue

            cmd = msg.get("cmd")
            if cmd == "cancel_stream":
                _CANCEL_EVENT.set()
                STATE.stream_audio = bytearray()
                STATE.unprocessed = 0
                STATE.partial_text = ""
                _respond(msg, {"status": "ok"})
            elif cmd in ("ping", "status"):
                # Fast lane. These are read-only probes that touch no torch and
                # no model state, so answering them here keeps them responsive
                # even while `cmd-worker` is blocked in a transcription that
                # may legitimately run for minutes. Routing them through the
                # single FIFO queue meant a status poll inherited the latency of
                # whatever was ahead of it, and the caller's short status
                # budget then killed a perfectly healthy sidecar.
                try:
                    _respond(msg, handle(msg))
                except Exception as e:
                    _respond(msg, {"status": "error", "error": str(e)})
            else:
                if cmd == "probe_cuda":
                    # Ack immediately, then run the warm imports (which
                    # compute the CUDA capability snapshot) HERE on the main
                    # thread — the only thread where a cold torch import is
                    # safe (see the native-deadlock note above _warm_imports).
                    #
                    # This exists so the caller can ask for the CUDA answer
                    # *before* committing to a model load. The load decision
                    # needs `cuda_available`, but the probe only ran inside
                    # the first `load_model` — so a decision made before any
                    # load always saw "pending" and picked the CPU path on
                    # GPU-capable machines. The ack must not wait on the
                    # warmup: it can take 10-60s on a cold filesystem, and a
                    # timed-out command tears the sidecar down.
                    _respond(msg, {"status": "probing"})
                    warm_imports_safe()
                elif cmd == "load_model":
                    # Ack immediately, then warm the load path's native
                    # extensions HERE on the main thread before any worker
                    # can start importing (see _warm_imports). The ack must
                    # not wait on the warmup: a cold machine can take longer
                    # than the caller's command budget, and a timed-out
                    # load_model tears the sidecar down.
                    #
                    # Publish the pending load *before* the warmup blocks this
                    # thread. The warmup stops us answering `status` at all for
                    # its duration, so the first status served afterwards must
                    # already say "loading" — otherwise the caller sees a
                    # not-loaded/not-loading sidecar and concludes the load
                    # failed.
                    STATE.load_pending = True
                    STATE.loaded = False
                    STATE.load_error = None
                    STATE.backend = "loading…"
                    _respond(msg, {"status": "loading"})
                    warm_imports_safe()
                    _cmd_queue.put(dict(msg, _ack_early=True))
                elif cmd == "install_model":
                    _cmd_queue.put(msg)
                    # The install downloads for minutes before it auto-loads;
                    # warming after enqueue finishes long before the auto-load
                    # needs the modules, and keeps the install ack instant.
                    warm_imports_safe()
                else:
                    _cmd_queue.put(msg)

    _cmd_queue.put(None)
    log_err("stdin closed; sidecar exiting")


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] in ("--selftest", "selftest"):
        sys.exit(0 if _selftest() else 1)
    main()
