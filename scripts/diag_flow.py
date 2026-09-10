#!/usr/bin/env python3
"""Validate Reflow's Stage 2 (LLM polish) independently of the desktop app.

Launches %APPDATA%/reflow/bin/llama-server.exe with the same arguments
FlowRuntime::launch_on_port builds, waits for /health, then posts the same
OpenAI chat-completions request FlowClient::rewrite sends.

Usage: python scripts/diag_flow.py [cpu|gpu] [gguf-filename] [chat-model-name]
"""

import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

APPDATA = os.environ.get("APPDATA", os.path.expanduser("~"))
ROOT = os.path.join(APPDATA, "reflow")
BIN = os.path.join(ROOT, "bin", "llama-server.exe")

DEFAULT_GGUF = "Qwen3.5-0.8B-Q4_K_M.gguf"
DEFAULT_NAME = "Qwen3.5-0.8B"

GGUF = os.path.join(
    ROOT, "models", "flow", sys.argv[2] if len(sys.argv) > 2 else DEFAULT_GGUF
)
MODEL_NAME = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_NAME

T0 = time.time()


def say(msg):
    print(f"[{time.time() - T0:6.1f}s] {msg}", flush=True)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def main():
    mode = (sys.argv[1] if len(sys.argv) > 1 else "gpu").lower()
    n_gpu_layers = "99" if mode == "gpu" else "0"

    say(f"bin={BIN} exists={os.path.isfile(BIN)} size={os.path.getsize(BIN) if os.path.isfile(BIN) else 0}")
    say(f"gguf={GGUF} exists={os.path.isfile(GGUF)} size={os.path.getsize(GGUF) if os.path.isfile(GGUF) else 0}")
    if not os.path.isfile(BIN) or not os.path.isfile(GGUF):
        say("FAIL: binary or model missing")
        return 1

    port = free_port()
    args = [
        BIN, "-m", GGUF,
        "--host", "127.0.0.1",
        "--port", str(port),
        "--n-gpu-layers", n_gpu_layers,
        "--ctx-size", "1024",
        "--parallel", "1",
        "--jinja",
    ]
    if mode == "gpu":
        args += ["--main-gpu", "0"]
    extra = os.environ.get("REFLOW_EXTRA_ARGS", "").split()
    if extra:
        args += extra
    say(f"launching ({mode}, n_gpu_layers={n_gpu_layers}) on port {port} extra={extra}")

    proc = subprocess.Popen(
        args,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )

    stderr_tail = []

    import threading

    def drain():
        for raw in proc.stderr:
            line = raw.decode("utf-8", "replace").rstrip()
            stderr_tail.append(line)
            if len(stderr_tail) > 40:
                stderr_tail.pop(0)

    threading.Thread(target=drain, daemon=True).start()

    try:
        base = f"http://127.0.0.1:{port}"
        ready = False
        for _ in range(120):
            if proc.poll() is not None:
                say(f"FAIL: llama-server exited early rc={proc.returncode}")
                say("stderr tail:\n  " + "\n  ".join(stderr_tail[-25:]))
                return 1
            try:
                with urllib.request.urlopen(base + "/health", timeout=2) as r:
                    if r.status == 200:
                        ready = True
                        break
            except Exception:
                time.sleep(1.0)
        if not ready:
            say("FAIL: never became healthy")
            say("stderr tail:\n  " + "\n  ".join(stderr_tail[-25:]))
            return 1
        say("server healthy")

        transcript = "Please give me the key. I need the key, , for opening the door."
        body = {
            "model": MODEL_NAME,
            "temperature": 0.0,
            "top_p": 1.0,
            "repeat_penalty": 1.1,
            "max_tokens": 128,
            "stop": ["<|im_end|>", "<|endoftext|>", "\n\nTranscript:", "\nUser:"],
            "messages": [
                {
                    "role": "system",
                    "content": (
                        "You clean up dictated speech. Fix grammar, punctuation and "
                        "capitalization. Do not add or remove meaning. Reply with the "
                        "corrected text only."
                    ),
                },
                {"role": "user", "content": transcript},
            ],
        }
        req = urllib.request.Request(
            base + "/v1/chat/completions",
            data=json.dumps(body).encode(),
            headers={"Content-Type": "application/json"},
        )
        started = time.time()
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                payload = json.loads(r.read().decode())
        except urllib.error.HTTPError as e:
            say(f"FAIL: HTTP {e.code}: {e.read().decode()[:400]}")
            say("stderr tail:\n  " + "\n  ".join(stderr_tail[-25:]))
            return 1
        content = payload["choices"][0]["message"]["content"]
        reasoning = payload["choices"][0]["message"].get("reasoning_content")
        usage = payload.get("usage", {})
        say(f"completion in {time.time() - started:.2f}s  usage={usage}")
        say(f"  in : {transcript!r}")
        say(f"  out: {content!r}")
        if reasoning:
            say(f"  reasoning_content: {str(reasoning)[:200]!r}")
        finish = payload["choices"][0].get("finish_reason")
        say(f"  finish_reason={finish}")
        ok = bool((content or "").strip())
        say(f"RESULT: {'PASS' if ok else 'FAIL (empty completion)'}")
        return 0 if ok else 1
    finally:
        proc.kill()
        try:
            proc.wait(timeout=10)
        except Exception:
            pass
        say(f"llama-server rc={proc.returncode}")


if __name__ == "__main__":
    sys.exit(main())
