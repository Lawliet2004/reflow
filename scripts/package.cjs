#!/usr/bin/env node
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { spawnSync } = require("node:child_process");

const repositoryRoot = path.resolve(__dirname, "..");

function buildPlan({
  platform = process.platform,
  root = repositoryRoot,
  skipFrontendBuild = false,
  environment = process.env,
  allowMacosPreview = false,
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
  const options = { cwd: plan.cwd, stdio: "inherit", shell: false };
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
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--platform") platform = args[++index];
    else if (args[index] === "--skip-frontend-build") skipFrontendBuild = true;
    else if (args[index] === "--dry-run") dryRun = true;
    else if (args[index] === "--macos-preview") allowMacosPreview = true;
    else if (args[index] === "--sign-binaries")
      throw new Error(
        "This helper does not implement signing. Configure reviewed Tauri signing before building.",
      );
    else throw new Error(`Unknown packaging option: ${args[index]}`);
  }
  const plan = buildPlan({ platform, skipFrontendBuild, allowMacosPreview });
  if (dryRun) console.log(JSON.stringify(plan, null, 2));
  else execute(plan);
}

module.exports = { buildPlan, collectArtifacts, writeChecksums, execute, main };
if (require.main === module) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
