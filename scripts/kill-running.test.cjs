const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { selectOwnedProcesses, stopOwnedInstances } = require("./kill-running.cjs");

test("automatic shutdown never matches unrelated command lines", () => {
  const commands = [];
  const source = fs.readFileSync(path.join(__dirname, "kill-running.cjs"), "utf8");
  const child = {
    execSync: (command) => {
      commands.push(command);
    },
    execFileSync: () => "",
  };
  const processStub = {
    platform: "linux",
    pid: 1,
    argv: [],
    env: {},
    kill: () => {},
    execPath: process.execPath,
  };
  vm.runInNewContext(source, {
    require: (name) =>
      name === "child_process" || name === "node:child_process" ? child : require(name),
    process: processStub,
    console: { log() {}, warn() {} },
    __dirname,
    module: { exports: {} },
  });
  assert.ok(
    commands.every((command) => !command.includes("pkill -f") && !command.includes("/IM")),
    "Shutdown matched executable names or arbitrary command lines instead of checkout paths",
  );
});

test("only exact debug/release executable paths from this checkout are selected", () => {
  const root = path.resolve(__dirname, "..");
  const owned = path.join(root, "src-tauri", "target", "debug", "reflow.exe");
  const selected = selectOwnedProcesses(
    [
      { pid: 11, executable: owned },
      { pid: 12, executable: path.join(root, "src-tauri", "target", "release", "reflow.exe") },
      { pid: 13, executable: path.join(root, "other-checkout", "reflow.exe") },
      { pid: 14, executable: `${owned}.backup` },
      { pid: 15, executable: "C:/Program Files/Reflow/reflow.exe" },
      { pid: 16, executable: null },
      { pid: 99, executable: owned },
    ],
    root,
    "win32",
    99,
  );
  assert.deepEqual(
    selected.map(({ pid }) => pid),
    [11, 12],
  );
});

test("a changed process identity cannot receive a shutdown signal", () => {
  const root = path.resolve(__dirname, "..");
  const executable = path.join(root, "src-tauri", "target", "debug", "reflow");
  let samples = 0;
  const signals = [];
  const stopped = stopOwnedInstances({
    root,
    platform: "linux",
    inventory: () => [{ pid: 77, executable: samples++ === 0 ? executable : "/usr/bin/unrelated" }],
    terminate: (pid) => signals.push(pid),
  });
  assert.equal(stopped, 0);
  assert.deepEqual(signals, []);
});

test("shutdown signals only a still-owned PID and tolerates already-exited processes", () => {
  const root = path.resolve(__dirname, "..");
  const inventory = () => [
    { pid: 77, executable: path.join(root, "src-tauri", "target", "debug", "reflow") },
  ];
  const signals = [];
  assert.equal(
    stopOwnedInstances({
      root,
      platform: "linux",
      inventory,
      terminate: (pid) => signals.push(pid),
    }),
    1,
  );
  assert.deepEqual(signals, [77]);
  assert.equal(
    stopOwnedInstances({
      root,
      platform: "linux",
      inventory,
      terminate: () => {
        throw new Error("exited");
      },
    }),
    0,
  );
});
