#!/usr/bin/env node
/**
 * Quits any running Reflow instance so `tauri dev` / `tauri build` can
 * replace the executable and own the global hotkey. Runs as the `pretauri`
 * npm hook. Always exits 0 — a stale instance is not an error.
 */
const { execSync } = require("child_process");

try {
  if (process.platform === "win32") {
    execSync("taskkill /IM reflow.exe /F", { stdio: "ignore" });
    console.log("Reflow: stopped the running instance (dev/build takes over).");
  } else {
    execSync("pkill -f reflow || true", { stdio: "ignore" });
  }
} catch {
  // not running — nothing to do
}
