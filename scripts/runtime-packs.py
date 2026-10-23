#!/usr/bin/env python3
"""Assemble native, model-free speech runtime packs from reviewed binary locks.

Build on the target OS/architecture. Only build preparation uses package indexes;
the install probe denies sockets and installs entirely from the wheelhouse.
No populated venv is archived or relocated. See docs/runtime-packs.md.
"""
import argparse
from email.parser import BytesParser
import gzip
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from datetime import datetime, timezone
from urllib.parse import urlsplit
from urllib.request import urlopen
import zipfile

ROOT = Path(__file__).resolve().parents[1]
LOCKS = ROOT / "model-runtime/locks"
BUILDS = json.loads((ROOT / "model-runtime/python-builds.json").read_text())
LICENSE_DIR = ROOT / "model-runtime/licenses"
LICENSE_SUPPLEMENTS = json.loads((LICENSE_DIR / "supplements.json").read_text())["packages"]
RELEASE_TARGETS = {"x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu",
                   "aarch64-apple-darwin", "x86_64-apple-darwin"}
RUNNERS = {"x86_64-pc-windows-msvc": "windows-latest", "x86_64-unknown-linux-gnu": "ubuntu-22.04",
           "aarch64-apple-darwin": "macos-latest", "x86_64-apple-darwin": "macos-15-intel"}
DENY_NETWORK = """
import socket
def denied(*args, **kwargs):
    raise RuntimeError('Network is disabled during the runtime-pack install probe')
socket.socket.connect = denied
socket.create_connection = denied
"""


def digest(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def host_target():
    arch = {"amd64": "x86_64", "arm64": "aarch64"}.get(platform.machine().lower(),
                                                         platform.machine().lower())
    suffix = {"Linux": "unknown-linux-gnu", "Windows": "pc-windows-msvc",
              "Darwin": "apple-darwin"}.get(platform.system(), "unsupported")
    return f"{arch}-{suffix}"


def python_path(folder):
    return folder / ("Scripts/python.exe" if os.name == "nt" else "bin/python")


def clean_environment():
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("PIP_", "PYTHON", "UV_")) and k != "VIRTUAL_ENV"}
    env.update(PIP_CONFIG_FILE=os.devnull, PYTHONNOUSERSITE="1", HF_HUB_OFFLINE="1",
               TRANSFORMERS_OFFLINE="1", PIP_DISABLE_PIP_VERSION_CHECK="1")
    return env


def run(python, args, offline=False):
    if offline:
        if args[:2] == ["-m", "pip"]:
            args = ["-c", DENY_NETWORK + "\nimport runpy, sys\nsys.argv = ['pip'] + sys.argv[1:]\n"
                    "runpy.run_module('pip', run_name='__main__')", *args[2:]]
        elif args[:1] == ["-c"]:
            args = ["-c", DENY_NETWORK + args[1], *args[2:]]
    subprocess.run([str(python), *map(str, args)], check=True, env=clean_environment(), timeout=1800)


def fetch_python(pin, cache):
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / pin["asset"]
    if archive.is_file() and archive.stat().st_size == pin["approx_bytes"] and digest(archive) == pin["sha256"]:
        return archive
    partial = archive.with_suffix(archive.suffix + ".partial")
    try:
        with urlopen(pin["url"], timeout=30) as response, open(partial, "wb") as output:
            url = urlsplit(response.url)
            if url.scheme != "https" or url.hostname not in (
                    "github.com", "release-assets.githubusercontent.com", "objects.githubusercontent.com"):
                raise ValueError("Unexpected Python archive redirect")
            received = 0
            while chunk := response.read(1024 * 1024):
                received += len(chunk)
                if received > pin["approx_bytes"]:
                    raise ValueError("Python archive exceeds pinned size")
                output.write(chunk)
        if partial.stat().st_size != pin["approx_bytes"] or digest(partial) != pin["sha256"]:
            raise ValueError("Python archive does not match its pin")
        partial.replace(archive)
        return archive
    finally:
        partial.unlink(missing_ok=True)


def license_inventory(wheelhouse):
    inventory = []
    for wheel in sorted(wheelhouse.glob("*.whl")):
        with zipfile.ZipFile(wheel) as archive:
            # Vendored distributions can carry nested .dist-info directories.
            # The wheel's own metadata lives at the archive root; retain the
            # nested license files below, but do not mistake them for its identity.
            metadata_files = [n for n in archive.namelist()
                              if n.count("/") == 1 and n.endswith(".dist-info/METADATA")]
            if len(metadata_files) != 1:
                raise ValueError(f"Missing or ambiguous metadata: {wheel.name}")
            metadata = BytesParser().parsebytes(archive.read(metadata_files[0]))
            texts = []
            for entry in archive.infolist():
                name = Path(entry.filename).name.lower()
                if not entry.is_dir() and name.startswith(("license", "licence", "copying", "notice")):
                    if entry.file_size > 8 * 1024 * 1024:
                        raise ValueError(f"License file needs manual packaging: {wheel.name}/{entry.filename}")
                    texts.append({"path": entry.filename,
                                  "text": archive.read(entry).decode("utf-8", errors="replace")})
            license_name = metadata.get("License-Expression") or metadata.get("License")
            if not texts and (not license_name or license_name.strip().upper() == "UNKNOWN"):
                supplement = LICENSE_SUPPLEMENTS.get(f"{metadata['Name'].lower()}=={metadata['Version']}")
                if supplement is None:
                    raise ValueError(f"License information needs review: {wheel.name}")
                notice = LICENSE_DIR / supplement["file"]
                if digest(notice) != supplement["sha256"]:
                    raise ValueError(f"Supplemental license hash changed: {wheel.name}")
                license_name = supplement["license"]
                texts.append({"path": supplement["file"], "source": supplement["source"],
                              "sha256": supplement["sha256"], "origin": "upstream-source",
                              "text": notice.read_text(encoding="utf-8")})
            inventory.append({"wheel": wheel.name, "name": metadata["Name"], "version": metadata["Version"],
                              "license": license_name, "license_files": texts})
    if not inventory:
        raise ValueError("Empty wheelhouse")
    return inventory


def check_code(profile):
    shared = "import numpy, huggingface_hub, httpx\n"
    if profile == "zipformer":
        return shared + "import sherpa_onnx\nprint('Offline Zipformer imports passed')\n"
    engine = ("import fermion, scipy, soundfile, safetensors, zstandard\nfrom fermion._speech import engine_phonon2_cpu\nfrom transformers import ParakeetForTDT, ParakeetTDTConfig\n" if profile == "phonon" else
              "from transformers import AutoProcessor, AutoModelForMultimodalLM, Qwen3ASRProcessor, Qwen3ASRForConditionalGeneration\n")
    return shared + "import torch, transformers\n" + engine + "assert torch.ones(1).add(1).item() == 2\nprint('Offline CPU tensor/import checks passed')\n"


def build(output, target, profile, variant, cache, commit):
    if host_target() != target:
        raise ValueError(f"Build {target} on its native host; this host is {host_target()}")
    lock = LOCKS / f"{target}-{profile}-{variant}.txt"
    if not lock.is_file():
        raise ValueError("No reviewed lock for this target/profile/variant")
    pin = next(p for p in BUILDS["pins"] if p["target"] == target)
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="reflow-pack-", dir=output) as working:
        work = Path(working)
        payload = work / "payload"
        wheels = payload / "wheelhouse"
        wheels.mkdir(parents=True)
        (payload / "bootstrap").mkdir()
        bootstrap = payload / "bootstrap" / pin["asset"]
        shutil.copyfile(fetch_python(pin, cache), bootstrap)
        shutil.copyfile(lock, payload / "requirements.txt")
        shutil.copyfile(LOCKS / "pip.txt", payload / "pip-requirements.txt")
        base = work / "base"
        with tarfile.open(bootstrap) as archive:
            archive.extractall(base, filter="data")
        base_python = base / "python" / ("python.exe" if os.name == "nt" else "bin/python3")
        run(base_python, ["-m", "venv", work / "probe"])
        python = python_path(work / "probe")
        # Use only pinned binary artifacts. The build host's site packages and
        # pip configuration cannot satisfy or redirect a dependency.
        download = ["-m", "pip", "download", "--require-hashes", "--only-binary=:all:",
                    "--dest", wheels, "--index-url", "https://pypi.org/simple", "--timeout", "120", "--retries", "8"]
        run(python, [*download, "-r", payload / "pip-requirements.txt"])
        local = ["-m", "pip", "install", "--no-index", "--find-links", wheels,
                 "--require-hashes", "--only-binary=:all:", "--no-input"]
        run(python, [*local, "--upgrade", "-r", payload / "pip-requirements.txt"], offline=True)
        if variant in ("cpu", "cu126", "cu130"):
            download += ["--extra-index-url", f"https://download.pytorch.org/whl/{variant}"]
        run(python, [*download, "-r", payload / "requirements.txt"])
        run(python, [*local, "-r", payload / "requirements.txt"], offline=True)
        run(python, ["-m", "pip", "check"], offline=True)
        run(python, ["-c", check_code(profile)], offline=True)
        inventory = license_inventory(wheels)
        (payload / "licenses.json").write_text(json.dumps({"schema": 1, "packages": inventory}, indent=2) + "\n")
        (payload / "build-check.json").write_text(json.dumps({
            "schema": 1, "target": target, "source_commit": commit,
            "tested_at": datetime.now(timezone.utc).isoformat(),
            "offline_wheel_install": "pass", "pip_check": "pass", "cpu_imports": "pass",
            "accelerator_execution": "requires_native_acceptance",
            "note": "No model weights; this probe does not prove ASR or desktop permission behavior."
        }, indent=2) + "\n")
        files = [{"path": f.relative_to(payload).as_posix(), "bytes": f.stat().st_size, "sha256": digest(f)}
                 for f in sorted(payload.rglob("*")) if f.is_file()]
        manifest = {"schema": 1, "target": target, "profile": profile, "variant": variant,
                    "requirements_sha256": digest(lock), "pip_lock_sha256": digest(LOCKS / "pip.txt"),
                    "pbs_sha256": pin["sha256"], "files": files}
        encoded = (json.dumps(manifest, indent=2) + "\n").encode()
        (payload / "manifest.json").write_bytes(encoded)
        asset = f"speech-runtime-{target}-{profile}-{variant}.tar.gz"
        temporary = output / (asset + ".partial")
        try:
            with open(temporary, "wb") as stream, gzip.GzipFile(filename="", fileobj=stream, mode="wb", mtime=0, compresslevel=1) as zipped:
                with tarfile.open(fileobj=zipped, mode="w") as archive:
                    for name in ["manifest.json", *[f["path"] for f in files]]:
                        info = archive.gettarinfo(payload / name, arcname=name)
                        info.uid = info.gid = info.mtime = 0
                        info.uname = info.gname = ""
                        info.mode = 0o600
                        with open(payload / name, "rb") as stream_in:
                            archive.addfile(info, stream_in)
            temporary.replace(output / asset)
        finally:
            temporary.unlink(missing_ok=True)
        descriptor = {k: manifest[k] for k in ("target", "profile", "variant", "requirements_sha256", "pip_lock_sha256", "pbs_sha256")}
        descriptor.update(schema=1, asset=asset, sha256=digest(output / asset), bytes=(output / asset).stat().st_size,
                          unpacked_bytes=sum(f["bytes"] for f in files) + len(encoded), source_commit=commit)
        (output / asset.replace(".tar.gz", ".json")).write_text(json.dumps(descriptor, indent=2) + "\n")
        print(json.dumps(descriptor), flush=True)


def catalog(directory, release, repository, commit, output, partial=False, metadata_only=False):
    if not re.fullmatch(r"v\d+\.\d+\.\d+", release) or not re.fullmatch(r"[a-zA-Z0-9_.-]+/[a-zA-Z0-9_.-]+", repository):
        raise ValueError("Invalid release or repository")
    if not re.fullmatch(r"[a-f0-9]{40}", commit):
        raise ValueError("The pack catalog requires an exact source commit")
    expected = {p.stem for p in LOCKS.glob("*.txt") if any(p.name.startswith(t + "-") for t in RELEASE_TARGETS)}
    packs = []
    keys = set()
    for filename in sorted(directory.glob("speech-runtime-*.json")):
        pin = json.loads(filename.read_text())
        key = f"{pin['target']}-{pin['profile']}-{pin['variant']}"
        if key not in expected or key in keys or pin.get("schema") != 1 or pin.get("source_commit") != commit:
            raise ValueError(f"Unexpected, duplicate or stale pack: {filename.name}")
        asset = f"speech-runtime-{key}.tar.gz"
        python = next(p for p in BUILDS["pins"] if p["target"] == pin["target"])
        if (pin["asset"] != asset or not re.fullmatch(r"[a-f0-9]{64}", pin.get("sha256", ""))
                or type(pin.get("bytes")) is not int or not 0 < pin["bytes"] <= 16 * 1024**3
                or type(pin.get("unpacked_bytes")) is not int or not 0 < pin["unpacked_bytes"] <= 24 * 1024**3
                or pin["requirements_sha256"] != digest(LOCKS / f"{key}.txt")
                or pin["pip_lock_sha256"] != digest(LOCKS / "pip.txt")
                or pin["pbs_sha256"] != python["sha256"]):
            raise ValueError(f"Pack differs from its reviewed inputs: {asset}")
        if not metadata_only and (pin["sha256"] != digest(directory / asset)
                                  or pin["bytes"] != (directory / asset).stat().st_size):
            raise ValueError(f"Pack archive differs from its build: {asset}")
        pin["url"] = f"https://github.com/{repository}/releases/download/{release}/{asset}"
        packs.append(pin)
        keys.add(key)
    if not packs or (not partial and keys != expected):
        raise ValueError(f"Missing runtime packs: {sorted(expected - keys)}")
    output.write_text(json.dumps({"schema": 1, "release": release, "source_commit": commit, "packs": packs}, indent=2) + "\n")


def matrix():
    rows = []
    for target in sorted(RELEASE_TARGETS):
        for lock in sorted(LOCKS.glob(f"{target}-*.txt")):
            profile, variant = lock.stem[len(target) + 1:].rsplit("-", 1)
            rows.append({"target": target, "profile": profile, "variant": variant, "platform": RUNNERS[target]})
    return {"include": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("matrix")
    build_parser = sub.add_parser("build")
    build_parser.add_argument("target")
    build_parser.add_argument("profile", choices=["zipformer", "qwen", "phonon"])
    build_parser.add_argument("variant", choices=["none", "default", "cpu", "cu126", "cu130"])
    build_parser.add_argument("--output", type=Path, required=True)
    build_parser.add_argument("--cache", type=Path, required=True)
    build_parser.add_argument("--commit", default=os.environ.get("GITHUB_SHA"))
    catalog_parser = sub.add_parser("catalog")
    catalog_parser.add_argument("directory", type=Path)
    catalog_parser.add_argument("release")
    catalog_parser.add_argument("repository")
    catalog_parser.add_argument("commit")
    catalog_parser.add_argument("output", type=Path)
    catalog_parser.add_argument("--allow-partial", action="store_true", help="Local tests only; never use for a release")
    catalog_parser.add_argument("--metadata-only", action="store_true", help="Assemble from this workflow's trusted build descriptors; publication must rehash the archives")
    args = parser.parse_args()
    if args.command == "build":
        build(args.output.resolve(), args.target, args.profile, args.variant, args.cache.resolve(), args.commit)
    elif args.command == "catalog":
        catalog(args.directory, args.release, args.repository, args.commit, args.output, args.allow_partial, args.metadata_only)
    else:
        print(json.dumps(matrix(), separators=(",", ":")))


if __name__ == "__main__":
    main()
