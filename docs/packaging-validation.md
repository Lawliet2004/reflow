# Packaging, process ownership and platform gates

**0.3.0 release update:** Explicitly requested macOS preview DMGs now build for both Apple Silicon and Intel. They retain the clipboard/manual-copy limitations below and use ad-hoc signing, with a microphone usage description. The local helper requires `--macos-preview` to opt into that preview; the default macOS parity gate remains. Tag releases now publish Windows, Linux, both macOS previews and a signed Android companion only after reusable CI, platform builds, APK signature verification and the complete installer/checksum inventory succeed. See [0.3.0 release notes](release-0.3.0.md) for user-facing limits. The historical validation record below describes the earlier 0.2.0 packaging slice.

Implemented 1 October 2026. These changes address the packaging/process-termination findings in the backend audit and add the missing Android and macOS compilation gates.

## Installer commands

Both platform wrappers delegate to `scripts/package.cjs`, which derives the repository root from its own location. It invokes the installed, lockfile-controlled Tauri CLI once, with an argument array and no shell. The normal Tauri `beforeBuildCommand` builds the frontend once. Python is no longer a packaging prerequisite: the optional Python sidecar source is a bundle resource, and native-ASR installations do not need a Python interpreter just to produce an installer.

```powershell
# From any working directory; replaces the previous binary-only cargo invocation.
& "C:\path\to\reflow\scripts\package_windows.ps1"
& "C:\path\to\reflow\scripts\package_windows.ps1" -DryRun
```

```bash
bash /path/to/reflow/scripts/package_linux.sh
bash /path/to/reflow/scripts/package_linux.sh --dry-run
```

Windows requests `nsis,msi`; Linux requests `deb,appimage`. Windows and Linux builds must run on their matching host. `-SkipFrontendBuild` / `--skip-frontend-build` explicitly overrides the frontend hook and requires existing `dist/index.html`; the caller is responsible for those assets being current. A custom `CARGO_TARGET_DIR` must be absolute so output verification uses the correct directory.

After a successful Tauri command, the helper requires nonempty installer files for every requested bundle format, matching the current app version and build time. It writes SHA-256 files for the installer bytes themselves. A failed build, missing format, old version, or stale artifact cannot be reported as successful packaging. Checksums detect changed bytes; they do not provide Authenticode identity.

The old `-SignBinaries` flag printed a signing claim while performing no signing. It now fails before any build with an actionable message. Certificate-store selection, timestamping, signature verification and release credential provisioning must be implemented/reviewed as a real signing configuration; this helper does not claim that work has happened. Standard Tauri Windows signing configuration can be used independently of this rejected legacy switch.

Current development prerequisites are Node 22, Rust/Cargo (declared minimum Rust 1.88), the locked npm dependencies, and the target OS's Tauri build dependencies. CI uses stable Rust; no local Rust 1.88 compiler run was performed for this packaging slice.

## Scoped shutdown

The npm `pretauri` helper now inventories executable paths and signals only a Reflow debug/release binary at this checkout's default `src-tauri/target` path. It never uses command-line substring matching, `pkill -f`, or Windows image-name termination. It verifies the executable identity again immediately before signaling a PID and tolerates processes that have exited.

Other checkouts, installed Reflow copies, unrelated processes whose command lines contain “reflow,” and processes with unreadable identities are left alone. Custom/shared Cargo output directories require an explicit manual shutdown, since the helper cannot establish checkout ownership from them. This is a narrower process-ownership rule, not an atomic guarantee against the kernel reusing a PID between inventory and signaling.

## CI and release support

- Windows runs frontend lint/tests/build, packaging/process-ownership regressions, and Rust lint/tests.
- Python preprocessing/self-tests and model-free regressions run on Windows and Linux.
- Linux retains frontend, Rust lint/tests, and packaging-contract checks.
- Android provisions JDK 17 / SDK 35 and runs recorder/session/TLS/network regressions, debug APK build and lint. The wrapper jar runs directly through Java, so the absent Unix `gradlew` launcher does not break Linux CI. Reports and the debug APK are retained as validation artifacts.
- macOS runs compilation and unit tests as a development/clipboard target. Public macOS installers are gated until native dictation parity is implemented and tested.
- Tag releases call the complete CI workflow before creating draft Windows/Linux installers. Branch pushes and pull requests retain ordinary CI; tag validation uses the reusable workflow, avoiding duplicate standalone tag runs. This code has not been pushed and these hosted jobs have not run yet.

Linux X11 injection is implemented. Wayland permissions, compositor support and available `wtype`/`ydotool` tools can require manual paste. macOS currently reports a clipboard/manual-copy fallback, lacks native text injection and active-window/startup integration, and needs a supported native hotkey. Restoring macOS installer publication requires actual implementation plus accessibility/hotkey/focus/device verification; compilation alone cannot establish parity.

## Validation and limits

- Five red tests reproduced the previous caller-directory change, pretend signing success, broad shutdown behavior, missing real bundle plan and missing macOS release gate.
- Eleven packaging/process-ownership tests pass. They exercise wrapper delegation from an unrelated directory, argument construction, failures, every-format/version/freshness checks, checksum bytes, exact executable selection and changed PID identity.
- Windows and Linux wrapper dry-runs produce the expected Tauri plans. Git Bash validates the Linux wrapper syntax. PowerShell wrapper tests intercept build commands; they do not run signing or publishing.
- Workflow YAML parses and is checked with `actionlint`; see `docs/implementation-ci-validation.log` for the final result.
- No installer was actually created, installed or signed during this slice. No live process was terminated, no GitHub workflow was dispatched and no release was published. Hosted Windows/Linux/macOS/Android validation remains to be observed after these changes are pushed.

Evidence: `docs/implementation-packaging-red.log`, `docs/implementation-packaging-tests-final.log`, `docs/implementation-windows-package-plan.json`, `docs/implementation-linux-package-plan.json`, and `docs/implementation-ci-validation.log`.

The command choices follow [Tauri's Windows installer documentation](https://v2.tauri.app/distribute/windows-installer/) and the installed Tauri CLI's `build --help`. Gradle caching follows the [official setup-gradle action](https://github.com/gradle/actions/blob/main/setup-gradle/README.md).
