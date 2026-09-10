#!/usr/bin/env python3
"""Diagnose the Reflow ASR sidecar using the *exact* transport the Rust app uses.

Differences from scripts/test_sidecar.py that matter for this diagnosis:
  * audio is pushed with the 13-byte binary frame header (0x01 ...), which is
    what src-tauri/src/asr/sidecar.rs::push_audio actually writes;
  * the sidecar's stderr is inherited so its log lines are visible live;
  * every step reports elapsed time and the child's exit status, so a silent
    death is distinguishable from a hang.

Usage: python scripts/diag_sidecar.py [path/to/test.wav]
"""

import json
import os
import struct
import subprocess
import sys
import threading
import time
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
SIDECAR = os.path.normpath(os.path.join(HERE, "..", "model-runtime", "qwen3_asr_runtime.py"))
MODEL_DIR = os.environ.get(
    "REFLOW_MODEL_DIR",
    os.path.join(
        os.environ.get("APPDATA", os.path.expanduser("~")),
        "reflow", "models", "qwen3-asr-1.7b",
    ),
)
PRECISION = os.environ.get("REFLOW_PRECISION", "int8")

T0 = time.time()


def say(msg):
    print(f"[{time.time() - T0:6.1f}s] {msg}", flush=True)


def load_wav_16k_mono(path):
    with wave.open(path, "rb") as w:
        rate, ch, width = w.getframerate(), w.getnchannels(), w.getsampwidth()
        frames = w.readframes(w.getnframes())
    if ch > 1 or rate != 16000:
        import audioop
        if ch > 1:
            frames = audioop.tomono(frames, width, 0.5, 0.5)
        if rate != 16000:
            frames, _ = audioop.ratecv(frames, width, 1, rate, 16000, None)
    return frames


def main():
    wav = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "test_speech.wav")
    pcm = load_wav_16k_mono(wav)
    say(f"sidecar={SIDECAR}")
    say(f"model={MODEL_DIR} precision={PRECISION}")
    say(f"audio={wav} ({len(pcm) / 2 / 16000:.1f}s of 16k mono PCM16)")

    proc = subprocess.Popen(
        [sys.executable, "-u", SIDECAR],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=None,  # inherit: show the sidecar's own log lines
    )

    replies = {}
    lock = threading.Condition()
    dead = threading.Event()

    def reader():
        for raw in proc.stdout:
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except Exception:
                say(f"NON-JSON from sidecar: {line[:200]!r}")
                continue
            with lock:
                replies[msg.get("id")] = msg
                lock.notify_all()
        dead.set()
        with lock:
            lock.notify_all()

    threading.Thread(target=reader, daemon=True).start()

    counter = [0]

    def send(payload, timeout):
        counter[0] += 1
        rid = counter[0]
        payload = dict(payload, id=rid)
        proc.stdin.write((json.dumps(payload) + "\n").encode())
        proc.stdin.flush()
        deadline = time.time() + timeout
        with lock:
            while rid not in replies:
                if dead.is_set():
                    raise RuntimeError(
                        f"sidecar died (returncode={proc.poll()}) waiting for {payload['cmd']}"
                    )
                if not lock.wait(max(0.05, deadline - time.time())):
                    if time.time() >= deadline:
                        raise TimeoutError(
                            f"{payload['cmd']} did not answer within {timeout}s"
                        )
            return replies.pop(rid)

    try:
        say(f"ping -> {send({'cmd': 'ping'}, 5)}")
        say(
            "load_model -> "
            + str(send(
                {
                    "cmd": "load_model",
                    "model_dir": MODEL_DIR,
                    "device": "auto",
                    "precision": PRECISION,
                },
                90,
            ))
        )

        # Poll status the way spawn_model_status_watch does: every 700ms,
        # 20s budget per poll.
        loaded = False
        for i in range(200):
            try:
                st = send({"cmd": "status"}, 20)
            except TimeoutError as e:
                say(f"status poll #{i + 1} TIMEOUT: {e}")
                continue
            if i == 0 or st.get("loaded") or st.get("error") or i % 5 == 0:
                say(
                    "status: loaded={loaded} is_loading={is_loading} device={device} "
                    "backend={backend} err={error}".format(
                        loaded=st.get("loaded"),
                        is_loading=st.get("is_loading"),
                        device=st.get("device"),
                        backend=st.get("backend"),
                        error=st.get("error"),
                    )
                )
            if st.get("error"):
                raise RuntimeError(f"load failed: {st['error']}")
            if st.get("loaded"):
                loaded = True
                break
            time.sleep(0.7)
        if not loaded:
            raise RuntimeError("model never reported loaded")

        say(f"start_stream -> {send({'cmd': 'start_stream', 'language': 'auto', 'vocabulary': ['Reflow']}, 90)}")

        # Binary frames, 1s each, exactly like Qwen3AsrSidecar::push_audio.
        chunk = 16000 * 2
        for off in range(0, len(pcm), chunk):
            piece = pcm[off:off + chunk]
            proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(piece)) + piece)
            proc.stdin.flush()
            time.sleep(0.05)
        say(f"pushed {len(pcm)} PCM bytes as binary frames")

        final = send({"cmd": "stop_stream"}, 180)
        say(f"stop_stream -> text={final.get('text')!r} lang={final.get('language')!r} "
            f"warning={final.get('warning')!r} status={final.get('status')!r}")
        text = (final.get("text") or "").lower()
        ok = bool(text.strip())
        say(f"RESULT: {'PASS' if ok else 'FAIL'}")
        return 0 if ok else 1
    finally:
        try:
            proc.stdin.close()
        except Exception:
            pass
        try:
            proc.wait(timeout=15)
        except Exception:
            proc.kill()
        say(f"sidecar returncode={proc.returncode}")


if __name__ == "__main__":
    sys.exit(main())
