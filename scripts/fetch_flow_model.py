#!/usr/bin/env python3
"""Fetch a Reflow "flow" (polish) GGUF into the directory the app reads.

Downloads into %APPDATA%/reflow/models/flow/, which is what
`FlowRuntime::flow_gguf_path` resolves to, so the app picks the file up with no
further wiring.

Usage:
  python scripts/fetch_flow_model.py [repo] [filename]

Defaults to unsloth/Qwen3.5-0.8B-GGUF / Qwen3.5-0.8B-Q4_K_M.gguf.
"""

import os
import sys
import time

DEFAULT_REPO = "unsloth/Qwen3.5-0.8B-GGUF"
DEFAULT_FILE = "Qwen3.5-0.8B-Q4_K_M.gguf"


def main():
    repo = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_REPO
    filename = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_FILE

    appdata = os.environ.get("APPDATA", os.path.expanduser("~"))
    dest_dir = os.path.join(appdata, "reflow", "models", "flow")
    os.makedirs(dest_dir, exist_ok=True)
    dest = os.path.join(dest_dir, filename)

    if os.path.isfile(dest):
        print(f"already present: {dest} ({os.path.getsize(dest)} bytes)", flush=True)
        return 0

    print(f"repo     : {repo}", flush=True)
    print(f"file     : {filename}", flush=True)
    print(f"dest dir : {dest_dir}", flush=True)

    from huggingface_hub import hf_hub_download

    started = time.time()
    path = hf_hub_download(
        repo_id=repo,
        filename=filename,
        local_dir=dest_dir,
    )
    size = os.path.getsize(path)
    elapsed = time.time() - started
    print(
        f"downloaded {size} bytes ({size / 1024 / 1024:.1f} MB) in {elapsed:.1f}s "
        f"-> {path}",
        flush=True,
    )
    # The app expects the bare filename directly inside models/flow.
    if os.path.abspath(path) != os.path.abspath(dest):
        print(f"WARNING: landed at {path}, expected {dest}", flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
