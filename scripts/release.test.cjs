const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { verifyVersion, verifyAssets } = require("./release.cjs");

test("release versions agree across desktop, lockfiles and Android", () => {
  const version = require("../package.json").version;
  assert.equal(verifyVersion(`v${version}`), version);
  assert.throws(() => verifyVersion("v0.0.0"), /disagree/);
  assert.throws(() => verifyVersion("v3; echo bad"), /tag/);
});

test("publication requires every platform installer, with correct version and nonempty bytes", (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "reflow-release-"));
  t.after(() => {
    assert.equal(path.dirname(path.resolve(dir)), path.resolve(os.tmpdir()));
    fs.rmSync(dir, { recursive: true, force: true });
  });
  const names = [
    "Reflow_0.3.0_x64-setup.exe",
    "Reflow_0.3.0_x64_en-US.msi",
    "Reflow_0.3.0_amd64.deb",
    "Reflow_0.3.0_amd64.AppImage",
    "Reflow_0.3.0_aarch64.dmg",
    "Reflow_0.3.0_x64.dmg",
    "Reflow-0.3.0-android.apk",
  ];
  for (const name of names.slice(0, -1))
    fs.writeFileSync(path.join(dir, name), "installer fixture");
  assert.throws(() => verifyAssets(dir, "0.3.0"), /apk/);
  fs.writeFileSync(path.join(dir, names.at(-1)), "");
  assert.throws(() => verifyAssets(dir, "0.3.0"), /apk/);
  fs.writeFileSync(path.join(dir, names.at(-1)), "signed APK fixture");
  assert.deepEqual(verifyAssets(dir, "0.3.0"), names.sort());
  assert.throws(() => verifyAssets(dir, "0.4.0"), /Missing installer/);
});
