#!/usr/bin/env python3
"""Measure whether Reflow's ASR model and polish LLM can be resident together.

Reflow wants both preloaded at launch. On a 4 GB card that is not obviously
possible, so this measures it instead of assuming:

  baseline -> ASR loaded -> llama-server loaded -> both exercised

reporting VRAM at each step, and confirming ASR still transcribes correctly
while llama-server is resident.

Usage: python scripts/diag_both.py [asr-model-id] [gguf] [gpu|cpu]
"""

import json
import os
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request
import wave

HERE = os.path.dirname(os.path.abspath(__file__))
SIDECAR = os.path.normpath(os.path.join(HERE, "..", "model-runtime", "qwen3_asr_runtime.py"))
APPDATA = os.environ.get("APPDATA", os.path.expanduser("~"))
ROOT = os.path.join(APPDATA, "reflow")
BIN = os.path.join(ROOT, "bin", "llama-server.exe")

T0 = time.time()


def say(msg):
    print(f"[{time.time() - T0:6.1f}s] {msg}", flush=True)


def vram():
    """(total, used, free) MiB, or None when nvidia-smi is unavailable."""
    try:
        out = subprocess.check_output(
            [
                "nvidia-smi",
                "--query-gpu=memory.total,memory.used,memory.free",
                "--format=csv,noheader,nounits",
            ],
            stderr=subprocess.DEVNULL,
        )
        total, used, free = (int(x.strip()) for x in out.decode().strip().split(","))
        return total, used, free
    except Exception:
        return None


def report(stage):
    v = vram()
    if v:
        total, used, free = v
        say(f"VRAM after {stage:<24} used={used:>5} MiB  free={free:>5} MiB  / {total} MiB")
    else:
        say(f"VRAM after {stage}: unavailable")
    return v


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


class Sidecar:
    def __init__(self):
        self.proc = subprocess.Popen(
            [sys.executable, "-u", SIDECAR],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.replies = {}
        self.cv = threading.Condition()
        self.dead = threading.Event()
        self.n = 0
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.proc.stdout:
            line = raw.decode("utf-8", "replace").strip()
            if not line:
                continue
            try:
                msg = json.loads(line)
            except Exception:
                continue
            with self.cv:
                self.replies[msg.get("id")] = msg
                self.cv.notify_all()
        self.dead.set()
        with self.cv:
            self.cv.notify_all()

    def send(self, payload, timeout):
        self.n += 1
        rid = self.n
        self.proc.stdin.write((json.dumps(dict(payload, id=rid)) + "\n").encode())
        self.proc.stdin.flush()
        deadline = time.time() + timeout
        with self.cv:
            while rid not in self.replies:
                if self.dead.is_set():
                    raise RuntimeError("sidecar died")
                if not self.cv.wait(max(0.05, deadline - time.time())):
                    if time.time() >= deadline:
                        raise TimeoutError(f"{payload['cmd']} timed out")
            return self.replies.pop(rid)

    def push_pcm(self, pcm):
        chunk = 16000 * 2
        for off in range(0, len(pcm), chunk):
            piece = pcm[off:off + chunk]
            self.proc.stdin.write(b"\x01" + struct.pack("<III", 0, 0, len(piece)) + piece)
            self.proc.stdin.flush()

    def close(self):
        try:
            self.proc.stdin.close()
        except Exception:
            pass
        try:
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()


def main():
    asr_id = sys.argv[1] if len(sys.argv) > 1 else "1.7b"
    gguf_name = sys.argv[2] if len(sys.argv) > 2 else "Qwen3.5-0.8B-Q4_K_M.gguf"
    mode = (sys.argv[3] if len(sys.argv) > 3 else "gpu").lower()

    asr_dir = os.path.join(ROOT, "models", f"qwen3-asr-{asr_id}")
    gguf = os.path.join(ROOT, "models", "flow", gguf_name)
    wav = os.path.join(HERE, "test_speech.wav")
    for path in (asr_dir, gguf, BIN, wav):
        if not os.path.exists(path):
            say(f"FAIL: missing {path}")
            return 1

    base = report("baseline")

    say(f"loading ASR {asr_id} int8 on cuda...")
    sc = Sidecar()
    sc.send({"cmd": "ping"}, 10)
    sc.send(
        {"cmd": "load_model", "model_dir": asr_dir, "device": "auto", "precision": "int8"},
        90,
    )
    deadline = time.time() + 240
    ready = False
    while time.time() < deadline:
        try:
            st = sc.send({"cmd": "status"}, 15)
        except TimeoutError:
            continue
        if st.get("error"):
            say(f"FAIL: ASR load error {st['error']}")
            sc.close()
            return 1
        if st.get("loaded"):
            say(
                f"ASR ready on {st.get('device')} ({st.get('backend')}) "
                f"self-reported vram={st.get('vram_mb')} MB"
            )
            ready = True
            break
        time.sleep(0.7)
    if not ready:
        say("FAIL: ASR never became ready")
        sc.close()
        return 1
    after_asr = report("ASR loaded")

    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    args = [
        BIN, "-m", gguf, "--host", "127.0.0.1", "--port", str(port),
        "--n-gpu-layers", "99" if mode == "gpu" else "0",
        "--ctx-size", "1024", "--parallel", "1", "--jinja",
        "--reasoning", "off",
    ]
    if mode == "gpu":
        args += ["--main-gpu", "0"]
    say(f"launching llama-server ({mode}) on port {port}...")
    llama = subprocess.Popen(
        args, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
    )
    tail = []

    def drain():
        for line in llama.stderr:
            tail.append(line.decode("utf-8", "replace").rstrip())
            if len(tail) > 60:
                tail.pop(0)

    threading.Thread(target=drain, daemon=True).start()

    healthy = False
    started = time.time()
    while time.time() - started < 120:
        if llama.poll() is not None:
            say(f"FAIL: llama-server exited rc={llama.returncode}")
            say("stderr tail:\n  " + "\n  ".join(tail[-25:]))
            sc.close()
            return 1
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    healthy = True
                    break
        except Exception:
            time.sleep(0.5)
    if not healthy:
        say("FAIL: llama-server never healthy")
        say("stderr tail:\n  " + "\n  ".join(tail[-25:]))
        llama.kill()
        sc.close()
        return 1
    say(f"llama-server healthy in {time.time() - started:.1f}s")
    after_llm = report("ASR + LLM loaded")

    body = {
        "model": "Qwen3.5-0.8B",
        "temperature": 0.0,
        "max_tokens": 96,
        "messages": [
            {
                "role": "system",
                "content": "Rewrite dictated speech as clean written text. Output only the text.",
            },
            {
                "role": "user",
                "content": "i think we should uh ship this on friday i mean thursday",
            },
        ],
    }
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    t = time.time()
    with urllib.request.urlopen(req, timeout=60) as r:
        payload = json.loads(r.read().decode())
    llm_ms = (time.time() - t) * 1000
    llm_out = payload["choices"][0]["message"]["content"]
    say(f"LLM completion in {llm_ms:.0f} ms -> {llm_out!r}")

    pcm = load_wav_16k_mono(wav)
    sc.send({"cmd": "start_stream", "language": "auto", "vocabulary": []}, 60)
    sc.push_pcm(pcm)
    t = time.time()
    final = sc.send({"cmd": "stop_stream"}, 180)
    asr_ms = (time.time() - t) * 1000
    asr_text = (final.get("text") or "").strip()
    say(f"ASR transcription in {asr_ms:.0f} ms -> {asr_text!r}")

    after_both = report("both exercised")

    llama.kill()
    try:
        llama.wait(timeout=10)
    except Exception:
        pass
    sc.close()

    ok = bool(asr_text) and bool(llm_out.strip())
    if base and after_asr and after_llm:
        say(
            f"ASR cost ~{after_asr[1] - base[1]} MiB, "
            f"LLM cost ~{after_llm[1] - after_asr[1]} MiB, "
            f"free with both resident: {after_llm[2]} MiB"
        )
        if after_both:
            say(f"free after exercising both: {after_both[2]} MiB")
    say(f"RESULT: {'PASS - both fit and both work' if ok else 'FAIL'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
