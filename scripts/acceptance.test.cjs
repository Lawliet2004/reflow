const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { CHECKS, requiredProfiles, verifyAcceptance, TARGETS } = require("./acceptance.cjs");
const { fileDigest, platformInstallers } = require("./release.cjs");

test("publication requires exact native journeys and intact evidence for every supported profile", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "reflow-acceptance-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const commit = "a".repeat(40);
  assert.throws(() => verifyAcceptance(dir, "0.5.0", commit), /Missing runtime pack catalog/);
  const catalog = { schema: 1, release: "v0.5.0", source_commit: commit, packs: [] };
  for (const [platform, target] of Object.entries(TARGETS)) {
    for (const key of requiredProfiles(platform)) {
      const [profile, variant] = key.split("-");
      catalog.packs.push({ target, profile, variant, sha256: "c".repeat(64) });
    }
  }
  fs.writeFileSync(path.join(dir, "runtime-packs.json"), JSON.stringify(catalog));
  assert.throws(() => verifyAcceptance(dir, "0.5.0", commit), /Missing native acceptance/);
  for (const [platform, names] of Object.entries(platformInstallers("0.5.0"))) {
    if (platform === "android") continue;
    for (const name of names) fs.writeFileSync(path.join(dir, name), Buffer.alloc(4096));
    const proof = `acceptance-${platform}-fixture.log`;
    fs.writeFileSync(
      path.join(dir, proof),
      "Synthetic fixture for verifier tests; not native acceptance.\n",
    );
    const evidence = { schema: 1, version: "0.5.0", source_commit: commit, platform, runs: [] };
    for (const installer of names) {
      for (const session of platform === "linux"
        ? ["x11", "wayland-kde", "wayland-gnome"]
        : ["native"]) {
        for (const profile of requiredProfiles(platform)) {
          evidence.runs.push({
            installer,
            session,
            profile,
            installer_sha256: fileDigest(path.join(dir, installer)),
            runtime_pack_sha256: "c".repeat(64),
            clean_profile: true,
            no_system_python: true,
            os_version: "Synthetic fixture",
            hardware: "Synthetic fixture",
            loaded_device: "cpu",
            tested_at: "2026-10-22T00:00:00Z",
            checks: Object.fromEntries(CHECKS.map((check) => [check, "pass"])),
            evidence_files: [{ name: proof, sha256: fileDigest(path.join(dir, proof)) }],
          });
        }
      }
    }
    fs.writeFileSync(path.join(dir, `acceptance-${platform}.json`), JSON.stringify(evidence));
  }
  verifyAcceptance(dir, "0.5.0", commit);
  assert.throws(() => verifyAcceptance(dir, "0.5.0", "b".repeat(40)), /identity mismatch/);
  const file = path.join(dir, "acceptance-linux.json");
  const original = fs.readFileSync(file, "utf8");
  const mutate = (change, error) => {
    const evidence = JSON.parse(original);
    change(evidence);
    fs.writeFileSync(file, JSON.stringify(evidence));
    assert.throws(() => verifyAcceptance(dir, "0.5.0", commit), error);
    fs.writeFileSync(file, original);
  };
  mutate((e) => e.runs.pop(), /Missing or duplicate/);
  mutate((e) => e.runs.push(e.runs[0]), /Missing or duplicate/);
  mutate((e) => {
    e.runs[0].checks.runtime_setup = "skipped";
  }, /Unverified native journey/);
  mutate((e) => {
    e.runs[0].clean_profile = false;
  }, /Unverified native journey/);
  mutate((e) => {
    e.runs[0].installer_sha256 = "f".repeat(64);
  }, /Unverified native journey/);
  mutate((e) => {
    e.runs[0].runtime_pack_sha256 = "f".repeat(64);
  }, /Unverified native journey/);
  mutate((e) => {
    e.runs[0].evidence_files[0].name = "../acceptance-linux-fixture.log";
  }, /Invalid acceptance attachment/);
  fs.appendFileSync(path.join(dir, "acceptance-linux-fixture.log"), "changed");
  assert.throws(() => verifyAcceptance(dir, "0.5.0", commit), /attachment differs/);
});
