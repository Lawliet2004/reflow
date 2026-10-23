# Native release acceptance

The release workflow leaves the release draft unpublished when native acceptance evidence is absent, incomplete, stale, or does not match its installer and runtime pack. A successful build and `--status` are insufficient. It builds the [runtime packs](runtime-packs.md) before desktop compilation, then verifies the actual archives and embedded catalog at publication. This gate does not manufacture physical-device evidence.

For the exact tagged source commit, collect results on clean Windows x64, Linux x64, macOS Apple Silicon and macOS Intel machines. Preserve the current macOS preview status until these checks pass. Linux requires X11, KDE Wayland and GNOME Wayland. Every installer format and every managed dependency profile currently shipped for that target must have a separate run; profiles are read from `model-runtime/locks/<target>-*.txt`. Intel macOS currently has only the English Zipformer profile. Native ASR, additional architectures and other advertised accelerators also need their own acceptance before support claims expand.

Upload `acceptance-windows.json`, `acceptance-linux.json`, `acceptance-macos-arm64.json` and `acceptance-macos-x64.json` to the existing draft release, alongside their referenced logs or screenshots. Each document has this shape; the example is incomplete and will fail verification:

```json
{
  "schema": 1,
  "version": "0.6.0",
  "source_commit": "REPLACE_WITH_THE_40_CHARACTER_TAGGED_COMMIT",
  "platform": "linux",
  "runs": [
    {
      "installer": "Reflow_0.6.0_amd64.AppImage",
      "installer_sha256": "REPLACE_WITH_THE_INSTALLER_SHA256",
      "runtime_pack_sha256": "REPLACE_WITH_THE_TESTED_PACK_SHA256_FROM_THE_EMBEDDED_CATALOG",
      "session": "wayland-kde",
      "profile": "zipformer-none",
      "clean_profile": true,
      "no_system_python": true,
      "os_version": "Record the actual OS and desktop versions",
      "hardware": "Record CPU, RAM, GPU and driver versions",
      "loaded_device": "Record the actual ASR device, not GPU detection alone",
      "tested_at": "2026-10-23T00:00:00Z",
      "checks": {},
      "evidence_files": [
        {
          "name": "acceptance-linux-zipformer-wayland-kde.log",
          "sha256": "REPLACE_WITH_THE_EVIDENCE_FILE_SHA256"
        }
      ]
    }
  ]
}
```

`session` is `native` for Windows/macOS. `checks` must contain all 13 keys exported as `CHECKS` in `scripts/acceptance.cjs`, each with the value `pass`, including review of the pack's license inventory. A denied permission must be retried successfully. A shortcut or insertion fallback must be explicitly selected and recorded; do not label a copied transcript as successful external insertion. Record actual dependency setup, model recognition, quit/restart, offline operation, optional writing setup, and failed-repair recovery. Signature/quarantine checks require the actual shipped artifact and downloaded runtime to install without an unexplained security workaround. Include timing, peak RAM/VRAM/storage, runtime receipts, driver/device and any limitations in the attachments.

Attachments must be ordinary files with the platform prefix and `.log`, `.txt`, `.json` or `.png` extensions. Their hashes are verified. Tests use explicitly synthetic fixtures only to test this verifier; they are not acceptance evidence.

Validate downloaded release assets locally:

```sh
python3 scripts/runtime-packs.py catalog release-downloads v0.6.0 Lawliet2004/reflow \
  "$TAGGED_COMMIT" verified-runtime-packs.json
cmp release-downloads/runtime-packs.json verified-runtime-packs.json
node scripts/release.cjs verify-assets release-downloads 0.6.0
node scripts/acceptance.cjs release-downloads 0.6.0 "$TAGGED_COMMIT"
```

After collecting real evidence, rerun the failed publication job. It verifies the same source commit and artifacts before publishing. Do not rebuild the installers underneath existing evidence; changed bytes require new acceptance.
