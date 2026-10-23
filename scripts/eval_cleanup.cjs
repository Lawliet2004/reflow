#!/usr/bin/env node
// Runs the cleanup golden corpus against the REAL local model and prints
// input / expected / actual / pass-fail for every case. Opt-in: it needs the
// installed Qwen3.5 GGUF and llama-server, never installs anything, and is not
// part of `cargo test`.
//
//   node scripts/eval_cleanup.cjs
//   REFLOW_EVAL_ONLY=self_correction node scripts/eval_cleanup.cjs
//   REFLOW_EVAL_MODEL=qwen3.5-2b REFLOW_EVAL_BACKEND=vulkan node scripts/eval_cleanup.cjs
//
// Exit code is 1 when the pass rate is below REFLOW_EVAL_MIN_PASS (default 1.0).
const { spawnSync } = require("node:child_process");
const path = require("node:path");

const manifest = path.join(__dirname, "..", "src-tauri", "Cargo.toml");
const result = spawnSync(
  "cargo",
  [
    "test",
    "--manifest-path",
    manifest,
    "--offline",
    "--test",
    "cleanup_eval_live",
    "--",
    "--ignored",
    "--nocapture",
    "--test-threads=1",
  ],
  { encoding: "utf8", env: process.env, maxBuffer: 64 * 1024 * 1024 },
);
const output = `${result.stdout || ""}\n${result.stderr || ""}`;
const start = output.indexOf("\n[");
console.log(start >= 0 ? output.slice(start) : output);

if (result.error || result.status !== 0) {
  console.error(result.error?.message || `Eval process exited with status ${result.status}.`);
  process.exit(2);
}

const summary = output.match(
  /EVAL_SUMMARY passed=(\d+) total=(\d+) rewrite_ms_median=(\d+) rewrite_ms_mean=(\d+)/,
);
if (!summary) {
  console.error("The eval did not finish (model or runtime missing?). See output above.");
  process.exit(2);
}
const [, passed, total, median, mean] = summary.map(Number);
const rate = total === 0 ? 0 : passed / total;
console.log(
  `\nPass rate ${passed}/${total} (${(rate * 100).toFixed(0)}%). Cleanup stage median ${median} ms, mean ${mean} ms.`,
);
const minimum = Number(process.env.REFLOW_EVAL_MIN_PASS ?? "1");
process.exit(rate >= minimum ? 0 : 1);
