#!/usr/bin/env python3
"""
Reflow End-to-End System Smoke Test Matrix & Verification CLI.
Runs all subsystem verification gates across Python Runtime, Rust Core,
Audio/ASR Pipeline, SQLite Persistence, HTTP API, and Frontend Assets.
"""

import os
import subprocess
import sys
import time


def print_banner(text: str):
    print("\n" + "=" * 60)
    print(f"  {text}")
    print("=" * 60)


def run_gate(name: str, cmd_str: str, cwd: str = ".") -> tuple[bool, float, str]:
    print(f"--> Running {name}...", end="", flush=True)
    t0 = time.time()
    try:
        res = subprocess.run(
            cmd_str,
            cwd=cwd,
            capture_output=True,
            text=True,
            check=False,
            shell=True,
        )
        elapsed = time.time() - t0
        success = res.returncode == 0
        status = "PASSED" if success else "FAILED"
        print(f" [{status}] ({elapsed:.2f}s)")
        output = res.stdout if success else (res.stdout + "\n" + res.stderr)
        if not success:
            print(f"    [DETAIL] {output.strip()[:300]}")
        return success, elapsed, output.strip()
    except Exception as e:
        elapsed = time.time() - t0
        print(f" [ERROR] ({elapsed:.2f}s)")
        return False, elapsed, str(e)


def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    print_banner("Reflow End-to-End Verification Smoke Matrix")

    results = []

    # 1. Python ASR Runtime Self-Test
    p_ok, p_dur, p_out = run_gate(
        "Python ASR Runtime Self-Test (58 checks)",
        f'"{sys.executable}" model-runtime/qwen3_asr_runtime.py --selftest',
        cwd=root,
    )
    results.append(("Python Runtime Matrix", p_ok, p_dur))

    # 2. Rust Core & Integration Test Suite (267 tests)
    r_ok, r_dur, r_out = run_gate(
        "Rust Test Suite (267 unit & integration tests)",
        "cargo test",
        cwd=os.path.join(root, "src-tauri"),
    )
    results.append(("Rust Core & Tests", r_ok, r_dur))

    # 3. Rust Clippy Linters
    c_ok, c_dur, c_out = run_gate(
        "Rust Clippy & Linter Check",
        "cargo clippy --all-targets --message-format short -- -D warnings",
        cwd=os.path.join(root, "src-tauri"),
    )
    results.append(("Rust Clippy Lint", c_ok, c_dur))

    # 4. TypeScript Typecheck
    t_ok, t_dur, t_out = run_gate(
        "Frontend TypeScript Compilation (tsc --noEmit)",
        "npx tsc --noEmit",
        cwd=root,
    )
    results.append(("TypeScript Gates", t_ok, t_dur))

    # 5. ESLint Gate
    e_ok, e_dur, e_out = run_gate(
        "Frontend ESLint Rules",
        "npm run lint",
        cwd=root,
    )
    results.append(("ESLint Rules", e_ok, e_dur))

    # 6. Prettier Formatting
    pr_ok, pr_dur, pr_out = run_gate(
        "Frontend Prettier Style Check",
        "npm run format:check",
        cwd=root,
    )
    results.append(("Prettier Check", pr_ok, pr_dur))

    # 7. Vitest Unit Test Suite
    v_ok, v_dur, v_out = run_gate(
        "Frontend Vitest Suite (33 UI tests)",
        "npm run test",
        cwd=root,
    )
    results.append(("Vitest Suite", v_ok, v_dur))

    # Print Summary Matrix
    print_banner("Verification Results Summary")
    print(f"{'Gate Name':<35} | {'Status':<10} | {'Duration'}")
    print("-" * 60)
    all_passed = True
    for name, ok, dur in results:
        status_str = "PASS" if ok else "FAIL"
        if not ok:
            all_passed = False
        print(f"{name:<35} | {status_str:<10} | {dur:.2f}s")
    print("-" * 60)

    if all_passed:
        print("\nAll 7 System Verification Gates Passed Successfully!")
        sys.exit(0)
    else:
        print("\nSome Verification Gates Failed. Please check the logs above.")
        sys.exit(1)


if __name__ == "__main__":
    main()
