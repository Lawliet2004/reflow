"""Serial, local calibration of installed runtimes (no downloads).

Usage: python scripts/benchmark_runtime.py corpus.json output.json
The corpus uses the same language/audio/reference schema as --benchmark.
ASR timings include final decoding, not model loading or microphone duration.
LLM measurements are isolated, with ASR unloaded, and are diagnostic only.
"""
import base64
import json
import os
import pathlib
import queue
import re
import socket
import statistics
import subprocess
import sys
import threading
import time
import urllib.request
import wave
import math
import unicodedata

def error_rates(reference, hypothesis):
    def normalized(text):
        return ''.join(c if c.isalnum() or c in '+-' or unicodedata.category(c).startswith('M') else ' ' for c in text.casefold().replace('−', '-'))
    a, b = normalized(reference), normalized(hypothesis)
    def distance(a, b):
        row = list(range(len(b) + 1))
        for i, item in enumerate(a, 1):
            next_row = [i]
            for j, other in enumerate(b, 1):
                next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (item != other)))
            row = next_row
        return row[-1]
    words_a, words_b = a.split(), b.split()
    chars_a, chars_b = ''.join(a.split()), ''.join(b.split())
    if not words_a or not chars_a or not any(c.isalnum() for c in reference): raise ValueError('A labelled reference is required')
    return distance(words_a, words_b) / len(words_a), distance(chars_a, chars_b) / len(chars_a)

def median_p95(values):
    if not values or any(not math.isfinite(value) or value < 0 for value in values): raise ValueError('Invalid timings')
    ordered = sorted(values)
    return statistics.median(ordered), ordered[math.ceil(len(ordered) * .95) - 1]

def select_candidate(rows):
    def error(row): return row['cer'] if row.get('character_language') else row['wer']
    def eligible(row):
        value = error(row)
        median, p95 = row['median_ms'], row['p95_ms']
        return row.get('labelled', False) and row.get('repeats', 0) >= 3 and not row.get('spill_detected', False) and all(math.isfinite(number) for number in (value, median, p95)) and 0 <= value <= (.03 if row.get('character_language') else .05) and 0 < median <= p95 <= median * 1.35 + 30
    admitted = [row for row in rows if eligible(row)]
    if not admitted: return None
    best = min(map(error, admitted))
    return min((row for row in admitted if error(row) <= best + .005), key=lambda row: row['median_ms'])

ROOT = pathlib.Path(__file__).resolve().parents[1]
APP = pathlib.Path(os.environ.get("APPDATA", pathlib.Path.home())) / "reflow"


def wer(reference, hypothesis):
    a, b = [re.findall(r"\w+", s.casefold()) for s in (reference, hypothesis)]
    row = list(range(len(b) + 1))
    for i, word in enumerate(a, 1):
        next_row = [i]
        for j, other in enumerate(b, 1):
            next_row.append(min(next_row[-1] + 1, row[j] + 1, row[j - 1] + (word != other)))
        row = next_row
    return row[-1] / max(1, len(a))


class Sidecar:
    def __init__(self):
        self.log = open(ROOT / "docs" / "audit-calibration-sidecar.log", "w", encoding="utf-8")
        self.proc = subprocess.Popen([sys.executable, str(ROOT / "model-runtime" / "qwen3_asr_runtime.py")],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.log,
            text=True, encoding="utf-8", bufsize=1)
        self.lines = queue.Queue()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.proc.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def send(self, **payload):
        self.proc.stdin.write(json.dumps(payload) + "\n")
        self.proc.stdin.flush()
        line = self.lines.get(timeout=240)
        if line is None:
            raise RuntimeError("Sidecar exited")
        value = json.loads(line)
        if value.get("error"):
            raise RuntimeError(value["error"])
        return value

    def close(self):
        try:
            self.proc.terminate()
            self.proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.log.close()


def asr_rows(corpus, root):
    sidecar = Sidecar()
    rows = []
    try:
        sidecar.send(cmd="ping")
        for precision in ("bf16", "int8"):
            item = {"runtime": "python", "model": "0.6b", "requested_precision": precision}
            print(f"Calibrating ASR {precision}", flush=True)
            try:
                started = time.monotonic()
                sidecar.send(cmd="load_model", model_dir=str(APP / "models" / "qwen3-asr-0.6b"),
                    device="cuda", precision=precision)
                deadline = time.monotonic() + 240
                while time.monotonic() < deadline:
                    status = sidecar.send(cmd="status")
                    if status.get("loaded"):
                        break
                    time.sleep(0.3)
                else:
                    raise TimeoutError("Model load did not settle")
                item.update(load_wall_seconds=time.monotonic() - started,
                    device=status.get("device"), precision=status.get("precision"),
                    vram_mb=status.get("vram_mb"), spill_detected=status.get("spill_detected"))
                runs = []
                for sample in corpus:
                    with wave.open(str(root / sample["audio"]), "rb") as wav:
                        if (wav.getframerate(), wav.getnchannels(), wav.getsampwidth()) != (16000, 1, 2):
                            raise ValueError("Corpus requires 16kHz mono PCM16")
                        frames = wav.readframes(wav.getnframes())
                    duration = len(frames) / 32000
                    for repetition in range(3):
                        sidecar.send(cmd="start_stream", language=sample["language"], vocabulary=["Reflow"])
                        for i in range(0, len(frames), 32000):
                            sidecar.send(cmd="push_audio_b64", audio_b64=base64.b64encode(frames[i:i+32000]).decode())
                        started = time.monotonic()
                        result = sidecar.send(cmd="stop_stream")
                        elapsed = time.monotonic() - started
                        runs.append({"language": sample["language"], "repetition": repetition,
                            "audio_seconds": duration, "decode_ms": elapsed * 1000, "rtf": elapsed / duration,
                            "wer": wer(sample["reference"], result.get("text", "")),
                            "warning": result.get("warning")})
                item.update(runs=runs, median_decode_ms=statistics.median(r["decode_ms"] for r in runs),
                    median_rtf=statistics.median(r["rtf"] for r in runs))
            except Exception as exc:
                item["error"] = str(exc)
            rows.append(item)
            sidecar.send(cmd="unload_model")
    finally:
        sidecar.close()
    return rows


def llm_rows():
    rows = []
    binary = APP / "bin" / "llama-server.exe"
    model = APP / "models" / "flow" / "Qwen3.5-0.8B-Q4_K_M.gguf"
    if not binary.is_file() or not model.is_file():
        return [{"error": "Refinement runtime or weights are missing"}]
    for device, layers in (("cpu", 0), ("gpu", 99)):
        print(f"Calibrating isolated LLM {device}", flush=True)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        log = open(ROOT / "docs" / f"audit-calibration-llm-{device}.log", "w", encoding="utf-8")
        proc = subprocess.Popen([str(binary), "-m", str(model), "--host", "127.0.0.1",
            "--port", str(port), "--n-gpu-layers", str(layers), "--ctx-size", "1024",
            "--parallel", "1", "--threads", "6", "--jinja", "--reasoning-budget", "0"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
        item = {"runtime": "llama-server", "model": "qwen3.5-0.8b", "requested_device": device,
            "gpu_layers": layers, "threads": 6, "context_size": 1024, "asr_resident": False}
        base = f"http://127.0.0.1:{port}"
        started = time.monotonic()
        try:
            deadline = started + 90
            while time.monotonic() < deadline:
                if proc.poll() is not None:
                    raise RuntimeError(f"llama-server exited with {proc.returncode}; see calibration log")
                try:
                    with urllib.request.urlopen(base + "/health", timeout=1) as response:
                        if response.status == 200:
                            break
                except Exception:
                    time.sleep(0.2)
            else:
                raise TimeoutError("llama-server readiness timeout")
            item["load_seconds"] = time.monotonic() - started
            runs = []
            for repetition in range(3):
                body = {"model": "qwen3.5-0.8b", "temperature": 0, "max_tokens": 128,
                    "chat_template_kwargs": {"enable_thinking": False},
                    "messages": [{"role": "system", "content": "Clean dictated speech. Fix punctuation and capitalization. Preserve meaning. Reply with the corrected text only."},
                        {"role": "user", "content": "we should ship this on friday i mean thursday please tell alex about the change"}]}
                request = urllib.request.Request(base + "/v1/chat/completions", data=json.dumps(body).encode(),
                    headers={"Content-Type": "application/json"})
                started = time.monotonic()
                with urllib.request.urlopen(request, timeout=30) as response:
                    payload = json.load(response)
                elapsed = time.monotonic() - started
                choice = payload["choices"][0]
                tokens = payload.get("usage", {}).get("completion_tokens", 0)
                runs.append({"repetition": repetition, "completion_ms": elapsed * 1000,
                    "output_tokens": tokens, "output_tokens_per_wall_second": tokens / elapsed,
                    "finish_reason": choice.get("finish_reason"), "nonempty": bool(choice["message"].get("content")),
                    "server_timings": payload.get("timings")})
            item.update(runs=runs, median_completion_ms=statistics.median(r["completion_ms"] for r in runs))
        except Exception as exc:
            item["error"] = str(exc)
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
            log.close()
        rows.append(item)
    return rows


if __name__ == "__main__":
    corpus_path, output_path = map(pathlib.Path, sys.argv[1:3])
    corpus = json.loads(corpus_path.read_text(encoding="utf-8-sig"))
    report = {"scope": "One installed English corpus; three warm repetitions per ASR precision and isolated LLM device. No native weights installed; no multilingual accuracy conclusion.",
        "asr": asr_rows(corpus, corpus_path.parent), "llm": llm_rows()}
    output_path.write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(f"Saved calibration to {output_path}", flush=True)
