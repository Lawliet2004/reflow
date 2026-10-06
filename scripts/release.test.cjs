const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { verifyVersion, verifyAssets, draftPayload, buildManifest } = require("./release.cjs");

function installerFixture(name) {
  const fixture = Buffer.alloc(4096);
  let magic;
  if (name.endsWith(".exe")) magic = Buffer.from("MZ executable fixture");
  else if (name.endsWith(".msi")) magic = Buffer.from("d0cf11e0a1b11ae1", "hex");
  else if (name.endsWith(".deb")) magic = Buffer.from("!<arch>\nfixture");
  else if (name.endsWith(".AppImage"))
    magic = Buffer.from([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0, 0x41, 0x49, 2]);
  else if (name.endsWith(".dmg")) {
    fixture.write("koly", fixture.length - 512);
    return fixture;
  } else if (name.endsWith(".app.tar.gz")) magic = Buffer.from([0x1f, 0x8b, 8, 0]);
  else magic = Buffer.from([0x50, 0x4b, 3, 4]);
  magic.copy(fixture);
  return fixture;
}

test("release versions agree across desktop, lockfiles and Android", () => {
  const version = require("../package.json").version;
  assert.equal(verifyVersion(`v${version}`), version);
  assert.throws(() => verifyVersion("v0.0.0"), /disagree/);
  assert.throws(() => verifyVersion("v3; echo bad"), /tag/);
});

test("release creation uses a draft payload with the exact release notes", () => {
  const version = require("../package.json").version;
  const payload = JSON.parse(JSON.stringify(draftPayload(`v${version}`)));
  assert.deepEqual(payload, {
    tag_name: `v${version}`,
    name: `Reflow ${version}`,
    body: fs.readFileSync(path.join(__dirname, `../docs/release-${version}.md`), "utf8"),
    draft: true,
  });
  assert.ok(payload.body.includes("\n"));
  assert.throws(() => draftPayload("v0.0.0"), /disagree/);
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
    fs.writeFileSync(path.join(dir, name), installerFixture(name));
  assert.throws(() => verifyAssets(dir, "0.3.0"), /apk/);
  fs.writeFileSync(path.join(dir, names.at(-1)), "");
  assert.throws(() => verifyAssets(dir, "0.3.0"), /apk/);
  fs.writeFileSync(path.join(dir, names.at(-1)), installerFixture(names.at(-1)));
  assert.throws(() => verifyAssets(dir, "0.3.0"), /Missing build manifest/);
  for (const platform of ["windows", "linux", "macos-arm64", "macos-x64", "android"])
    fs.writeFileSync(
      path.join(dir, `build-${platform}.json`),
      JSON.stringify(buildManifest(dir, "0.3.0", platform)),
    );
  assert.deepEqual(verifyAssets(dir, "0.3.0"), names.sort());
  assert.throws(() => verifyAssets(dir, "0.4.0"), /Missing installer/);
  for (const name of names) {
    fs.writeFileSync(path.join(dir, name), "corrupted installer");
    assert.throws(() => verifyAssets(dir, "0.3.0"), /Invalid installer format/);
    fs.writeFileSync(path.join(dir, name), installerFixture(name).subarray(0, 12));
    assert.throws(() => verifyAssets(dir, "0.3.0"), /truncated/);
    fs.writeFileSync(path.join(dir, name), installerFixture(name));
  }
  const changed = installerFixture(names[0]);
  changed[4000] = 1;
  fs.writeFileSync(path.join(dir, names[0]), changed);
  assert.throws(() => verifyAssets(dir, "0.3.0"), /differs from original build/);
  fs.writeFileSync(path.join(dir, names[0]), installerFixture(names[0]));
  const buildPath = path.join(dir, "build-windows.json");
  const originalBuild = fs.readFileSync(buildPath, "utf8");
  const wrongVersion = JSON.parse(originalBuild);
  wrongVersion.version = "0.0.0";
  fs.writeFileSync(buildPath, JSON.stringify(wrongVersion));
  assert.throws(() => verifyAssets(dir, "0.3.0"), /Invalid build manifest/);
  fs.writeFileSync(buildPath, originalBuild);
  const archive = "Reflow_aarch64.app.tar.gz";
  fs.writeFileSync(path.join(dir, archive), "");
  assert.throws(() => verifyAssets(dir, "0.3.0"), /truncated/);
  fs.writeFileSync(path.join(dir, archive), installerFixture(archive));
  assert.deepEqual(verifyAssets(dir, "0.3.0"), [...names, archive].sort());
  const wrongArchive = "Reflow_0.2.0_x64.app.tar.gz";
  fs.writeFileSync(path.join(dir, wrongArchive), installerFixture(wrongArchive));
  assert.throws(() => verifyAssets(dir, "0.3.0"), /Unexpected release asset/);
  fs.unlinkSync(path.join(dir, wrongArchive));
  fs.writeFileSync(path.join(dir, archive), installerFixture(archive).subarray(0, 3));
  assert.throws(() => verifyAssets(dir, "0.3.0"), /truncated/);
  const nested = path.join(dir, "nested");
  fs.mkdirSync(nested);
  fs.copyFileSync(
    path.join(
      dir,
      names.find((name) => name.endsWith(".exe")),
    ),
    path.join(
      nested,
      names.find((name) => name.endsWith(".exe")),
    ),
  );
  assert.throws(() => buildManifest(dir, "0.3.0", "windows"), /Duplicate built installer/);
  assert.throws(() => buildManifest(dir, "0.3.0", "unknown"), /Unknown build platform/);
});
