"""Exercise Reflow's real download/load/transcription path on CPU and CUDA.

python scripts/verify_phonon.py --audio speech.wav --install
Downloads use Reflow's network policy and journal. Existing weights stay offline.
"""
import argparse
import json
import math
import os
import pathlib
import sys
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "model-runtime"))
import qwen3_asr_runtime as runtime
from phonon_runtime import ARCHIVE, ARCHIVE_SHA256, REVISION


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--audio", type=pathlib.Path, required=True)
    parser.add_argument("--model-dir", type=pathlib.Path,
                        default=pathlib.Path(os.environ.get("APPDATA", pathlib.Path.home() / ".local/share")) / "reflow/models/phonon-2")
    parser.add_argument("--install", action="store_true")
    parser.add_argument("--device", choices=["cpu", "cuda", "both"], default="both")
    parser.add_argument("--output", type=pathlib.Path)
    parser.add_argument("--reference", help="Expected words, ignoring case and punctuation")
    args = parser.parse_args()
    os.environ.setdefault("REFLOW_NETWORK_POLICY", str(args.model_dir.parent.parent / "network-policy.json"))
    runtime._warm_imports()
    if args.install and not (args.model_dir / ARCHIVE).is_file():
        result = runtime.start_install(str(args.model_dir), "FermionResearch/Phonon-2", "phonon-2", 163515201,
                                       REVISION, [{"filename": ARCHIVE, "sha256": ARCHIVE_SHA256}])
        if result["status"] not in ("downloading", "already-downloading"):
            raise RuntimeError(result)
        while any(t.name == "model-installer" and t.is_alive() for t in runtime.threading.enumerate()):
            time.sleep(0.5)
        if runtime.STATE.download_error:
            raise RuntimeError(runtime.STATE.download_error)
        while runtime._load_thread_alive():
            time.sleep(0.5)
        runtime._unload_model_blocking()
    import soundfile as sf
    import numpy as np
    from scipy.signal import resample_poly
    wave, rate = sf.read(args.audio, dtype="float32", always_2d=True)
    wave = wave.mean(axis=1)
    if rate != runtime.SAMPLE_RATE:
        divisor = math.gcd(rate, runtime.SAMPLE_RATE)
        wave = resample_poly(wave, runtime.SAMPLE_RATE // divisor, rate // divisor)
    pcm = (np.clip(wave, -1, 1) * 32767).astype("<i2").tobytes()
    rows = []
    devices = ["cpu", "cuda"] if args.device == "both" else [args.device]
    for device in devices:
        result = runtime.load_model_blocking(str(args.model_dir), device, "auto", "phonon-2")
        if result["status"] != "ok":
            raise RuntimeError(result)
        if runtime.STATE.device != device:
            raise AssertionError(f"Requested {device}, but runtime loaded {runtime.STATE.device}; cannot verify this device")
        runtime._CANCEL_EVENT.clear()
        started = time.perf_counter()
        text, language, warnings = runtime.transcribe_segmented(pcm, "English", None)
        elapsed = time.perf_counter() - started
        if not text.strip() or language != "en":
            raise AssertionError(f"Expected English speech: {text!r}, {language!r}")
        if args.reference:
            import re
            if re.findall(r"\w+", text.lower()) != re.findall(r"\w+", args.reference.lower()):
                raise AssertionError(f"Transcript differs from reference: {text!r}")
        rows.append({"device": device, "precision": runtime.STATE.precision, "backend": runtime.STATE.backend,
                     "load_seconds": runtime.STATE.load_seconds, "audio_seconds": len(wave) / runtime.SAMPLE_RATE,
                     "transcribe_seconds": elapsed, "text": text, "warnings": warnings})
        print(json.dumps(rows[-1]), flush=True)
        runtime._unload_model_blocking()
    if args.output:
        args.output.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
