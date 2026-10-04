"""Pinned Phonon-2 adapter. CPU kernels come from Fermion; CUDA uses HF Parakeet.

Portions derived from Fermion Research's Apache-2.0 runtime, Copyright 2026
Fermion Research. Reflow adds verified extraction and a CUDA reference decoder.
See licenses/Fermion-Apache-2.0.txt. Weights are CC-BY-4.0; see docs/phonon-2.md.
"""
import hashlib
import json
import os
import pathlib
import shutil
import tarfile

MODEL_ID = "phonon-2"
REVISION = "ca1bef26bcd8ef4a7e16d0636d8a77bb25e298ee"
ARCHIVE = "phonon-2.bps.tar.zst"
ARCHIVE_SHA256 = "98125795b6dda72f5c6eee9ba33d19815df65dcb18b50a357bf9f73c9935309e"
CONTAINER_SHA256 = "4b6bfa3a12cc3c4e0a54f2ab3ec4ca7a842b09e5c7ecfc8e7ca0ac6cc8c11468"


def require_dependencies():
    try:
        import fermion  # noqa: F401
        import zstandard  # noqa: F401
        from importlib.metadata import version
        if version("fermion-research") != "0.2.7":
            raise ImportError("Reflow requires fermion-research==0.2.7")
    except ImportError as error:
        raise RuntimeError(
            "Install the Phonon runtime with: python -m pip install -r "
            "model-runtime/requirements-phonon.txt (beside Reflow's runtime)."
        ) from error


def digest(path):
    with open(path, "rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def unpack_model(archive, destination):
    """Join byte planes, verify each member, and publish an atomic model tree."""
    import zstandard
    from fermion._speech._engine import load
    join_file = load("package_release_bps").join_file
    destination = pathlib.Path(destination)
    staging = destination.with_name(destination.name + ".partial")
    if staging.exists():
        shutil.rmtree(staging)
    staging.mkdir(parents=True)
    try:
        rows = None
        written = set()
        with open(archive, "rb") as source, zstandard.ZstdDecompressor().stream_reader(source) as stream:
            with tarfile.open(fileobj=stream, mode="r|") as tar:
                for member in tar:
                    if not member.isfile():
                        raise ValueError("Phonon archive contains a non-file path")
                    path = pathlib.PurePosixPath(member.name)
                    if path.is_absolute() or ".." in path.parts or "\\" in member.name or ":" in member.name:
                        raise ValueError("Unsafe Phonon archive path")
                    data = tar.extractfile(member).read()
                    if member.name == "bps_manifest.json":
                        if rows is not None:
                            raise ValueError("Duplicate Phonon archive manifest")
                        rows = {row["path"]: row for row in json.loads(data)["files"]}
                        continue
                    if rows is None:
                        raise ValueError("Phonon archive manifest must come first")
                    relative = member.name.removesuffix(".bps")
                    if relative in written or relative not in rows:
                        raise ValueError("Unexpected Phonon archive path")
                    row = rows[relative]
                    if "transform" in row:
                        data = join_file(data, row["transform"])
                    if len(data) != row["original_bytes"] or hashlib.sha256(data).hexdigest() != row["original_sha256"]:
                        raise ValueError(f"Phonon checksum mismatch for {relative}")
                    target = staging / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(data)
                    written.add(relative)
        if rows is None or written != set(rows):
            raise ValueError("Incomplete Phonon archive")
        if destination.exists():
            shutil.rmtree(destination)
        staging.rename(destination)
    finally:
        if staging.exists():
            shutil.rmtree(staging)


class PhononModel:
    model_id = MODEL_ID

    def __init__(self, model_dir, device, cancel_event):
        require_dependencies()
        root = pathlib.Path(model_dir)
        archive = root / ARCHIVE
        if not archive.is_file():
            raise FileNotFoundError("Phonon-2 weights are missing. Download them in Model settings.")
        if digest(archive) != ARCHIVE_SHA256:
            raise ValueError("Phonon-2 archive SHA-256 mismatch. Remove and download the model again.")
        unpacked = root / "unpacked"
        if not (unpacked / "model.fermion").is_file():
            unpack_model(archive, unpacked)
        if digest(unpacked / "model.fermion") != CONTAINER_SHA256:
            raise ValueError("Phonon-2 container SHA-256 mismatch. Remove and download the model again.")
        self.device = device
        self.cancel_event = cancel_event
        # No Hub calls or remote Python code: only the verified local container.
        from fermion._speech import engine_phonon2_cpu as engine
        if device == "cpu":
            self.speech = engine.load(unpacked, profile="five-value", backend="phonon2-five-value", quiet=True)
            self.precision = "int8" if (self.speech.decode.get("packed") or {}).get("modules") else "fp32"
            self.runtime = self.speech.describe()["runtime"]
        elif device == "cuda":
            import torch
            from transformers import ParakeetForTDT, ParakeetTDTConfig
            weights, _, _, _ = engine._read_container(unpacked / "model.fermion")
            with torch.device("meta"):
                self.model = ParakeetForTDT(ParakeetTDTConfig(**engine.HF_CONFIG))
            self.model.load_state_dict(weights, strict=True, assign=True)
            positions = self.model.encoder.encode_positions
            self.model.encoder.encode_positions = type(positions)(positions.config)
            del weights
            self.model = self.model.eval().to("cuda")
            self.frontend = engine.Phonon2CpuSpeechModel
            self._window = torch.hann_window(engine.WIN)
            self._melf = torch.from_numpy(engine.mel_filters())
            config = json.loads((unpacked / "config.json").read_text())
            self.vocab = config.get("vocabulary") or config["joint"]["vocabulary"]
            self.special = engine._special
            self.precision = "fp32"
            self.runtime = "CUDA Parakeet TDT"
        else:
            raise ValueError(f"Unsupported Phonon device: {device}")

    def transcribe(self, samples):
        if self.cancel_event.is_set():
            return ""
        if self.device == "cpu":
            text, _, _ = self.speech.transcribe_array(samples)
        else:
            ids = self._decode_cuda(samples)
            text = "".join(self.vocab[i] for i in ids if i < len(self.vocab) and not self.special(self.vocab[i])).replace("▁", " ").strip()
        return "" if self.cancel_event.is_set() else text

    def _decode_cuda(self, samples):
        """Fermion's greedy TDT reference loop, with all model tensors on CUDA.

        Token logits and duration logits are distinct. Generic HF generate in
        transformers 5.15.1 can choose a duration index as an embedding token;
        explicitly splitting the two heads avoids that out-of-range CUDA fault.
        Based on fermion/_speech/engine_phonon2_cpu.py (Apache-2.0).
        """
        import torch
        model = self.model
        config = model.config
        blank = config.blank_token_id
        vocab_size = config.vocab_size
        ids = []
        with torch.inference_mode():
            features, mask = self.frontend._log_mel(self, samples)
            encoded = model.encoder(input_features=features.to("cuda"), attention_mask=mask.to("cuda")).last_hidden_state
            length = int(model._get_subsampling_output_length(mask.sum(-1))[0])
            projected = model.encoder_projector(encoded)[0]
            hidden = torch.zeros(model.decoder.lstm.num_layers, 1, model.decoder.lstm.hidden_size, device="cuda")
            cell = torch.zeros_like(hidden)
            frame, last, symbols = 0, blank, 0
            for _ in range(config.max_symbols_per_step * length + 16):
                if self.cancel_event.is_set():
                    return []
                if frame >= length:
                    return ids
                embedded = model.decoder.embedding(torch.tensor([[last]], device="cuda"))
                output, (next_hidden, next_cell) = model.decoder.lstm(embedded, (hidden, cell))
                decoded = model.decoder.decoder_projector(output[0])
                logits = model.joint.head(model.joint.activation(projected[frame][None] + decoded))[0]
                token = int(logits[:vocab_size].argmax())
                duration = config.durations[int(logits[vocab_size:].argmax())]
                if token == blank and duration == 0:
                    duration = 1
                if token != blank:
                    ids.append(token)
                    last, hidden, cell = token, next_hidden, next_cell
                if duration == 0:
                    symbols += 1
                    if symbols >= config.max_symbols_per_step:
                        duration, symbols = 1, 0
                else:
                    symbols = 0
                frame += duration
        raise RuntimeError("Phonon CUDA decoder exceeded its frame bound; transcript discarded.")
