"""Parakeet Ultra with local Transformers weights, without Photon or Hub code.

Uses the existing Fermion Parakeet mel frontend and a bounded greedy TDT
decoder. The optional Photon VAD head is unused; Reflow segments recordings.
"""
import pathlib

MODEL_ID = "parakeet-ultra"


def require_dependencies():
    from phonon_runtime import require_dependencies as check
    check()
    from tokenizers import Tokenizer  # noqa: F401


class ParakeetModel:
    model_id = MODEL_ID

    def __init__(self, model_dir, device, cancel_event):
        require_dependencies()
        import torch
        from transformers import ParakeetForTDT
        from tokenizers import Tokenizer
        from fermion._speech import engine_phonon2_cpu as engine
        self.device, self.cancel_event = device, cancel_event
        dtype = torch.bfloat16 if device == "cuda" else torch.float32
        self.model, info = ParakeetForTDT.from_pretrained(
            model_dir, local_files_only=True, dtype=dtype,
            output_loading_info=True,
        )
        if info.get("missing_keys") or any(
            not key.startswith("vad_head.") for key in info.get("unexpected_keys", [])
        ):
            raise ValueError("Parakeet Ultra weights do not match the supported Transformers graph.")
        self.model = self.model.eval().to(device)
        self.tokenizer = Tokenizer.from_file(str(pathlib.Path(model_dir) / "tokenizer.json"))
        self.frontend = engine.Phonon2CpuSpeechModel
        self._window = torch.hann_window(engine.WIN, periodic=False)
        self._melf = torch.from_numpy(engine.mel_filters())
        self.precision = "bf16" if device == "cuda" else "fp32"
        self.runtime = "Transformers Parakeet TDT"

    def transcribe(self, samples):
        import torch
        texts = []
        # Bound encoder memory and attention work for imported long recordings.
        for start in range(0, len(samples), 30 * 16000):
            if self.cancel_event.is_set():
                return ""
            features, mask = self.frontend._log_mel(self, samples[start:start + 30 * 16000])
            ids = decode_tdt(self.model, features, mask, self.device, self.cancel_event)
            texts.append(self.tokenizer.decode(ids, skip_special_tokens=True).strip())
        return "" if self.cancel_event.is_set() else " ".join(filter(None, texts))


def decode_tdt(model, features, mask, device, cancel_event):
    """Keep duration logits out of token embeddings (HF 5.15 generation bug).

    Adapted from Reflow's Phonon decoder, based on Fermion's Apache-2.0 loop.
    """
    import torch
    config = model.config
    blank, vocab_size = config.blank_token_id, config.vocab_size
    ids = []
    with torch.inference_mode():
        dtype = next(model.parameters()).dtype
        encoded = model.encoder(
            input_features=features.to(device=device, dtype=dtype),
            attention_mask=mask.to(device),
        ).last_hidden_state
        length = int(model._get_subsampling_output_length(mask.sum(-1))[0])
        projected = model.encoder_projector(encoded)[0]
        hidden = torch.zeros(model.decoder.lstm.num_layers, 1,
                             model.decoder.lstm.hidden_size, device=device, dtype=dtype)
        cell = torch.zeros_like(hidden)
        frame, last, symbols = 0, blank, 0
        for _ in range(config.max_symbols_per_step * length + 16):
            if cancel_event.is_set():
                return []
            if frame >= length:
                return ids
            embedded = model.decoder.embedding(torch.tensor([[last]], device=device))
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
    raise RuntimeError("Parakeet decoder exceeded its frame bound; transcript discarded.")
