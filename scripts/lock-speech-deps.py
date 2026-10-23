#!/usr/bin/env python3
"""Regenerate CPython 3.12 binary dependency locks using uv (maintenance only).

The installer consumes these checked-in hashes without invoking uv or resolving
floating transitives. Review lock changes and run clean installation acceptance
before shipping. See https://docs.astral.sh/uv/reference/cli/ for target semantics.
"""
import concurrent.futures
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
LOCKS = ROOT / "model-runtime" / "locks"
SHARED = "numpy==2.4.4\nhuggingface_hub==1.11.0\nhttpx==0.28.1\n"
PROFILES = {
    "zipformer": SHARED + (ROOT / "model-runtime/requirements-zipformer.txt").read_text(),
    "phonon": SHARED + (ROOT / "model-runtime/requirements-phonon.txt").read_text()
        + "\ntorch==2.13.0\ntransformers==5.15.1\n",
    "qwen": "\n".join(line for line in (ROOT / "model-runtime/requirements.txt").read_text().splitlines()
                        if not line.startswith("-r ")) + "\nhttpx==0.28.1\n",
}
TARGETS = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu",
           "x86_64-pc-windows-msvc", "aarch64-apple-darwin", "x86_64-apple-darwin"]


def generate(job):
    target, profile, variant = job
    args = ["uv", "pip", "compile", str(LOCKS / f"{profile}.in"), "--python-version", "3.12",
            "--python-platform", target, "--generate-hashes", "--only-binary", ":all:",
            "--no-header", "--no-annotate", "--default-index", "https://pypi.org/simple",
            "-o", str(LOCKS / f"{target}-{profile}-{variant}.txt")]
    if variant not in ("none", "default"):
        args += ["--index", f"https://download.pytorch.org/whl/{variant}",
                 "--index-strategy", "unsafe-best-match"]
    env = {k: v for k, v in os.environ.items() if not k.startswith(("UV_", "PIP_"))}
    if "darwin" in target:
        env["MACOSX_DEPLOYMENT_TARGET"] = "14.0" if target.startswith("aarch64") else "10.15"
    result = subprocess.run(args, env=env, capture_output=True, text=True, timeout=600)
    log = ROOT / f"docs/audit-2026-10-22/verification/lock-{target}-{profile}-{variant}.log"
    log.write_text(result.stderr)
    print(f"{target}/{profile}/{variant}: {result.returncode}", flush=True)
    return result.returncode


def main():
    for profile, text in PROFILES.items():
        (LOCKS / f"{profile}.in").write_text(text)
    jobs = []
    for target in TARGETS:
        for profile in PROFILES:
            if "--phonon-only" in sys.argv and profile != "phonon":
                continue
            if "--qwen-only" in sys.argv and profile != "qwen":
                continue
            if target == "x86_64-apple-darwin" and profile != "zipformer":
                continue
            variants = (["none"] if profile == "zipformer" else ["default"] if "darwin" in target
                        else ["cpu", "cu130", "cu126"] if target.startswith("x86_64") else ["cpu", "cu130"])
            jobs.extend((target, profile, variant) for variant in variants)
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        results = list(pool.map(generate, jobs))
        return int(any(results))


if __name__ == "__main__":
    sys.exit(main())
