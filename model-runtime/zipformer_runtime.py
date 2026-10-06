"""Sherpa-ONNX streaming Zipformer adapter. English int8 weights, CPU only.

The weights are the pinned csukuangfj 20M int8 ONNX export; inference is the
official sherpa-onnx OnlineRecognizer. Unlike Reflow's buffered engines this
one decodes as audio arrives, so `accept_pcm16` runs on the reader thread and
`finish_stream` only drains the tail. Endpoint-finalized utterances are kept
as committed text and the sherpa stream is reset, which bounds feature state
over long dictations.
"""
import hashlib
import os
import pathlib
import threading

import numpy as np

MODEL_ID = "zipformer-20m"
REVISION = "d42f2d9f7ca24806fb667456a18a9f1b60f70d16"
SAMPLE_RATE = 16000

ENCODER = "encoder-epoch-99-avg-1.int8.onnx"
DECODER = "decoder-epoch-99-avg-1.int8.onnx"
JOINER = "joiner-epoch-99-avg-1.int8.onnx"
TOKENS = "tokens.txt"

FILES = {
    ENCODER: "3810755ce7c3ab26b42a8bcf39d191308fa27fb0f53358823ba46141d03b7eb3",
    DECODER: "21e2a2acd961b3ac72f55be2f10f1a285e1b0b0ba010d7c0b6eab141411b163c",
    JOINER: "e085d73b593cf9b0707f370dbd656d58327d3fe36d80d849202ef81df02cb01e",
    TOKENS: "49e3c2646595fd907228b3c6787069658f67b17377c60aeb8619c4551b2316fb",
}


def require_dependencies():
    try:
        import sherpa_onnx  # noqa: F401
        from importlib.metadata import version
        if version("sherpa-onnx") != "1.13.4":
            raise ImportError("Reflow requires sherpa-onnx==1.13.4")
    except ImportError as error:
        raise RuntimeError(
            "Install the Zipformer runtime with: python -m pip install -r "
            "model-runtime/requirements-zipformer.txt (beside Reflow's runtime)."
        ) from error


def digest(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def _result_text(recognizer, stream) -> str:
    """`get_result_all` carries the OnlineRecognizerResult; older bindings only
    have `get_result`, which already returns the plain string."""
    get_all = getattr(recognizer, "get_result_all", None)
    if get_all is not None:
        return (get_all(stream).text or "").strip()
    return (recognizer.get_result(stream) or "").strip()


class ZipformerModel:
    """Loaded recognizer plus the in-flight stream state.

    All public entry points take `self.lock`: audio arrives on the sidecar's
    reader thread while `finish_stream`/`cancel_stream` run on cmd-worker.
    """

    model_id = MODEL_ID

    def __init__(self, model_dir, device, cancel_event):
        require_dependencies()
        if device != "cpu":
            raise ValueError("Zipformer 20M is CPU-optimized; it does not use the GPU.")
        root = pathlib.Path(model_dir)
        for name, expected in FILES.items():
            path = root / name
            if not path.is_file():
                raise FileNotFoundError(
                    f"Zipformer weights are missing ({name}). Download them in Model settings."
                )
            if digest(path) != expected:
                raise ValueError(
                    f"Zipformer SHA-256 mismatch for {name}. Remove and download the model again."
                )
        import sherpa_onnx

        # A 20M transducer saturates well below two cores; more threads buy
        # nothing and would contend with the rest of the app.
        threads = max(1, min(2, os.cpu_count() or 2))
        self.recognizer = sherpa_onnx.OnlineRecognizer.from_transducer(
            encoder=str(root / ENCODER),
            decoder=str(root / DECODER),
            joiner=str(root / JOINER),
            tokens=str(root / TOKENS),
            num_threads=threads,
            sample_rate=SAMPLE_RATE,
            feature_dim=80,
            decoding_method="modified_beam_search",
            max_active_paths=4,
            enable_endpoint_detection=True,
            provider="cpu",
            model_type="zipformer",
        )
        self.device = device
        self.cancel_event = cancel_event
        # Reentrant: accept_pcm16 reports partial_text() under the lock.
        self.lock = threading.RLock()
        self.stream = None
        self.committed = []
        self.precision = "int8"
        self.runtime = "sherpa-onnx"
        # This 20M streaming encoder drops speech that starts at t=0 of a
        # fresh stream (verified: ~0.4s of lead-in is the knee). Half a second
        # of zeros gives it the context it needs without firing an endpoint.
        self._head_pad = np.zeros(int(0.5 * SAMPLE_RATE), dtype=np.float32)
        # Prime the ONNX sessions so the first dictated words do not pay the
        # cold-arena cost.
        warm = self.recognizer.create_stream()
        warm.accept_waveform(SAMPLE_RATE, np.zeros(int(0.4 * SAMPLE_RATE), dtype=np.float32))
        warm.input_finished()
        while self.recognizer.is_ready(warm):
            self.recognizer.decode_stream(warm)

    def start_stream(self):
        with self.lock:
            self.stream = self.recognizer.create_stream()
            self.stream.accept_waveform(SAMPLE_RATE, self._head_pad)
            self.committed = []

    def accept_pcm16(self, pcm16: bytes) -> str:
        """Decode incoming PCM16. Returns the live transcript for display."""
        if self.cancel_event.is_set():
            return ""
        samples = np.frombuffer(pcm16, dtype=np.int16).astype(np.float32) / 32768.0
        if samples.size == 0:
            return self.partial_text()
        with self.lock:
            stream = self.stream
            if stream is None:
                return ""
            stream.accept_waveform(SAMPLE_RATE, samples)
            while not self.cancel_event.is_set() and self.recognizer.is_ready(stream):
                self.recognizer.decode_stream(stream)
            if self.recognizer.is_endpoint(stream):
                # is_endpoint implies an already-decoded segment; finalize it
                # and reset so feature state stays bounded over long sessions.
                text = _result_text(self.recognizer, stream)
                if text:
                    self.committed.append(text)
                self.recognizer.reset(stream)
                # A reset stream has the same cold-start drop; re-pad.
                stream.accept_waveform(SAMPLE_RATE, self._head_pad)
            return self.partial_text()

    def partial_text(self) -> str:
        """Committed endpoints plus the current revisable hypothesis."""
        with self.lock:
            if self.stream is None:
                return ""
            head = _result_text(self.recognizer, self.stream)
        parts = [*self.committed, head]
        return " ".join(p for p in parts if p).strip()

    def finish_stream(self) -> str:
        """stop_stream: mark input finished, drain the decoder, return text."""
        with self.lock:
            stream = self.stream
            self.stream = None
        if stream is None:
            return " ".join(self.committed).strip()
        stream.input_finished()
        while not self.cancel_event.is_set() and self.recognizer.is_ready(stream):
            self.recognizer.decode_stream(stream)
        tail = "" if self.cancel_event.is_set() else _result_text(self.recognizer, stream)
        parts = [*self.committed, tail]
        self.committed = []
        return " ".join(p for p in parts if p).strip()

    def cancel_stream(self):
        with self.lock:
            self.stream = None
            self.committed = []

    def transcribe(self, samples):
        """Batch transcription for the shared one-shot paths and tests."""
        if self.cancel_event.is_set():
            return ""
        stream = self.recognizer.create_stream()
        stream.accept_waveform(SAMPLE_RATE, self._head_pad)
        stream.accept_waveform(SAMPLE_RATE, np.asarray(samples, dtype=np.float32).reshape(-1))
        tail = np.zeros(int(0.5 * SAMPLE_RATE), dtype=np.float32)
        stream.accept_waveform(SAMPLE_RATE, tail)
        stream.input_finished()
        while not self.cancel_event.is_set() and self.recognizer.is_ready(stream):
            self.recognizer.decode_stream(stream)
        return "" if self.cancel_event.is_set() else _result_text(self.recognizer, stream)
