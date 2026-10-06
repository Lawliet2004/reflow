const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

function platformInstallers(version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Invalid release version");
  return {
    windows: [`Reflow_${version}_x64-setup.exe`, `Reflow_${version}_x64_en-US.msi`],
    linux: [`Reflow_${version}_amd64.deb`, `Reflow_${version}_amd64.AppImage`],
    "macos-arm64": [`Reflow_${version}_aarch64.dmg`],
    "macos-x64": [`Reflow_${version}_x64.dmg`],
    android: [`Reflow-${version}-android.apk`],
  };
}

function fileDigest(filename) {
  const hash = crypto.createHash("sha256");
  const descriptor = fs.openSync(filename, "r");
  const buffer = Buffer.alloc(1024 * 1024);
  try {
    let count;
    while ((count = fs.readSync(descriptor, buffer, 0, buffer.length, null)) > 0)
      hash.update(buffer.subarray(0, count));
  } finally {
    fs.closeSync(descriptor);
  }
  return hash.digest("hex");
}

function buildManifest(directory, version, platform) {
  const required = platformInstallers(version)[platform];
  if (!required) throw new Error("Unknown build platform");
  const found = new Map();
  function visit(folder) {
    for (const entry of fs.readdirSync(folder, { withFileTypes: true })) {
      const filename = path.join(folder, entry.name);
      if (entry.isDirectory()) visit(filename);
      else if (entry.isFile() && required.includes(entry.name)) {
        if (found.has(entry.name)) throw new Error(`Duplicate built installer: ${entry.name}`);
        found.set(entry.name, filename);
      }
    }
  }
  visit(directory);
  return {
    version,
    platform,
    assets: required.map((name) => {
      const filename = found.get(name);
      if (!filename) throw new Error(`Missing built installer: ${name}`);
      return { name, size: fs.statSync(filename).size, sha256: fileDigest(filename) };
    }),
  };
}

function verifyBuildManifests(directory, version) {
  for (const [platform, required] of Object.entries(platformInstallers(version))) {
    const filename = path.join(directory, `build-${platform}.json`);
    if (!fs.existsSync(filename)) throw new Error(`Missing build manifest: ${platform}`);
    const manifest = JSON.parse(fs.readFileSync(filename, "utf8"));
    if (
      manifest.version !== version ||
      manifest.platform !== platform ||
      !Array.isArray(manifest.assets) ||
      manifest.assets.length !== required.length
    )
      throw new Error(`Invalid build manifest: ${platform}`);
    for (const name of required) {
      const entries = manifest.assets.filter((asset) => asset.name === name);
      const built = entries[0];
      const assetPath = path.join(directory, name);
      if (
        entries.length !== 1 ||
        !Number.isSafeInteger(built.size) ||
        built.size < 4096 ||
        !/^[a-f0-9]{64}$/.test(built.sha256) ||
        fs.statSync(assetPath).size !== built.size ||
        fileDigest(assetPath) !== built.sha256
      )
        throw new Error(`Installer differs from original build: ${name}`);
    }
  }
}

function verifyVersion(tag, root = path.resolve(__dirname, "..")) {
  if (!/^v\d+\.\d+\.\d+$/.test(tag)) throw new Error("Release tag must be vMAJOR.MINOR.PATCH");
  const version = tag.slice(1);
  const read = (file) => fs.readFileSync(path.join(root, file), "utf8");
  const json = (file) => JSON.parse(read(file));
  const lock = json("package-lock.json");
  const versions = [
    json("package.json").version,
    lock.version,
    lock.packages[""].version,
    json("src-tauri/tauri.conf.json").version,
    read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1],
    read("src-tauri/Cargo.lock").match(/name = "reflow"\r?\nversion = "([^"]+)"/)?.[1],
    read("android/app/build.gradle.kts").match(/versionName = "([^"]+)"/)?.[1],
  ];
  if (versions.some((value) => value !== version))
    throw new Error(`Release versions disagree with ${tag}: ${versions.join(", ")}`);
  return version;
}

function verifyAssets(directory, version) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error("Invalid release version");
  const escaped = version.replaceAll(".", "\\.");
  const required = [
    new RegExp(`^Reflow_${escaped}_x64-setup\\.exe$`),
    new RegExp(`^Reflow_${escaped}_x64_en-US\\.msi$`),
    new RegExp(`^Reflow_${escaped}_amd64\\.deb$`),
    new RegExp(`^Reflow_${escaped}_amd64\\.AppImage$`),
    new RegExp(`^Reflow_${escaped}_aarch64\\.dmg$`),
    new RegExp(`^Reflow_${escaped}_x64\\.dmg$`),
    new RegExp(`^Reflow-${escaped}-android\\.apk$`),
  ];
  const files = fs.readdirSync(directory).filter((name) => {
    const stat = fs.statSync(path.join(directory, name));
    return stat.isFile();
  });
  for (const pattern of required) {
    if (!files.some((name) => pattern.test(name))) throw new Error(`Missing installer: ${pattern}`);
  }
  const installers = files.filter(
    (name) =>
      required.some((pattern) => pattern.test(name)) ||
      new RegExp(`^Reflow(?:_${escaped})?_(?:aarch64|x64)\\.app\\.tar\\.gz$`).test(name),
  );
  const metadata = [
    "SHA256SUMS.txt",
    ...Object.keys(platformInstallers(version)).map((platform) => `build-${platform}.json`),
  ];
  for (const name of files) {
    if (!installers.includes(name) && !metadata.includes(name))
      throw new Error(`Unexpected release asset: ${name}`);
  }
  for (const name of installers) {
    const filename = path.join(directory, name);
    const size = fs.statSync(filename).size;
    if (size < 4096) throw new Error(`Invalid installer format (truncated): ${name}`);
    const descriptor = fs.openSync(filename, "r");
    const header = Buffer.alloc(12);
    const trailer = Buffer.alloc(4);
    try {
      fs.readSync(descriptor, header, 0, header.length, 0);
      if (size >= 512) fs.readSync(descriptor, trailer, 0, trailer.length, size - 512);
    } finally {
      fs.closeSync(descriptor);
    }
    const starts = (bytes) => header.subarray(0, bytes.length).equals(bytes);
    const valid = name.endsWith(".exe")
      ? starts(Buffer.from("MZ"))
      : name.endsWith(".msi")
        ? starts(Buffer.from("d0cf11e0a1b11ae1", "hex"))
        : name.endsWith(".deb")
          ? starts(Buffer.from("!<arch>\n"))
          : name.endsWith(".AppImage")
            ? starts(Buffer.from([0x7f, 0x45, 0x4c, 0x46])) &&
              header.subarray(8, 11).equals(Buffer.from([0x41, 0x49, 2]))
            : name.endsWith(".dmg")
              ? size >= 512 && trailer.toString("ascii") === "koly"
              : name.endsWith(".app.tar.gz")
                ? starts(Buffer.from([0x1f, 0x8b, 8]))
                : starts(Buffer.from([0x50, 0x4b, 3, 4]));
    if (!valid) throw new Error(`Invalid installer format: ${name}`);
  }
  verifyBuildManifests(directory, version);
  return installers.sort();
}

function draftPayload(tag, root = path.resolve(__dirname, "..")) {
  const version = verifyVersion(tag, root);
  return {
    tag_name: tag,
    name: `Reflow ${version}`,
    body: fs.readFileSync(path.join(root, `docs/release-${version}.md`), "utf8"),
    draft: true,
  };
}

if (require.main === module) {
  const [command, arg, version, platform, output] = process.argv.slice(2);
  if (command === "verify-version") console.log(verifyVersion(arg));
  else if (command === "draft-payload") console.log(JSON.stringify(draftPayload(arg)));
  else if (command === "build-manifest") {
    if (!output) throw new Error("Provide a build manifest output path");
    fs.writeFileSync(output, JSON.stringify(buildManifest(arg, version, platform), null, 2));
  } else if (command === "verify-assets") {
    const files = verifyAssets(arg, version);
    const sums = files
      .map((name) => {
        const digest = fileDigest(path.join(arg, name));
        return `${digest}  ${name}\n`;
      })
      .join("");
    fs.writeFileSync(path.join(arg, "SHA256SUMS.txt"), sums);
    console.log(`Verified ${files.length} installers; wrote SHA256SUMS.txt`);
  } else
    throw new Error(
      "Use verify-version TAG, draft-payload TAG, verify-assets DIRECTORY VERSION or build-manifest DIRECTORY VERSION PLATFORM OUTPUT",
    );
}
module.exports = { verifyVersion, verifyAssets, draftPayload, buildManifest };
