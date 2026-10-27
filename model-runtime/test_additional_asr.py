"""Integrity and language boundaries for the new local model adapters."""
import hashlib
import pathlib
import sys
import tempfile
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import patch, Mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
import qwen3_asr_runtime as runtime
import indic_runtime
import parakeet_runtime


class AdditionalAsrTests(unittest.TestCase):
    def test_indic_loads_with_2600_mib_free_beside_writing_model(self):
        cuda = Mock()
        cuda.mem_get_info.return_value = (2600 * 1024**2, 4096 * 1024**2)
        cuda.memory_reserved.return_value = 1500 * 1024**2
        with patch.object(runtime, "verify_weight_files"), \
                patch.object(runtime, "pick_device", return_value="cuda"), \
                patch.object(runtime, "admit_gpu_host_load"), \
                patch.object(runtime, "_unload_model_blocking"), \
                patch.object(runtime, "_torch", return_value=SimpleNamespace(cuda=cuda)), \
                patch.object(runtime, "STATE", runtime.RuntimeState()), \
                patch.object(runtime, "_MODEL", None), \
                patch.object(indic_runtime, "IndicModel") as loader:
            result = runtime.load_model_blocking("unused", "cuda", "auto", "indic-transcribe-8bit")
            self.assertEqual(result["status"], "ok", result)
            loader.assert_called_once()

    def test_indic_low_memory_reports_measured_free_capacity(self):
        cuda = Mock()
        cuda.mem_get_info.return_value = (1800 * 1024**2, 4096 * 1024**2)
        with patch.object(runtime, "verify_weight_files"), \
                patch.object(runtime, "pick_device", return_value="cuda"), \
                patch.object(runtime, "admit_gpu_host_load"), \
                patch.object(runtime, "_unload_model_blocking"), \
                patch.object(runtime, "_torch", return_value=SimpleNamespace(cuda=cuda)), \
                patch.object(runtime, "STATE", runtime.RuntimeState()), \
                patch.object(indic_runtime, "IndicModel") as loader:
            result = runtime.load_model_blocking("unused", "cuda", "auto", "indic-transcribe-8bit")
            self.assertEqual(result["status"], "error")
            self.assertIn("1800 MiB is available", result["error"])
            loader.assert_not_called()

    def test_corrupt_code_is_rejected_before_loading_any_custom_module(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "modeling_indic_canary.py"
            path.write_bytes(b"altered custom code")
            manifest = {"files": [{"filename": path.name, "sha256": hashlib.sha256(b"reviewed").hexdigest()}]}
            with patch.object(runtime, "additional_model_manifest", return_value=manifest), \
                    patch.object(runtime, "STATE", runtime.RuntimeState()), \
                    patch.object(indic_runtime, "IndicModel") as loader:
                result = runtime.load_model_blocking(folder, "cuda", "auto", "indic-transcribe-8bit")
                self.assertEqual(result["status"], "error")
                self.assertIn("SHA-256 mismatch", result["error"])
                loader.assert_not_called()

    def test_indic_rejects_cpu_without_silently_loading_qwen(self):
        with patch.object(runtime, "verify_weight_files"), \
                patch.object(runtime, "pick_device", return_value="cpu"), \
                patch.object(runtime, "STATE", runtime.RuntimeState()), \
                patch.object(indic_runtime, "IndicModel") as loader:
            result = runtime.load_model_blocking("unused", "cpu", "auto", "indic-transcribe-8bit")
            self.assertEqual(result["status"], "error")
            self.assertIn("NVIDIA GPU", result["error"])
            loader.assert_not_called()

    def test_missing_indic_dependencies_prevent_model_transfer(self):
        with patch.object(indic_runtime, "require_dependencies", side_effect=RuntimeError("missing dependencies")), \
                patch.object(runtime, "download_snapshot_with_retry") as download:
            result = runtime.start_install("unused", "unused", "indic-transcribe-8bit")
            self.assertEqual(result["status"], "error")
            download.assert_not_called()

    def test_indic_languages_are_selected_per_model_and_preserved_for_decoding(self):
        with patch.object(runtime, "_MODEL", SimpleNamespace(model_id="indic-transcribe-8bit")), \
                patch.object(runtime, "STATE", runtime.RuntimeState()):
            result = runtime.handle({"cmd": "start_stream", "language": "bn"})
            self.assertEqual(result["status"], "ok")
            self.assertEqual(runtime.STATE.stream_language, "Bengali")
        with patch.object(runtime, "_MODEL", SimpleNamespace(model_id="0.6b")), \
                patch.object(runtime, "STATE", runtime.RuntimeState()):
            self.assertEqual(runtime.handle({"cmd": "start_stream", "language": "bn"})["status"], "error")

    def test_cancelled_indic_decode_discards_partial_text(self):
        import numpy as np
        model = indic_runtime.IndicModel.__new__(indic_runtime.IndicModel)
        model.cancel_event = threading.Event()
        model.cancel_event.set()
        model.languages = {"hi"}
        self.assertEqual(model.transcribe(np.ones(16000, dtype=np.float32), "hi"), "")

    def test_parakeet_decoder_never_embeds_a_duration_logit(self):
        import torch
        model = Mock()
        model.config = SimpleNamespace(blank_token_id=2, vocab_size=3, durations=[0, 1], max_symbols_per_step=2)
        model.parameters.return_value = iter([torch.ones(1)])
        model.encoder.return_value.last_hidden_state = torch.ones(1, 2, 1)
        model._get_subsampling_output_length.return_value = torch.tensor([2])
        model.encoder_projector.return_value = torch.ones(1, 2, 1)
        model.decoder.lstm.num_layers = 1
        model.decoder.lstm.hidden_size = 1
        model.decoder.embedding.return_value = torch.ones(1, 1, 1)
        model.decoder.decoder_projector.return_value = torch.ones(1, 1)
        model.decoder.lstm.return_value = (torch.ones(1, 1, 1), (torch.ones(1, 1, 1), torch.ones(1, 1, 1)))
        # The largest logit is a duration, whose index is not a valid token.
        model.joint.head.return_value = torch.tensor([[0., 1., 2., 3., 10.]])
        result = parakeet_runtime.decode_tdt(model, torch.ones(1, 2, 1), torch.ones(1, 2), "cpu", threading.Event())
        self.assertEqual(result, [])
        self.assertTrue(all(int(call.args[0][0, 0]) == 2 for call in model.decoder.embedding.call_args_list))


if __name__ == "__main__":
    unittest.main()
