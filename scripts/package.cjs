#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const os = require("node:os");
const { spawnSync } = require("node:child_process");

const repositoryRoot = path.resolve(__dirname, "..");

// Hooks installed into the AppDir in a fixed order: the environment snapshot
// must run before linuxdeploy's generated hooks and the desktop fixup after.
const APPRUN_HOOKS = ["00-reflow-environment.sh", "zz-reflow-desktop.sh"];

function buildPlan({
  platform = process.platform,
  root = repositoryRoot,
  skipFrontendBuild = false,
  environment = process.env,
  allowMacosPreview = false,
  verbose = false,
} = {}) {
  if (platform === "darwin" && !allowMacosPreview)
    throw new Error(
      "macOS distribution is gated until dictation parity is verified; development builds retain clipboard/manual-copy fallback.",
    );
  const formats =
    platform === "win32"
      ? [
          ["nsis", ".exe"],
          ["msi", ".msi"],
        ]
      : platform === "linux"
        ? [
            ["deb", ".deb"],
            ["appimage", ".AppImage"],
          ]
        : platform === "darwin"
          ? [["dmg", ".dmg"]]
          : null;
  if (!formats) throw new Error(`Unsupported packaging platform: ${platform}`);
  const args = [
    path.join(root, "node_modules", "@tauri-apps", "cli", "tauri.js"),
    "build",
    "--bundles",
    formats.map(([name]) => name).join(","),
  ];
  if (verbose) args.push("--verbose");
  if (skipFrontendBuild)
    args.push("--config", JSON.stringify({ build: { beforeBuildCommand: "" } }));
  if (environment.CARGO_TARGET_DIR && !path.isAbsolute(environment.CARGO_TARGET_DIR)) {
    throw new Error(
      "Use an absolute CARGO_TARGET_DIR for packaging so installer output is unambiguous.",
    );
  }
  const target = environment.CARGO_TARGET_DIR || path.join(root, "src-tauri", "target");
  return {
    command: process.execPath,
    args,
    cwd: root,
    platform,
    skipFrontendBuild,
    artifactRoot: path.join(target, "release", "bundle"),
    artifactExtensions: formats.map(([, ext]) => ext),
    formats,
  };
}

function collectArtifacts(plan, { version, startedAt = 0 } = {}) {
  return plan.formats.flatMap(([format, extension]) => {
    const directory = path.join(plan.artifactRoot, format);
    const found = fs.existsSync(directory)
      ? fs
          .readdirSync(directory, { withFileTypes: true })
          .filter(
            (entry) =>
              entry.isFile() &&
              entry.name.endsWith(extension) &&
              (!version || entry.name.includes(`_${version}_`)),
          )
          .map((entry) => path.join(directory, entry.name))
          .filter((filename) => {
            const stat = fs.statSync(filename);
            return stat.size > 0 && stat.mtimeMs >= startedAt - 2000;
          })
      : [];
    if (!found.length)
      throw new Error(`Tauri did not produce a current ${format} installer in ${directory}`);
    return found;
  });
}

function writeChecksums(artifacts) {
  return artifacts.map((filename) => {
    const digest = crypto.createHash("sha256").update(fs.readFileSync(filename)).digest("hex");
    const output = `${filename}.sha256`;
    fs.writeFileSync(output, `${digest}  ${path.basename(filename)}\n`);
    return output;
  });
}

function finalizeAppImage(plan, artifacts, config, run = spawnSync) {
  if (plan.platform !== "linux") return;
  const appdir = path.join(plan.artifactRoot, "appimage", `${config.productName}.AppDir`);
  if (!fs.existsSync(path.join(appdir, "AppRun.wrapped")))
    throw new Error(
      "AppImage is missing the expected native AppRun wrapper; cannot preserve launch environment.",
    );
  const hooks = path.join(appdir, "apprun-hooks");
  fs.mkdirSync(hooks, { recursive: true });
  for (const name of APPRUN_HOOKS)
    fs.copyFileSync(path.join(plan.cwd, "linux", "apprun-hooks", name), path.join(hooks, name));
  // Tauri's linuxdeploy backend copies only usr/ from its staging tree, so
  // appimage.files cannot install root AppRun hooks. Finalize the generated
  // tree explicitly, then repack before any checksum or release upload.
  const launcher = `#!/usr/bin/env bash
set -e
this_dir="$(readlink -f "$(dirname "$0")")"
source "$this_dir/apprun-hooks/${APPRUN_HOOKS[0]}"
for hook in "$this_dir"/apprun-hooks/*.sh; do
  case "$(basename "$hook")" in ${APPRUN_HOOKS.join("|")}) continue;; esac
  source "$hook"
done
source "$this_dir/apprun-hooks/${APPRUN_HOOKS[1]}"
exec "$this_dir/AppRun.wrapped" "$@"
`;
  fs.writeFileSync(path.join(appdir, "AppRun"), launcher, { mode: 0o755 });
  const cache = process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  const plugin = path.join(cache, "tauri", "linuxdeploy-plugin-appimage.AppImage");
  if (!fs.existsSync(plugin))
    throw new Error("AppImage output plugin is missing after the Tauri build.");
  const image = artifacts.find((filename) => filename.endsWith(".AppImage"));
  if (!image) throw new Error("No AppImage was produced for finalization.");
  const result = run(plugin, ["--appimage-extract-and-run", "--appdir", appdir], {
    cwd: plan.cwd,
    stdio: "inherit",
    shell: false,
    timeout: 600_000,
    env: {
      ...process.env,
      APPIMAGE_EXTRACT_AND_RUN: "1",
      LDAI_OUTPUT: image,
      ARCH: process.arch === "x64" ? "x86_64" : process.arch === "arm64" ? "aarch64" : process.arch,
    },
  });
  if (result.error || result.status !== 0)
    throw new Error(
      "AppImage environment finalization failed; installer checksums were not written.",
    );
}

function execute(plan, run = spawnSync) {
  if (plan.platform !== process.platform)
    throw new Error(
      "Build installers on their matching Windows/Linux/macOS host; cross-platform dry-run plans are available.",
    );
  if (!fs.existsSync(plan.args[0]))
    throw new Error("Install the locked frontend dependencies with npm ci before packaging.");
  if (plan.skipFrontendBuild && !fs.existsSync(path.join(plan.cwd, "dist", "index.html"))) {
    throw new Error(
      "--skip-frontend-build requires existing dist/index.html; run npm run build first.",
    );
  }
  const env = { ...process.env };
  // Extract-and-run keeps AppImage tooling working on FUSE-less build hosts.
  if (plan.platform === "linux")
    env.APPIMAGE_EXTRACT_AND_RUN = process.env.APPIMAGE_EXTRACT_AND_RUN || "1";
  const options = { cwd: plan.cwd, stdio: "inherit", shell: false, env };
  const cargo = run("cargo", ["--version"], options);
  if (cargo.error || cargo.status !== 0) throw new Error("Cargo is required for packaging.");
  const startedAt = Date.now();
  const result = run(plan.command, plan.args, options);
  if (result.error || result.status !== 0)
    throw new Error(`Tauri installer build failed (${result.status ?? result.error?.message}).`);
  const config = JSON.parse(
    fs.readFileSync(path.join(plan.cwd, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const artifacts = collectArtifacts(plan, { version: config.version, startedAt });
  finalizeAppImage(plan, artifacts, config, run);
  writeChecksums(artifacts);
  console.log(`Created ${artifacts.length} installer(s), with SHA-256 checksums:`);
  artifacts.forEach((filename) => console.log(filename));
  return artifacts;
}

function main(args) {
  let platform = process.platform;
  let skipFrontendBuild = false;
  let dryRun = false;
  let allowMacosPreview = false;
  let verbose = false;
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--platform") platform = args[++index];
    else if (args[index] === "--skip-frontend-build") skipFrontendBuild = true;
    else if (args[index] === "--verbose") verbose = true;
    else if (args[index] === "--dry-run") dryRun = true;
    else if (args[index] === "--macos-preview") allowMacosPreview = true;
    else if (args[index] === "--sign-binaries")
      throw new Error(
        "This helper does not implement signing. Configure reviewed Tauri signing before building.",
      );
    else throw new Error(`Unknown packaging option: ${args[index]}`);
  }
  const plan = buildPlan({ platform, skipFrontendBuild, allowMacosPreview, verbose });
  if (dryRun) console.log(JSON.stringify(plan, null, 2));
  else execute(plan);
}

module.exports = { buildPlan, collectArtifacts, writeChecksums, finalizeAppImage, execute, main };
if (require.main === module) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
