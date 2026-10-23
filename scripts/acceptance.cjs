// Native installer evidence is collected on clean machines, never synthesized by CI.
const fs = require("node:fs");
const path = require("node:path");
const { buildManifest, fileDigest, platformInstallers } = require("./release.cjs");

const TARGETS = {
  windows: "x86_64-pc-windows-msvc",
  linux: "x86_64-unknown-linux-gnu",
  "macos-arm64": "aarch64-apple-darwin",
  "macos-x64": "x86_64-apple-darwin",
};
const CHECKS = [
  "install_and_launch",
  "permission_denial_and_retry",
  "microphone",
  "shortcut_or_explicit_record_button",
  "hardware_plan",
  "runtime_setup",
  "model_download_and_recognition",
  "external_insertion_or_explicit_clipboard",
  "restart_and_offline_dictation",
  "optional_writing_runtime_then_model",
  "cancel_resume_and_failed_repair_recovery",
  "signature_and_quarantine",
  "runtime_pack_license_review",
];

function requiredProfiles(platform) {
  const target = TARGETS[platform];
  if (!target) throw new Error("Unknown acceptance platform");
  return fs
    .readdirSync(path.join(__dirname, "../model-runtime/locks"))
    .filter((name) => name.startsWith(`${target}-`) && name.endsWith(".txt"))
    .map((name) => name.slice(target.length + 1, -4))
    .sort();
}

function verifyAcceptance(directory, version, commit) {
  if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error("Acceptance needs an exact source commit");
  const catalogPath = path.join(directory, "runtime-packs.json");
  if (!fs.existsSync(catalogPath)) throw new Error("Missing runtime pack catalog");
  const catalog = JSON.parse(fs.readFileSync(catalogPath, "utf8"));
  if (
    catalog.schema !== 1 ||
    catalog.release !== `v${version}` ||
    catalog.source_commit !== commit ||
    !Array.isArray(catalog.packs)
  )
    throw new Error("Runtime pack catalog identity mismatch");
  for (const platform of Object.keys(TARGETS)) {
    const filename = path.join(directory, `acceptance-${platform}.json`);
    if (!fs.existsSync(filename))
      throw new Error(`Missing native acceptance evidence: ${platform}`);
    const evidence = JSON.parse(fs.readFileSync(filename, "utf8"));
    if (
      evidence.schema !== 1 ||
      evidence.version !== version ||
      evidence.source_commit !== commit ||
      evidence.platform !== platform ||
      !Array.isArray(evidence.runs)
    )
      throw new Error(`Acceptance identity mismatch: ${platform}`);
    const built = buildManifest(directory, version, platform);
    const profiles = requiredProfiles(platform);
    const sessions = platform === "linux" ? ["x11", "wayland-kde", "wayland-gnome"] : ["native"];
    for (const installer of platformInstallers(version)[platform]) {
      for (const session of sessions) {
        for (const profile of profiles) {
          const packs = catalog.packs.filter(
            (pack) =>
              pack.target === TARGETS[platform] && `${pack.profile}-${pack.variant}` === profile,
          );
          if (packs.length !== 1 || !/^[a-f0-9]{64}$/.test(packs[0].sha256 ?? ""))
            throw new Error(`Missing or duplicate runtime pack: ${platform}/${profile}`);
          const runs = evidence.runs.filter(
            (run) =>
              run.installer === installer && run.session === session && run.profile === profile,
          );
          if (runs.length !== 1)
            throw new Error(`Missing or duplicate acceptance: ${installer}/${session}/${profile}`);
          const run = runs[0];
          if (
            run.installer_sha256 !==
              built.assets.find((asset) => asset.name === installer).sha256 ||
            run.runtime_pack_sha256 !== packs[0].sha256 ||
            run.clean_profile !== true ||
            run.no_system_python !== true ||
            typeof run.os_version !== "string" ||
            !run.os_version.trim() ||
            typeof run.hardware !== "string" ||
            !run.hardware.trim() ||
            typeof run.loaded_device !== "string" ||
            !run.loaded_device.trim() ||
            !Number.isFinite(Date.parse(run.tested_at)) ||
            Date.parse(run.tested_at) > Date.now() + 60_000 ||
            CHECKS.some((check) => run.checks?.[check] !== "pass") ||
            !Array.isArray(run.evidence_files) ||
            run.evidence_files.length < 1 ||
            run.evidence_files.length > 64
          )
            throw new Error(`Unverified native journey: ${installer}/${session}/${profile}`);
          for (const proof of run.evidence_files) {
            if (
              typeof proof.name !== "string" ||
              !proof.name.startsWith(`acceptance-${platform}-`) ||
              !/^[a-zA-Z0-9._-]+\.(?:log|txt|json|png)$/.test(proof.name) ||
              !/^[a-f0-9]{64}$/.test(proof.sha256 ?? "")
            )
              throw new Error(`Invalid acceptance attachment: ${platform}`);
            const proofPath = path.join(directory, proof.name);
            const stat = fs.lstatSync(proofPath);
            if (
              !stat.isFile() ||
              stat.size < 1 ||
              stat.size > 100 * 1024 * 1024 ||
              fileDigest(proofPath) !== proof.sha256
            )
              throw new Error(`Acceptance attachment differs: ${proof.name}`);
          }
        }
      }
    }
  }
}

if (require.main === module) {
  const [directory, version, commit] = process.argv.slice(2);
  verifyAcceptance(directory, version, commit);
  console.log("Native installer acceptance evidence matches the release artifacts and source.");
}
module.exports = { CHECKS, requiredProfiles, verifyAcceptance, TARGETS };
