# Airplane mode and transferring an installation

Reflow starts with Offline mode on. Installed speech and writing models, local
inference, history, file transcription, notes and the separately enabled local
API continue to work. Missing artifacts return `Offline mode is on` without
starting a download. Complete cached, hash-verified speech runtime packs can be
installed or repaired locally by the backend; this does not yet establish the
whole offline setup UI journey. Calibration
uses installed weights and does not download anything.

Turn Offline mode off in Advanced settings only when you want to install a
model or obtain a missing runtime. Release speech setup uses a complete
[runtime pack](runtime-packs.md) with a local wheelhouse. Each HTTPS request is limited to the pinned GitHub,
Hugging Face and artifact CDN hosts. Managed package setup additionally allows only pypi.org, files.pythonhosted.org, download.pytorch.org and download-r2.pytorch.org over HTTPS. It uses complete platform/profile locks with artifact hashes, binary wheels, an explicit index policy and no inherited pip configuration. System HTTPS proxy settings are honored while destination checks remain active. Turning Offline mode on during a transfer
stops further body reads. Partial downloads can be resumed later.

The read-only network journal records host, received body bytes and UTC timestamp
in `<data directory>/network-journal.jsonl`. It includes request failures (zero
bytes) and redirects. Managed pip also journals received payload bytes through its guarded transport; completed cache hits create no outbound traffic entry. A force-killed process can lose the final in-flight request tally, so the journal is not a packet capture or a complete billing counter. It excludes URLs, query strings, tokens, request content
and private file paths. Local speech/writing inference and incoming local API
requests are not outbound internet connections and do not appear in this list.
The view reads at most the newest 1,000 entries and 2 MiB of the journal.

To move weights to an air-gapped machine:

1. On a connected machine, disable Offline mode and install the desired weights
   from Reflow's model settings. Re-enable Offline mode after installation.
2. Copy the installed model directory from `<data directory>/models` to removable
   storage. For a writing model, its single `.gguf` file is sufficient. Native
   ASR also requires the matching `mmproj-*.gguf` in the same source folder.
   Python ASR requires its full model directory, including `config.json` and its
   tokenizer/processor metadata.
3. On the destination, use Advanced settings' model import and select the file
   or directory. Reflow recognizes the pinned SHA-256 digest, verifies every
   manifest weight and companion, copies into a staging directory, verifies the
   copied weight again and installs it. Unknown or corrupt bytes are rejected.
   Installed models are not overwritten; remove an incomplete or old install
   first if necessary. Python metadata must be ordinary, bounded files and
   metadata requesting remote executable code is rejected.
4. Choose the imported model in model settings. Copying weights does not copy
   platform runtime binaries; the destination still needs its matching speech
   or writing runtime installed separately. The bundled manifests describe
   supported weights; arbitrary GGUFs and incomplete directories are rejected.

Advanced settings' JSON config export transfers writing preferences, shortcuts,
modes, snippets, dictionary terms and replacements. The bundle includes both
bundle and settings schema versions and is validated before applying. Merge
updates matching IDs and keeps other local entries; Replace replaces the
portable lists. Matching modes with a command or file-output action keep their
entire trusted local configuration, including enablement, triggers and instructions.
When such modes remain, the local default mode and trigger-enable preference also
stay local so importing preferences cannot activate those actions. Both retain
machine placement, devices, network/API permissions,
credentials, encryption choices and local filesystem paths. Files are created
without overwriting an existing export.

Shell and file-output actions are changed to Paste in an exported mode. A bundle
containing either action is rejected on import; configure those actions locally
after reviewing them. This prevents importing a config from starting an
arbitrary shell command or writing to a path belonging to another machine.

The Python sidecar uses the pinned Hugging Face client's public HTTP factory and
HTTPX transport interfaces to enforce the same policy and count downloaded body
bytes. Local model loading uses `local_files_only=True` and disables remote code.
Xet transfers and SDK telemetry are disabled so neither can bypass the journal.
See [Hugging Face HTTP backend documentation](https://huggingface.co/docs/huggingface_hub/package_reference/utilities#configuring-the-http-backend)
and [HTTPX transports](https://www.python-httpx.org/advanced/transports/).

The standalone runtime smoke tool performs no downloads by default. Its optional
live test also reads Reflow's persisted Offline mode and uses the guarded,
journaled download boundary; the environment opt-in does not override Airplane
mode.
