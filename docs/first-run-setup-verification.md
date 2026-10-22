# First-run dependency setup

Reflow now offers a dependency-only **Download all dependencies** action before speech-model selection. **Download and set up** remains available to run both steps. No system Python, shell activation, compiler, or manual pip commands are required for the managed speech environment. Building Reflow from source still requires the development prerequisites; packaged-app users do not need those tools.

## What is checked

- Operating system and architecture, processor name, thread count, available memory, graphics adapters, and storage on the app-data filesystem.
- AVX2 instruction support on x86 processors, with a suggestion to try Zipformer on older hardware. The check does not guess CPU age or promise a benchmark result.
- NVIDIA adapter compute capability and maximum CUDA version supported by its driver. The installer checks the adapter targeted by the speech runtime, rather than choosing a different adapter's capabilities.
- CUDA 13 packages for Turing or newer with a compatible driver; CUDA 12.6 packages for supported older x86 NVIDIA hardware/drivers; CPU packages if compatibility cannot be verified.
- Intel macOS is limited to the supported Zipformer dependency set. AMD adapters are reported honestly: automatic AMD speech acceleration is not supported yet, and the managed Python runtime uses CPU packages.

The NVIDIA selection follows the [PyTorch support matrix](https://github.com/pytorch/pytorch/blob/main/RELEASE.md) and [NVIDIA driver compatibility guidance](https://docs.nvidia.com/deploy/cuda-compatibility/minor-version-compatibility.html). System graphics drivers are not modified by Reflow.

## Installation behavior

A pinned interpreter is downloaded and checksum-verified, then an isolated environment is created inside Reflow's data directory. Its private pip is upgraded to a pinned version with resumable downloads. Only binary packages are accepted, so setup never asks a non-developer to install a compiler. Completed downloads are cached outside the disposable environment. Download stages display progress; long package downloads report bytes for the current package rather than a misleading overall percentage.

The final check imports all required speech libraries and executes a CPU tensor operation. GPU installations additionally execute and synchronize a tensor operation on the NVIDIA GPU before a completion receipt is written. Cancellation and failures do not produce a successful receipt. A custom interpreter is preserved. Installation holds the dictation-operation guard while replacing the environment and stops the sidecar before replacing its files.

The frontend subscribes before starting setup, joins an already-running installer, and waits for verified completion before requesting a model. A dependency-only installation is reused even if the earlier model-status error has not refreshed yet. Airplane-mode consent must be saved successfully before downloading. Storage is checked again immediately before setup. Unsupported model choices are disabled on Intel Macs.

## Verification on 22 October 2026

The commit was checked in an isolated source snapshot so unrelated local shortcut work was excluded.

- Frontend: production build, ESLint, and all 293 tests passed.
- Rust: all 558 library unit tests passed (14 opt-in tests ignored); Clippy passed for all targets with warnings denied.
- UI: browser inspection of the onboarding flow and a clearly labelled hardware-report fixture at a narrow desktop window size. The mechanical UI detector reported no findings.
- Python runtime: all 39 protocol and dependency-policy tests passed.
- Installed managed environment: imports of NumPy, PyTorch, Transformers, Hugging Face Hub, httpx, sherpa-onnx, torchao, Fermion, SciPy, soundfile, safetensors, zstandard and accelerate passed. CPU and NVIDIA tensor operations passed with Python 3.12 and PyTorch 2.13.0+cu130 on an RTX 2050.
- Fresh isolated installation: interpreter checksum, extraction, environment creation and pip upgrade were exercised. The CPU wheel download timed out at the upstream PyTorch host. A further NVIDIA installation downloaded a 366 MB acceleration wheel but was stopped during the remaining slow upstream downloads. A complete fresh installation is therefore **not confirmed** by this verification. Existing downloaded artifacts were reused where available; this was not an offline bundle test.

Windows, macOS, Linux ARM, legacy NVIDIA GPUs and AMD hardware were not available for physical end-to-end testing. Their selection paths have automated coverage, but this report does not claim native verification on those systems. Full microphone-to-text recognition with a newly installed environment also remains unverified.
