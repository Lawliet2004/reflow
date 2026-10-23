# Model-free speech runtime packs

Release preparation now builds a separate pack for each advertised target, engine profile and acceleration variant. The installer remains small; speech and writing weights are separate downloads. The checked-in `model-runtime/runtime-packs.json` is intentionally empty. The release workflow replaces it with the catalog produced by that same workflow before compiling desktop installers. A release build without a matching catalog refuses managed setup with an update/recovery message; a debug build can still use the pinned index-based development installer.

The catalog is compiled into the app. It pins each archive's SHA-256, size, target, profile, variant, dependency-lock hashes, Python archive hash and source commit. There is no floating remote catalog. Existing healthy environments and explicit custom interpreters can still be reused after their health checks.

Each pack contains:

- The pinned, unmodified python-build-standalone archive, including its license files.
- A complete wheelhouse for the chosen CPython 3.12 target/profile, including pinned pip.
- The checked-in hashed requirements and pip locks.
- Per-file sizes and hashes, package license/notice inventory and build-check results.

It contains no model weights, user settings, transcripts, populated virtual environment or absolute environment paths. The builder uses binary wheels and hash-checking. Installation uses `--no-index --find-links` against the extracted wheelhouse. These controls follow pip's [download workflow](https://pip.pypa.io/en/stable/cli/pip_download/) and [secure-install guidance](https://pip.pypa.io/en/stable/topics/secure-installs/).

## Native build and release order

`scripts/runtime-packs.py matrix` derives the 18 advertised combinations from the locks: Windows x64 (7), Linux x64 (7), Apple Silicon (3), Intel Mac (Zipformer only). Linux ARM64 locks remain available for development but do not expand the advertised support matrix.

On the matching native host, for example:

```sh
python scripts/runtime-packs.py build x86_64-unknown-linux-gnu zipformer none \
  --output runtime-packs --cache /absolute/path/to/build-cache --commit "$TAGGED_COMMIT"
```

The builder downloads only reviewed Python/wheel artifacts, creates a disposable local environment, installs the wheelhouse with socket connections denied, runs `pip check`, imports the profile and executes a CPU tensor/array operation. CUDA builds still require real accelerator execution in native acceptance; an import on a CPU CI runner is insufficient. License inventory checks presence and retains notices; a maintainer must review distribution terms before release. No blanket license approval is inferred.

The release workflow uploads packs to the draft and retains their build descriptors as workflow artifacts. Catalog assembly may use `--metadata-only` **only for those trusted descriptors from the same workflow**. This avoids downloading every multi-GB pack before compilation. Complete target/profile coverage, source identity and all lock/Python hashes are still checked. Publication downloads and rehashes the actual archives using the strict default command, compares the resulting catalog byte-for-byte with the embedded catalog, then verifies installer provenance and native acceptance. `--allow-partial` is solely for isolated local tests; it is absent from release commands.

Installer build manifests bind the embedded catalog hash. Native acceptance binds each installer and tested runtime pack hash, and requires license review. Changing a pack, catalog or installer invalidates the corresponding evidence. No release was published during implementation. The full native workflow, runner storage requirements, accelerator execution and signed/quarantined runtime behavior remain unverified here.

## Installation and recovery

The app uses the existing owned setup attempt and interruptible download verifier. A complete cached pack can be verified locally while offline; a missing or corrupt cache cannot trigger a network fallback in offline mode. In offline mode, the setup UI offers **Try cached dependencies** without changing the network preference. A separate **Allow downloads and install dependencies** action explicitly enables downloads. A missing or damaged cache leaves offline mode unchanged and both recovery actions available. Component tests exercise these paths; a real packaged UI journey still needs native acceptance.

The app rejects mismatched profiles/locks, missing/duplicate payloads, links, traversal and reserved device names, excess size and corrupt hashes. Extraction checks cancellation between bounded reads. It creates Python and the environment at their final generation paths, installs locally, checks actual imports/device operations, `pip check` and the interpreter version, writes the receipt, then switches the active pointer. Failed/cancelled repairs preserve the incumbent. Superseded generations are retained; a complete lifecycle-aware reclamation policy remains outstanding.

Setup subprocesses use bounded diagnostic spools and deadlines rather than output-pipe EOF as proof of exit. Cancel/offline checks continue even after a process closes its output. Quiet stages, inherited handles and output floods cannot create blocked reader threads. Unix cleanup signals the owned process group directly; Windows uses the existing process-tree termination path and still needs native cancellation/descendant evidence.

The archive remains in the verified cache for repair. Extracted wheel payloads are removed after installation. The installed environment retains `reflow-licenses.json` and a receipt with the pack digest. Model download is admitted only after runtime health verification.

## Local proof and remaining coverage

The [Linux Zipformer verification](verification/runtime-pack-linux-2026-10-23/README.md) records a real 107,344,553-byte pack and production Rust installer runs. It installed offline into fresh, isolated app data in 19.613 seconds initially, 20.057 seconds with bounded installer probes, 19.011 seconds with a simulated AppImage environment and invalid inherited `PYTHONHOME`, 17.924 seconds after the Rust policy-write failure fix, and 16.476 seconds with the shared atomic writer and cancellation/activation boundary, checked imports, cancelled a repair, rejected corrupt/missing caches, retried offline, preserved the incumbent and recorded zero network journal entries or model downloads. These runs establish dependency installation and recovery on this host. The AppImage environment is simulated; it is not an actual AppImage installation test. It does not establish microphone recognition, external insertion, clean installer behavior or another platform/profile.

To reproduce that opt-in check after building a local pack and test catalog:

```sh
python scripts/runtime-packs.py catalog /absolute/path/to/runtime-packs v0.5.0 \
  Lawliet2004/reflow "$SOURCE_COMMIT" /absolute/path/to/runtime-packs/runtime-packs.json --allow-partial
REFLOW_TEST_RUNTIME_PACK_CATALOG=/absolute/path/to/runtime-packs/runtime-packs.json \
  cargo test --locked --manifest-path src-tauri/Cargo.toml --lib \
  cached_runtime_pack_installs_offline_and_failed_repairs_preserve_it -- --ignored --nocapture --test-threads=1
```

Run this environment-mutating opt-in test alone. It redirects app data to a unique temporary directory, uses the existing archive, creates no models and removes only its scratch directory. The local pack is deliberately not embedded in the checked-in catalog or represented as a published artifact.

The [Linux Qwen CPU checkpoint](verification/runtime-pack-qwen-linux-2026-10-23/README.md) adds a real 308,489,808-byte pack, concrete ASR API readiness checks and offline damaged-processor repair. Its build fixed vendored-metadata handling and retains a pinned Tokenizers source notice when the exact reviewed wheel omits it. Linux tmpfs and Unix symlink storage checks now use the appropriate filesystem. The native accelerator/platform/distribution-review requirements above remain.

The [Linux Phonon CPU checkpoint](verification/runtime-pack-phonon-linux-2026-10-23/README.md) adds concrete engine/Parakeet API verification in the builder, production health check and sidecar initialization. Its 349,482,063-byte pack passes offline installation, damaged-API rejection and repair. Actual sidecar decoding also passes with a fresh dependency environment and locally copied existing weights, in int8 and forced fp32 modes. No weights were downloaded.

`Desktop validation` builds four native preview installers and the 18 advertised dependency combinations in project CI. Its read-only repository permission and artifact uploads do not create a release or a downloadable release catalog. Pack archives stay temporary; metadata and license inventories are retained for review. CUDA import checks on hosted runners do not prove accelerator execution. Physical tests are deferred by the user; build results must be reported separately.
