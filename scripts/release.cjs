const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");

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
    return stat.isFile() && stat.size > 0;
  });
  for (const pattern of required) {
    if (!files.some((name) => pattern.test(name))) throw new Error(`Missing installer: ${pattern}`);
  }
  return files.filter((name) => required.some((pattern) => pattern.test(name))).sort();
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
  const [command, arg, version] = process.argv.slice(2);
  if (command === "verify-version") console.log(verifyVersion(arg));
  else if (command === "draft-payload") console.log(JSON.stringify(draftPayload(arg)));
  else if (command === "verify-assets") {
    const files = verifyAssets(arg, version);
    const sums = files
      .map((name) => {
        const digest = crypto
          .createHash("sha256")
          .update(fs.readFileSync(path.join(arg, name)))
          .digest("hex");
        return `${digest}  ${name}\n`;
      })
      .join("");
    fs.writeFileSync(path.join(arg, "SHA256SUMS.txt"), sums);
    console.log(`Verified ${files.length} installers; wrote SHA256SUMS.txt`);
  } else
    throw new Error("Use verify-version TAG, draft-payload TAG or verify-assets DIRECTORY VERSION");
}
module.exports = { verifyVersion, verifyAssets, draftPayload };
