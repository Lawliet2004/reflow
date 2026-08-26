#!/usr/bin/env python3
"""Standalone verification of the Reflow ASR sidecar with real speech audio.

Drives model-runtime/qwen3_asr_runtime.py exactly the way the Rust app does:
load_model (GPU-first) -> start_stream -> push_audio_b64 chunks -> stop_stream.

Usage:  python scripts/test_sidecar.py [path/to/test.wav]
"""

import json
import subprocess
import sys
import time
import wave
import os

HERE = os.path.dirname(os.path.abspath(__file__))
SIDECAR = os.path.join(HERE, "..", "model-runtime", "qwen3_asr_runtime.py")
MODEL_DIR = os.environ.get(
    "REFLOW_MODEL_DIR",
    os.path.join(
        os.environ.get("APPDATA", os.path.expanduser("~")),
        "reflow", "models", "qwen3-asr-1.7b",
    ),
)


def main():
    wav_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "test_speech.wav")
    print(f"sidecar : {os.path.abspath(SIDECAR)}")
    print(f"model   : {MODEL_DIR}")
    print(f"audio   : {wav_path}")

    with wave.open(wav_path, "rb") as w:
        rate = w.getframerate()
        frames = w.readframes(w.getnframes())
        channels = w.getnchannels()
        width = w.getsampwidth()

    if channels > 1:
        import audioop
        frames = audioop.tomono(frames, width, 0.5, 0.5)
    if rate != 16000:
        import audioop
        frames, _ = audioop.ratecv(frames, width, 1, rate, 16000, None)

    proc = subprocess.Popen(
        [sys.executable, SIDECAR],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        bufsize=1,
        text=True,
        encoding="utf-8",
    )

    def send(payload):
        proc.stdin.write(json.dumps(payload) + "\n")
        proc.stdin.flush()
        line = proc.stdout.readline()
        if not line:
            raise RuntimeError("sidecar closed")
        return json.loads(line)

    t0 = time.time()
    pong = send({"cmd": "ping"})
    assert pong.get("pong"), pong
    print(f"[{time.time()-t0:5.1f}s] ping ok")

    resp = send({"cmd": "load_model", "model_dir": MODEL_DIR, "device": "auto"})
    print(f"[{time.time()-t0:5.1f}s] load_model -> {resp}")
    assert resp.get("status") in ("loading", "ok"), resp

    # poll status until loaded (or fail)
    while True:
        st = send({"cmd": "status"})
        if st.get("loaded"):
            break
        if st.get("error"):
            raise RuntimeError(f"load failed: {st['error']}")
        time.sleep(1.0)
    print(f"[{time.time()-t0:5.1f}s] model loaded on {st['device']} ({st['backend']})")

    resp = send({"cmd": "start_stream", "language": "auto", "vocabulary": ["Reflow"]})
    assert resp.get("status") == "ok", resp
    print(f"[{time.time()-t0:5.1f}s] stream started")

    chunk = 3200 * 2  # 200ms of PCM16
    partial_seen = None
    for i in range(0, len(frames), chunk):
        import base64
        piece = frames[i : i + chunk]
        resp = send({
            "cmd": "push_audio_b64",
            "audio_b64": base64.b64encode(piece).decode(),
        })
        assert resp.get("status") == "ok", resp
        if resp.get("text") and not partial_seen:
            partial_seen = resp["text"]
            print(f"[{time.time()-t0:5.1f}s] first partial: {partial_seen!r}")
        time.sleep(0.05)  # roughly realtime pacing

    resp = send({"cmd": "stop_stream"})
    print(f"[{time.time()-t0:5.1f}s] final: {resp.get('text')!r} (lang={resp.get('language')})")

    send({"cmd": "unload_model"})
    proc.stdin.close()
    proc.wait(timeout=10)

    text = (resp.get("text") or "").lower()
    ok = ("fox" in text and "dog" in text) or "reflow" in text or len(text.split()) >= 8
    print("\nRESULT:", "PASS" if ok else "FAIL", "-", text[:120])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
