#!/usr/bin/env node
// The npm pretauri hook only stops binaries built by this checkout.
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

function allowedExecutablePaths(root, platform) {
  const executable = platform === "win32" ? "reflow.exe" : "reflow";
  return ["debug", "release"].map((profile) =>
    path.resolve(root, "src-tauri", "target", profile, executable),
  );
}

function selectOwnedProcesses(processes, root, platform, ownPid = process.pid) {
  const normalize = (value) =>
    platform === "win32" ? path.resolve(value).toLowerCase() : path.resolve(value);
  const allowed = new Set(allowedExecutablePaths(root, platform).map(normalize));
  return processes.filter(
    ({ pid, executable }) =>
      Number.isInteger(pid) &&
      pid > 0 &&
      pid !== ownPid &&
      typeof executable === "string" &&
      allowed.has(normalize(executable)),
  );
}

function listProcesses(platform) {
  if (platform === "win32") {
    const query =
      "Get-CimInstance Win32_Process -Filter \"Name = 'reflow.exe'\" | Select-Object ProcessId, ExecutablePath | ConvertTo-Json -Compress";
    const result = execFileSync(
      "powershell.exe",
      ["-NoProfile", "-NonInteractive", "-Command", query],
      { encoding: "utf8", windowsHide: true },
    );
    const parsed = result.trim() ? JSON.parse(result) : [];
    return (Array.isArray(parsed) ? parsed : [parsed])
      .filter(Boolean)
      .map((row) => ({ pid: row.ProcessId, executable: row.ExecutablePath }));
  }
  if (platform === "linux") {
    return fs
      .readdirSync("/proc")
      .filter((entry) => /^[0-9]+$/.test(entry))
      .flatMap((entry) => {
        try {
          return [{ pid: Number(entry), executable: fs.readlinkSync(`/proc/${entry}/exe`) }];
        } catch {
          return [];
        }
      });
  }
  if (platform === "darwin") {
    return execFileSync("ps", ["-axo", "pid=,comm="], { encoding: "utf8" })
      .split("\n")
      .flatMap((line) => {
        const match = line.match(/^\s*([0-9]+)\s+(.+)$/);
        return match ? [{ pid: Number(match[1]), executable: match[2].trim() }] : [];
      });
  }
  return [];
}

function stopOwnedInstances({
  root = path.resolve(__dirname, ".."),
  platform = process.platform,
  inventory = listProcesses,
  terminate = (pid) => process.kill(pid, "SIGTERM"),
} = {}) {
  let stopped = 0;
  for (const candidate of selectOwnedProcesses(inventory(platform), root, platform)) {
    // Recheck identity immediately before signaling to reduce process-exit/PID-reuse races.
    const stillOwned = selectOwnedProcesses(inventory(platform), root, platform).some(
      (current) => current.pid === candidate.pid && current.executable === candidate.executable,
    );
    if (!stillOwned) continue;
    try {
      terminate(candidate.pid);
      stopped++;
    } catch {
      /* exited or not signalable */
    }
  }
  return stopped;
}

module.exports = {
  allowedExecutablePaths,
  selectOwnedProcesses,
  listProcesses,
  stopOwnedInstances,
};
if (require.main === module) {
  try {
    const stopped = stopOwnedInstances();
    if (stopped) console.log(`Reflow: stopped ${stopped} instance(s) built by this checkout.`);
  } catch {
    /* process inventory is best-effort; never stop an unverified process */
  }
}
