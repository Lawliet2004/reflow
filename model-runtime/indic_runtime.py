# Audio frontend derived from Bodhan/NVIDIA's Apache-2.0 Indic Canary frontend.
# Copyright (c) 2020 NVIDIA CORPORATION; Copyright (c) 2026 Bodhan.
# See licenses/Indic-Apache-2.0.txt and the pinned feature_extraction source.
"""Pinned, verified local Indic Canary code with an isolated CUDA/int8 runtime."""
import importlib
import pathlib
import sys
import types

MODEL_ID = "indic-transcribe-8bit"


def require_dependencies():
    try:
        import torch, bitsandbytes, sentencepiece, soundfile  # noqa: F401
        from transformers import PreTrainedModel, GenerationMixin  # noqa: F401
        import accelerate  # noqa: F401
    except ImportError as error:
        raise RuntimeError("Indic speech dependencies are unavailable. Set up or repair speech dependencies in Reflow.") from error


class IndicModel:
    model_id = MODEL_ID
    precision = "int8"
    runtime = "Transformers Indic Canary + bitsandbytes"

    def __init__(self, model_dir, device, cancel_event):
        require_dependencies()
        import torch
        if device != "cuda" or not torch.cuda.is_available():
            raise ValueError("Indic Transcribe 8-bit requires a compatible NVIDIA GPU. Select GPU or Auto.")
        self.cancel_event = cancel_event
        # The caller checks every code/config/weight digest before this import.
        # A private package enables upstream's relative imports without sys.path
        # mutation or executing arbitrary __init__.py / remote Hub Python code.
        package = "_reflow_indic_2765ed1"
        root = str(pathlib.Path(model_dir).resolve())
        for name in list(sys.modules):
            if name == package or name.startswith(package + "."):
                del sys.modules[name]
        module = types.ModuleType(package)
        module.__path__ = [root]
        sys.modules[package] = module
        core = importlib.import_module(package + ".modeling_indic_canary")
        tokenizer = importlib.import_module(package + ".tokenization_indic_canary")
        self.lid = importlib.import_module(package + ".lid").lid_from_encoder_states
        self.model = core.IndicCanaryForConditionalGeneration.from_pretrained(
            root, device_map={"": "cuda:0"}, dtype=torch.float32,
            local_files_only=True,
        ).eval()
        self.tokenizer = tokenizer.IndicCanaryTokenizer.from_pretrained(root)
        from safetensors.torch import load_file
        buffers = load_file(str(pathlib.Path(root) / "feature_extractor.safetensors"), device="cuda:0")
        self.window, self.fb = buffers["window"], buffers["fb"]
        self.device = "cuda:0"
        import json
        metadata = json.loads(pathlib.Path(__file__).with_name("additional-models.json").read_text())
        self.languages = {item["code"] for item in metadata[MODEL_ID]["languages"]}

    def _features(self, samples):
        """NeMo frontend for Reflow's already resampled mono 16 kHz audio.

        Preserves the checkpoint's window/mel buffers, preemphasis, reflect
        padding and unbiased per-feature normalization. No audio-file decoder.
        """
        import torch
        wave = torch.as_tensor(samples, dtype=torch.float32, device=self.device)
        if wave.numel() < 16000:
            offset = round((16000 - wave.numel()) / 2)
            wave = torch.nn.functional.pad(wave, (offset, 16000 - wave.numel() - offset))
        length = wave.numel() // 160 + 1
        wave = wave.unsqueeze(0)
        wave = torch.cat((wave[:, :1], wave[:, 1:] - 0.97 * wave[:, :-1]), dim=1)
        spec = torch.stft(wave, n_fft=512, hop_length=160, win_length=400,
                          window=self.window, center=True, return_complex=True)
        magnitudes = torch.sqrt(torch.view_as_real(spec).pow(2).sum(-1)).pow(2)
        mel = torch.log(torch.matmul(self.fb, magnitudes) + 2.0 ** -24)
        mean = mel[:, :, :length].mean(dim=2, keepdim=True)
        std = mel[:, :, :length].std(dim=2, correction=1, keepdim=True) + 1e-5
        mask = torch.arange(mel.size(2), device=self.device)[None, :] < length
        return ((mel - mean) / std).masked_fill(~mask[:, None, :], 0.0), mask.long()

    def transcribe(self, samples, language=None):
        import torch
        from transformers import StoppingCriteria, StoppingCriteriaList
        event = self.cancel_event
        class Cancelled(StoppingCriteria):
            def __call__(self, input_ids, scores, **kwargs):
                return event.is_set()
        if language and language not in self.languages:
            raise ValueError(f"Indic Transcribe does not support language {language!r}.")
        texts = []
        for start in range(0, len(samples), 30 * 16000):
            if event.is_set():
                return ""
            features, mask = self._features(samples[start:start + 30 * 16000])
            with torch.inference_mode():
                encoder = None
                lang = language
                if lang is None:
                    encoder = self.model.model.encoder(features, attention_mask=mask)
                    lang = self.lid(self.model, encoder.last_hidden_state, encoder.lengths,
                                    tokenizer=self.tokenizer, topk=1)[0][0][0]
                prompt = self.tokenizer.encode_prompt(lang)
                inputs = {"input_features": features} if encoder is None else {"encoder_outputs": encoder}
                out = self.model.generate(
                    **inputs, attention_mask=mask,
                    decoder_input_ids=torch.tensor([prompt], device=self.device),
                    max_new_tokens=1000, do_sample=False,
                    stopping_criteria=StoppingCriteriaList([Cancelled()]),
                )
                if event.is_set():
                    return ""
                if len(out[0]) >= len(prompt) + 1000 and int(out[0][-1]) != self.tokenizer.eos_id:
                    raise RuntimeError("Indic decoder reached its token limit; incomplete transcript discarded.")
                tokens = self.tokenizer.strip_prompt_and_trim(out[0].tolist(), prompt)
                texts.append(self.tokenizer.decode(tokens))
        return "" if event.is_set() else " ".join(filter(None, texts))
