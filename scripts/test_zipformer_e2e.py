#!/usr/bin/env python3
"""End-to-end check of the Zipformer path through the real sidecar protocol.

Drives model-runtime/qwen3_asr_runtime.py exactly the way the Rust engine does:
binary 0x01 PCM frames, a get_partial probe per push, stop_stream for the
final transcript, then a second record/stop cycle on the same recognizer.

Usage: python scripts/test_zipformer_e2e.py [model_dir] [wav]
"""

import base64
import atexit
import json
import os
import queue
import struct
import subprocess
import sys
import threading
import time
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
SIDECAR = os.path.join(HERE, "..", "model-runtime", "qwen3_asr_runtime.py")
MODEL_DIR = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "REFLOW_ZIPFORMER_DIR", "/tmp/zipformer-20m"
)
WAV = sys.argv[2] if len(sys.argv) > 2 else os.path.join(MODEL_DIR, "test_wavs", "0.wav")
EXPECTED = "after early nightfall the yellow lamps would light up here and there the squalid quarter"


def read_reply(proc, want_id=None, timeout=60.0):
    if not hasattr(proc, "reflow_replies"):
        proc.reflow_replies = queue.Queue()

        def read_lines():
            try:
                for line in proc.stdout:
                    proc.reflow_replies.put(json.loads(line))
            except Exception as error:
                proc.reflow_replies.put(error)
            finally:
                proc.reflow_replies.put(None)

        threading.Thread(target=read_lines, daemon=True).start()
    deadline = time.monotonic() + timeout
    while True:
        try:
            msg = proc.reflow_replies.get(timeout=max(0, deadline - time.monotonic()))
        except queue.Empty as error:
            raise TimeoutError("timed out waiting for reply") from error
        if msg is None:
            raise RuntimeError("sidecar closed")
        if isinstance(msg, Exception):
            raise RuntimeError(f"invalid sidecar reply: {msg}") from msg
        if want_id is None or msg.get("id") == want_id:
            return msg


def shutdown_sidecar(proc):
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=10)


def send(proc, payload, timeout=60.0):
    proc.stdin.write((json.dumps(payload) + "\n").encode())
    proc.stdin.flush()
    return read_reply(proc, payload.get("id"), timeout)


def main():
    with wave.open(WAV, "rb") as w:
        assert w.getframerate() == 16000 and w.getsampwidth() == 2 and w.getnchannels() == 1
        pcm = w.readframes(w.getnframes())

    proc = subprocess.Popen(
        [sys.executable, SIDECAR],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=sys.stderr,
        bufsize=0,
    )
    atexit.register(shutdown_sidecar, proc)
    rid = [0]

    def cmd(payload):
        rid[0] += 1
        payload["id"] = rid[0]
        return send(proc, payload)

    t0 = time.time()
    assert cmd({"cmd": "ping"}).get("pong")
    print(f"[{time.time()-t0:5.1f}s] ping ok")

    resp = cmd({"cmd": "load_model", "model_id": "zipformer-20m", "model_dir": MODEL_DIR,
                "device": "auto", "precision": "auto"})
    assert resp.get("status") in ("ok", "loading"), resp
    while True:
        st = cmd({"cmd": "status"})
        if st.get("loaded"):
            break
        assert not st.get("error"), st["error"]
        time.sleep(0.5)
    print(f"[{time.time()-t0:5.1f}s] loaded: {st['backend']} ({st['device']}/{st.get('precision')})")

    # Non-English must be rejected before recording state changes.
    bad = cmd({"cmd": "start_stream", "language": "hi"})
    assert bad.get("status") == "error" and "English" in bad["error"], bad
    print(f"[{time.time()-t0:5.1f}s] hi rejected: {bad['error']}")

    frames_total = 0
    for cycle in (1, 2):
        resp = cmd({"cmd": "start_stream", "language": "en", "vocabulary": []})
        assert resp.get("status") == "ok", resp
        partials = []
        chunk = 16000 * 2  # 1 s of PCM16 per frame, matching the app's batching
        for off in range(0, len(pcm), chunk):
            frame = pcm[off : off + chunk]
            proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(frame)) + frame)
            proc.stdin.flush()
            frames_total += 1
            part = cmd({"cmd": "get_partial"})
            if part.get("text"):
                partials.append(part["text"])
        resp = cmd({"cmd": "stop_stream"})
        assert resp.get("status") == "ok", resp
        text = (resp.get("text") or "").strip().lower()
        print(f"[{time.time()-t0:5.1f}s] cycle {cycle} final: {text!r}")
        assert resp.get("language") == "en", resp
        assert EXPECTED in text, f"missing words: {text!r}"
        assert partials, "no live partials arrived during dictation"
        # The last live partial and the final must agree at the head; the tail
        # is the revisable hypothesis and may differ by a word or two.
        last_head = " ".join(partials[-1].lower().split()[:4])
        assert text.startswith(last_head), \
            f"stale partial survived into final: {partials[-1]!r} -> {text!r}"
        print(f"[{time.time()-t0:5.1f}s]   {len(partials)} partials, last={partials[-1]!r}")

    # Silence-only stream must not hang or emit spurious text.
    cmd({"cmd": "start_stream", "language": "en", "vocabulary": []})
    for _ in range(3):
        silence = b"\x00\x00" * 16000
        proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(silence)) + silence)
        proc.stdin.flush()
    resp = cmd({"cmd": "stop_stream"})
    assert resp.get("status") == "ok", resp
    assert len((resp.get("text") or "").split()) <= 3, resp

    # Cancellation: stream dropped, next dictation unaffected.
    cmd({"cmd": "start_stream", "language": "en", "vocabulary": []})
    proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(pcm[:32000])) + pcm[:32000])
    proc.stdin.flush()
    proc.stdin.write(b'{"cmd": "cancel_stream", "id": 900}\n')
    proc.stdin.flush()
    resp = read_reply(proc, want_id=900)
    assert resp.get("status") == "ok", resp
    cmd({"cmd": "start_stream", "language": "en", "vocabulary": []})
    proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(pcm)) + pcm)
    proc.stdin.flush()
    resp = cmd({"cmd": "stop_stream"})
    assert EXPECTED in (resp.get("text") or "").lower(), resp

    cmd({"cmd": "unload_model"})
    proc.stdin.close()
    proc.wait(timeout=10)
    print(f"\nRESULT: PASS — {frames_total} frames, 3 sessions, cancel, silence, en-gate")
    sys.exit(0)


if __name__ == "__main__":
    main()
