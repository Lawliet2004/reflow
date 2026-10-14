#!/usr/bin/env python3
"""Measure release model readiness and idle process-tree CPU/RSS on Linux.

Run: python scripts/measure_linux_desktop.py /path/to/reflow output.json
Uses --portable with an isolated, preconfigured reflow-data next to the binary.
The process is terminated after measurement; this measures forced-exit cleanup,
not the tray's graceful Quit action. No dictation or synthetic input is sent.
"""
import argparse
import json
import os
import pathlib
import signal
import statistics
import subprocess
import threading
import time


def process_snapshot():
    result = {}
    for path in pathlib.Path("/proc").iterdir():
        if not path.name.isdigit():
            continue
        try:
            fields = (path / "stat").read_text().rsplit(")", 1)[1].split()
            result[int(path.name)] = {
                "parent": int(fields[1]),
                "cpu": int(fields[11]) + int(fields[12]),
                "rss": int(fields[21]) * os.sysconf("SC_PAGE_SIZE"),
                "started": int(fields[19]),
            }
        except (OSError, ValueError, IndexError):
            continue
    return result


def descendants(root, snapshot):
    owned = {root}
    while True:
        next_owned = owned | {pid for pid, info in snapshot.items() if info["parent"] in owned}
        if owned == next_owned:
            return {pid: snapshot[pid] for pid in owned if pid in snapshot}
        owned = next_owned


def measure(binary, seconds=30):
    started = time.monotonic()
    process = subprocess.Popen([str(binary.resolve()), "--portable"], stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, start_new_session=True)
    model_ready = threading.Event()
    report = {"binary": str(binary.resolve()), "scope": "portable, tray-hidden, same selected model; CPU is percent of one logical core; RSS sums owned processes", "samples": []}
    logs = []
    def read():
        for line in process.stdout:
            logs.append(line)
            if "Model status: loaded=true" in line and not model_ready.is_set():
                report["model_ready_ms"] = (time.monotonic() - started) * 1000
                model_ready.set()
    reader = threading.Thread(target=read, daemon=True)
    reader.start()
    last_owned = {}
    try:
        if not model_ready.wait(timeout=90):
            raise RuntimeError("Model did not become ready; inspect measurement log")
        time.sleep(5)  # Separate startup probes/loading from idle.
        last_owned = descendants(process.pid, process_snapshot())
        previous_time = time.monotonic()
        for _ in range(seconds):
            time.sleep(1)
            if process.poll() is not None:
                raise RuntimeError("Application exited before measurement finished")
            now = time.monotonic()
            current = descendants(process.pid, process_snapshot())
            ticks = sum(info["cpu"] - last_owned[pid]["cpu"] for pid, info in current.items()
                if pid in last_owned and info["started"] == last_owned[pid]["started"])
            report["samples"].append({"cpu_pct": ticks / os.sysconf("SC_CLK_TCK") / (now - previous_time) * 100,
                "rss_mb": sum(info["rss"] for info in current.values()) / 1024 ** 2,
                "processes": len(current)})
            previous_time, last_owned = now, current
        report["idle_cpu_median_pct"] = statistics.median(row["cpu_pct"] for row in report["samples"])
        report["idle_rss_median_mb"] = statistics.median(row["rss_mb"] for row in report["samples"])
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        time.sleep(.5)
        snapshot = process_snapshot()
        survivors = [pid for pid, info in last_owned.items() if pid != process.pid and pid in snapshot and snapshot[pid]["started"] == info["started"]]
        report["owned_processes_surviving_termination"] = len(survivors)
        # Baseline lacks parent-death supervision. Reap only captured identities.
        for pid in survivors:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        reader.join(timeout=2)
        binary.with_suffix(".measurement.log").write_text("".join(logs), encoding="utf-8")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=pathlib.Path)
    parser.add_argument("output", type=pathlib.Path)
    parser.add_argument("--seconds", type=int, default=30)
    args = parser.parse_args()
    args.output.write_text(json.dumps(measure(args.binary, args.seconds), indent=2), encoding="utf-8")
