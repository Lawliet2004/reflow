const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");

const root = path.resolve(__dirname, "..");
const helper = fs.existsSync(path.join(__dirname, "package.cjs")) ? require("./package.cjs") : {};

test(
  "AppImage hooks retain explicit backends and let GTK choose the native session",
  { skip: process.platform !== "linux" },
  () => {
    const config = JSON.parse(
      fs.readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8"),
    );
    assert.equal(config.bundle.linux.appimage.bundleMediaFramework, true);
    assert.equal(typeof helper.finalizeAppImage, "function");
    for (const original of [undefined, "x11", "wayland"]) {
      const env = { ...process.env };
      delete env.GDK_BACKEND;
      if (original) env.GDK_BACKEND = original;
      const result = spawnSync(
        "bash",
        [
          "-c",
          'source "$1"; export GDK_BACKEND=x11; source "$2"; printf "%s" "${GDK_BACKEND-unset}"',
          "reflow-hook-test",
          path.join(root, "linux/apprun-hooks/00-reflow-environment.sh"),
          path.join(root, "linux/apprun-hooks/zz-reflow-desktop.sh"),
        ],
        { env, encoding: "utf8" },
      );
      assert.equal(result.status, 0, result.stderr);
      assert.equal(result.stdout, original ?? "unset");
    }
  },
);

test("installed speech runtime and language catalogue use the paths the app resolves", () => {
  const config = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const resources = config.bundle.resources;
  assert.equal(
    Array.isArray(resources),
    false,
    "Parent-relative array resources install under _up_, outside runtime discovery",
  );
  assert.equal(
    resources["../model-runtime/qwen3_asr_runtime.py"],
    "model-runtime/qwen3_asr_runtime.py",
  );
  assert.equal(resources["../model-runtime/languages.json"], "model-runtime/languages.json");
  for (const source of Object.keys(resources)) {
    assert.ok(fs.statSync(path.resolve(root, "src-tauri", source)).isFile());
  }
});

test("bundle plan invokes Tauri once from the repository root without requiring Python", () => {
  assert.equal(
    typeof helper.buildPlan,
    "function",
    "Packaging needs a reviewable bundle command plan",
  );
  const plan = helper.buildPlan({ platform: "win32", root });
  assert.equal(plan.cwd, root);
  assert.deepEqual(plan.args.slice(1), ["build", "--bundles", "nsis,msi"]);
  assert.equal(path.basename(plan.args[0]), "tauri.js");
  assert.equal(plan.command, process.execPath);
  assert.deepEqual(plan.artifactExtensions, [".exe", ".msi"]);
});

function windowsWrapper(args = []) {
  const filename = path.join(__dirname, "package_windows.ps1").replaceAll("'", "''");
  const commands = ["node", "cargo", "npm", "python"].map(
    (name) =>
      `function global:${name} { Write-Output ('RECORD ' + (@{ command='${name}'; cwd=(Get-Location).Path; args=@($args) } | ConvertTo-Json -Compress)); $global:LASTEXITCODE=0 }`,
  );
  const script = `${commands.join("\n")}\nWrite-Output ('BEFORE '+(Get-Location).Path); try { & '${filename}' ${args.join(" ")} } catch { Write-Output ('FAILURE '+ $_.Exception.Message); exit 1 }; Write-Output ('CALLER '+(Get-Location).Path)`;
  return spawnSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", script], {
    cwd: os.tmpdir(),
    encoding: "utf8",
  });
}

test(
  "Windows wrapper preserves caller cwd and delegates the actual installer command",
  { skip: process.platform !== "win32" },
  () => {
    const result = windowsWrapper();
    assert.equal(result.status, 0, result.stdout + result.stderr);
    const caller = result.stdout.split(/\r?\n/).find((line) => line.startsWith("CALLER "));
    const before = result.stdout.split(/\r?\n/).find((line) => line.startsWith("BEFORE "));
    assert.equal(
      caller?.slice(7),
      before?.slice(7),
      "Packaging changed the caller's working directory",
    );
    const records = result.stdout
      .split(/\r?\n/)
      .filter((line) => line.startsWith("RECORD "))
      .map((line) => JSON.parse(line.slice(7)));
    assert.equal(records.length, 1);
    assert.equal(records[0].command, "node");
    assert.equal(path.basename(records[0].args[0]), "package.cjs");
  },
);

test(
  "unsupported signing fails before any build or pretend success",
  { skip: process.platform !== "win32" },
  () => {
    const result = windowsWrapper(["-SignBinaries"]);
    assert.notEqual(result.status, 0, "Unimplemented signing was reported as successful");
    assert.match(result.stdout, /signing/i);
    assert.ok(!result.stdout.includes("RECORD "), "Unsupported signing ran build commands");
  },
);

test("macOS distribution is gated until dictation parity is verified", () => {
  assert.equal(typeof helper.buildPlan, "function");
  assert.throws(
    () => helper.buildPlan({ platform: "darwin", root }),
    /macOS.*clipboard|macOS.*parity/i,
  );
});

test("an explicitly requested macOS preview produces a real DMG plan", () => {
  const plan = helper.buildPlan({ platform: "darwin", root, allowMacosPreview: true });
  assert.deepEqual(plan.args.slice(1), ["build", "--bundles", "dmg"]);
  assert.deepEqual(plan.artifactExtensions, [".dmg"]);
  const plist = fs.readFileSync(path.join(root, "src-tauri", "Info.plist"), "utf8");
  assert.match(plist, /NSMicrophoneUsageDescription/);
  const config = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const entitlements = fs.readFileSync(
    path.join(root, "src-tauri", config.bundle.macOS.entitlements),
    "utf8",
  );
  assert.match(entitlements, /com.apple.security.device.audio-input/);
  assert.match(entitlements, /<true\/>/);
});

test("Linux plans request real deb/AppImage bundles and frontend skip is explicit", () => {
  const plan = helper.buildPlan({
    platform: "linux",
    root,
    skipFrontendBuild: true,
    environment: {},
  });
  assert.deepEqual(plan.args.slice(1, 4), ["build", "--bundles", "deb,appimage"]);
  assert.deepEqual(JSON.parse(plan.args.at(-1)), { build: { beforeBuildCommand: "" } });
  assert.deepEqual(plan.artifactExtensions, [".deb", ".AppImage"]);
});

function fixture(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "reflow-package-tests-"));
  t.after(() => {
    const resolved = path.resolve(directory);
    assert.equal(path.dirname(resolved), path.resolve(os.tmpdir()));
    fs.rmSync(resolved, { recursive: true, force: true });
  });
  return directory;
}

test(
  "AppImage finalization wraps GTK hooks before repacking and preserves explicit input",
  { skip: process.platform !== "linux" },
  (t) => {
    const directory = fixture(t);
    const plan = helper.buildPlan({ platform: "linux", root: directory, environment: {} });
    const appdir = path.join(plan.artifactRoot, "appimage", "Reflow.AppDir");
    fs.mkdirSync(path.join(appdir, "apprun-hooks"), { recursive: true });
    fs.mkdirSync(path.join(directory, "linux", "apprun-hooks"), { recursive: true });
    for (const name of ["00-reflow-environment.sh", "zz-reflow-desktop.sh"])
      fs.copyFileSync(
        path.join(root, "linux", "apprun-hooks", name),
        path.join(directory, "linux", "apprun-hooks", name),
      );
    fs.writeFileSync(
      path.join(appdir, "AppRun.wrapped"),
      '#!/usr/bin/env bash\nprintf "%s" "${GDK_BACKEND-unset}|${REFLOW_APPIMAGE_ORIGINAL_PYTHONHOME}"\n',
      { mode: 0o755 },
    );
    fs.writeFileSync(
      path.join(appdir, "apprun-hooks", "linuxdeploy-plugin-gtk.sh"),
      "export GDK_BACKEND=x11\n",
    );
    const image = path.join(plan.artifactRoot, "appimage", "Reflow_0.4.0_amd64.AppImage");
    const cache = path.join(directory, "cache");
    fs.mkdirSync(path.join(cache, "tauri"), { recursive: true });
    fs.writeFileSync(path.join(cache, "tauri", "linuxdeploy-plugin-appimage.AppImage"), "fixture");
    const previousCache = process.env.XDG_CACHE_HOME;
    process.env.XDG_CACHE_HOME = cache;
    try {
      helper.finalizeAppImage(
        plan,
        [image],
        { productName: "Reflow" },
        (_program, args, options) => {
          assert.deepEqual(args, ["--appimage-extract-and-run", "--appdir", appdir]);
          assert.equal(options.env.LDAI_OUTPUT, image);
          assert.equal(options.shell, false);
          return { status: 0 };
        },
      );
    } finally {
      if (previousCache === undefined) delete process.env.XDG_CACHE_HOME;
      else process.env.XDG_CACHE_HOME = previousCache;
    }
    const env = { ...process.env, GDK_BACKEND: "wayland", PYTHONHOME: "/chosen/python" };
    const result = spawnSync(path.join(appdir, "AppRun"), [], { env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "wayland|/chosen/python");
  },
);

test("checksums cover actual installer bytes and require every requested format", (t) => {
  const directory = fixture(t);
  const plan = helper.buildPlan({ platform: "win32", root: directory, environment: {} });
  const exe = path.join(plan.artifactRoot, "nsis", "Reflow_1.0.0_x64-setup.exe");
  const msi = path.join(plan.artifactRoot, "msi", "Reflow_1.0.0_x64_en-US.msi");
  fs.mkdirSync(path.dirname(exe), { recursive: true });
  fs.writeFileSync(exe, "installer fixture");
  assert.throws(() => helper.collectArtifacts(plan, { version: "1.0.0" }), /msi installer/);
  fs.mkdirSync(path.dirname(msi), { recursive: true });
  fs.writeFileSync(msi, "MSI fixture");
  const artifacts = helper.collectArtifacts(plan, { version: "1.0.0" });
  assert.deepEqual(artifacts, [exe, msi]);
  helper.writeChecksums(artifacts);
  const expected = crypto.createHash("sha256").update("installer fixture").digest("hex");
  assert.equal(fs.readFileSync(`${exe}.sha256`, "utf8"), `${expected}  ${path.basename(exe)}\n`);
  assert.throws(
    () => helper.collectArtifacts(plan, { version: "2.0.0" }),
    /current nsis installer/,
  );
  const old = new Date(Date.now() - 10_000);
  fs.utimesSync(exe, old, old);
  assert.throws(
    () => helper.collectArtifacts(plan, { version: "1.0.0", startedAt: Date.now() }),
    /current nsis installer/,
  );
});

test("failed or mismatched Tauri command cannot generate checksums or claim success", (t) => {
  const directory = fixture(t);
  const plan = helper.buildPlan({
    platform: process.platform === "darwin" ? "win32" : process.platform,
    root: directory,
    environment: {},
  });
  fs.mkdirSync(path.dirname(plan.args[0]), { recursive: true });
  fs.writeFileSync(plan.args[0], "CLI fixture");
  if (process.platform === "darwin") {
    assert.throws(
      () =>
        helper.execute(plan, () => {
          throw new Error("Unexpected build");
        }),
      /matching Windows\/Linux\/macOS host/,
    );
    return;
  }
  const commands = [];
  assert.throws(
    () =>
      helper.execute(plan, (command, args, options) => {
        commands.push({ command, args, options });
        return { status: command === "cargo" ? 0 : 1 };
      }),
    /installer build failed/,
  );
  assert.equal(commands.length, 2);
  assert.equal(commands[1].options.cwd, directory);
  assert.equal(commands[1].options.shell, false);
  if (process.platform === "linux") {
    assert.equal(
      commands[1].options.env.APPIMAGE_EXTRACT_AND_RUN,
      process.env.APPIMAGE_EXTRACT_AND_RUN || "1",
    );
  }
  assert.ok(!fs.existsSync(plan.artifactRoot));
});

test("Linux AppStream metadata references the generated desktop launcher", () => {
  const config = JSON.parse(
    fs.readFileSync(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const metadata = fs.readFileSync(path.join(root, "linux", "com.reflow.app.metainfo.xml"), "utf8");
  const launcher = metadata.match(/<launchable type="desktop-id">([^<]+)<\/launchable>/)?.[1];
  assert.equal(launcher, `${config.productName}.desktop`);
});
