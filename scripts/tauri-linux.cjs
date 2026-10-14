// Tauri action delegates here so repaired AppImage bytes are finalized before
// the existing release action uploads them. Local packaging uses package.cjs.
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const { buildPlan, collectArtifacts, finalizeAppImage, writeChecksums } = require("./package.cjs");
const root = path.resolve(__dirname, "..");
const args = process.argv.slice(2);
const result = spawnSync(
  process.execPath,
  [path.join(root, "node_modules/@tauri-apps/cli/tauri.js"), ...args],
  {
    cwd: root,
    stdio: "inherit",
    shell: false,
    env: process.env,
  },
);
if (result.error || result.status !== 0) process.exit(result.status || 1);
if (args[0] === "build") {
  const config = JSON.parse(fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"));
  const plan = buildPlan({ platform: "linux", root });
  const artifacts = collectArtifacts(plan, { version: config.version });
  finalizeAppImage(plan, artifacts, config);
  writeChecksums(artifacts);
}
