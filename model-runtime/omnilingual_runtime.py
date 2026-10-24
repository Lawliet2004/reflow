"""Pinned 300M INT8 Omnilingual v2 CTC adapter for sherpa-onnx 1.13.4.

Uses the published archive as the integrity boundary: no remote code executes.
Archive contents are copied explicitly, never extracted as arbitrary paths.
"""
import hashlib
import shutil
import os
import pathlib
import tarfile
import threading

import numpy as np
from zipformer_runtime import require_dependencies

MODEL_ID = "omnilingual-300m-int8"
ARCHIVE = "sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-v2-int8-2026-02-05.tar.bz2"
SHA256 = "b72bef9be75862684098e722d79fefecf9f10fefd3a0b2950738977b4c6b4147"


# Digests and sizes of regular members from the SHA256-pinned archive above.
EXPECTED_FILES = {
    "model.int8.onnx": ("440facfa94ed53cd132aca02db450f36e8fc0cdf2af3c5d80021921df7f18cff", 328006844),
    "tokens.txt": ("14aab5cabf425ea1a5efdce2bcb74a24f836947baeab52cfb720eacd28492974", 100918),
}


def check_cancel(cancel_event):
    if cancel_event is not None and cancel_event.is_set():
        raise RuntimeError("Omnilingual load cancelled")


def checked_digest(path, cancel_event):
    result = hashlib.sha256()
    with path.open("rb") as source:
        while True:
            check_cancel(cancel_event)
            chunk = source.read(1024 * 1024)
            if not chunk:
                break
            result.update(chunk)
    return result.hexdigest()


def prepare_archive(model_dir, cancel_event=None):
    root = pathlib.Path(model_dir)
    check_cancel(cancel_event)
    if all(
        (root / name).is_file() and not (root / name).is_symlink()
        and (root / name).stat().st_size == size
        and checked_digest(root / name, cancel_event) == sha
        for name, (sha, size) in EXPECTED_FILES.items()
    ):
        return root
    archive = root / ARCHIVE
    if not archive.is_file():
        raise FileNotFoundError("Omnilingual archive is missing. Download it in Model settings.")
    if checked_digest(archive, cancel_event) != SHA256:
        raise ValueError("Omnilingual archive SHA-256 mismatch. Remove and download it again.")
    needed = sum(size for _, size in EXPECTED_FILES.values()) + 16 * 1024 * 1024
    if shutil.disk_usage(root).free < needed:
        raise OSError(f"Omnilingual needs {needed // (1024 * 1024)} MB of free disk space to prepare its files. Free space and retry.")

    class CancellableReader:
        def __init__(self, source):
            self.source = source

        def read(self, size=-1):
            check_cancel(cancel_event)
            return self.source.read(size)

    temporary = {name: root / (name + ".part") for name in EXPECTED_FILES}
    seen = set()
    try:
        for path in temporary.values():
            path.unlink(missing_ok=True)
        # Stream once; cancellation also runs during decompression/header scans.
        with archive.open("rb") as raw, tarfile.open(fileobj=CancellableReader(raw), mode="r|bz2") as bundle:
            for member in bundle:
                check_cancel(cancel_event)
                name = pathlib.PurePosixPath(member.name).name
                if name not in EXPECTED_FILES:
                    continue
                if name in seen or not member.isfile():
                    raise ValueError(f"Omnilingual archive must contain exactly one regular {name}")
                seen.add(name)
                sha, size = EXPECTED_FILES[name]
                if member.size != size:
                    raise ValueError(f"Invalid Omnilingual archive member size: {name}")
                result = hashlib.sha256()
                with bundle.extractfile(member) as source, temporary[name].open("xb") as output:
                    while chunk := source.read(1024 * 1024):
                        check_cancel(cancel_event)
                        result.update(chunk)
                        output.write(chunk)
                if result.hexdigest() != sha:
                    raise ValueError(f"Omnilingual extracted file SHA-256 mismatch: {name}")
        if seen != set(EXPECTED_FILES):
            raise ValueError("Omnilingual archive is missing required model files.")
        check_cancel(cancel_event)
        for name, path in temporary.items():
            os.replace(path, root / name)
    finally:
        for path in temporary.values():
            path.unlink(missing_ok=True)
    return root


class OmnilingualModel:
    model_id = MODEL_ID
    precision = "int8"
    runtime = "sherpa-onnx"
    device = "cpu"

    def __init__(self, model_dir, device, cancel_event):
        require_dependencies()
        if device != "cpu":
            raise ValueError("Omnilingual INT8 uses the CPU runtime.")
        root = prepare_archive(model_dir, cancel_event)
        import sherpa_onnx
        self.recognizer = sherpa_onnx.OfflineRecognizer.from_omnilingual_asr_ctc(
            model=str(root / "model.int8.onnx"), tokens=str(root / "tokens.txt"),
            num_threads=max(1, min(4, os.cpu_count() or 2)), provider="cpu",
            decoding_method="greedy_search",
        )
        self.cancel_event = cancel_event
        self.lock = threading.Lock()

    def transcribe(self, samples):
        if self.cancel_event.is_set():
            return ""
        with self.lock:
            stream = self.recognizer.create_stream()
            stream.accept_waveform(16000, np.asarray(samples, dtype=np.float32).reshape(-1))
            self.recognizer.decode_stream(stream)
            return "" if self.cancel_event.is_set() else (stream.result.text or "").strip()
