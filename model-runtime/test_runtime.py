"""Model-free regression tests for ASR memory use and device admission."""

import importlib.util
import io
import json
from contextlib import contextmanager
import pathlib
import queue
import sys
import tempfile
import threading
import tracemalloc
import types
import unittest
from unittest import mock

import numpy as np


spec = importlib.util.spec_from_file_location(
    "asr_runtime", pathlib.Path(__file__).with_name("qwen3_asr_runtime.py")
)
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class RuntimeTests(unittest.TestCase):
    def test_early_loading_ack_exposes_missing_models_and_worker_exceptions_in_status(self):
        original_handle = runtime.handle
        for dispatch_error in [None, FileNotFoundError("fixture dispatch failed")]:
            with self.subTest(dispatch_error=dispatch_error), tempfile.TemporaryDirectory() as directory:
                # A real protocol request passes through main's immediate ack
                # and its command worker; no model/dependency loading is needed.
                request = {"cmd": "load_model", "model_dir": directory, "id": 23}
                incoming = types.SimpleNamespace(buffer=io.BytesIO((json.dumps(request) + "\n").encode()))
                replies = []

                def dispatch(message):
                    if message.get("cmd") == "load_model" and dispatch_error is not None:
                        raise dispatch_error
                    return original_handle(message)

                with mock.patch.object(runtime, "STATE", runtime.RuntimeState()), \
                        mock.patch.object(runtime, "_cmd_queue", queue.Queue()), \
                        mock.patch.object(runtime, "_warm_imports"), \
                        mock.patch.object(runtime, "log_err"), \
                        mock.patch.object(runtime, "_write_response", side_effect=replies.append), \
                        mock.patch.object(runtime, "handle", side_effect=dispatch), \
                        mock.patch.object(sys, "stdin", incoming):
                    runtime.main()
                    workers = [thread for thread in threading.enumerate() if thread.name == "cmd-worker"]
                    for worker in workers:
                        worker.join(2)
                        self.assertFalse(worker.is_alive(), "protocol worker did not finish")
                    self.assertEqual(replies, [{"status": "loading", "id": 23}],
                                     "the completed dispatch must not duplicate the early ack")
                    status = original_handle({"cmd": "status"})
                    self.assertFalse(status["loaded"])
                    self.assertFalse(status["is_loading"])
                    self.assertEqual(status["backend"], "load failed")
                    self.assertEqual(status["device"], "none")
                    self.assertEqual(status["failure_kind"], "weights_missing")
                    self.assertIn("fixture dispatch failed" if dispatch_error else "Model not installed",
                                  status["error"])

    def test_segment_progress_counts_all_parts_without_returning_a_partial_final(self):
        samples = np.full(33 * runtime.SAMPLE_RATE, .1, dtype=np.float32)
        pcm = (samples * 32768).astype("<i2").tobytes()
        events = []
        runtime._CANCEL_EVENT.clear()
        with mock.patch.object(runtime, "preprocess_waveform", return_value=(samples, {"voiced": True})), mock.patch.object(runtime, "transcribe_blocking", return_value=("part", "en")):
            text, language, _ = runtime.transcribe_segmented(pcm, "English", None, progress=lambda completed, total: events.append((completed, total)))
        self.assertEqual(events, [(0, 2), (1, 2), (2, 2)])
        self.assertEqual(text, "part part")
        self.assertEqual(language, "en")
    def test_shared_language_catalogue_and_invalid_language_preserve_session(self):
        self.assertEqual(len(runtime.LANG_NAMES), 30)
        self.assertEqual(runtime.LANG_NAMES["yue"], "Cantonese")
        self.assertEqual(runtime.LANG_NAMES["fil"], "Filipino")
        with mock.patch.object(runtime, "STATE", runtime.RuntimeState()):
            runtime.STATE.stream_audio.extend(b"unchanged")
            self.assertEqual(runtime.handle({"cmd": "start_stream", "language": "bn"})["status"], "error")
            self.assertEqual(runtime.STATE.stream_audio, b"unchanged")

    def test_audio_ingestion_is_bounded_and_total_duration_is_retained(self):
        with mock.patch.object(runtime, "STATE", runtime.RuntimeState()), \
                mock.patch.object(runtime, "FINAL_MAX_AUDIO_S", 0.001):
            runtime.handle({"cmd": "start_stream", "language": "ja"})
            runtime.append_stream_audio(b"a" * 20)
            runtime.append_stream_audio(b"b" * 40)
            self.assertEqual(len(runtime.STATE.stream_audio), 32)
            self.assertEqual(runtime.STATE.stream_captured_bytes, 60)
            runtime.handle({"cmd": "cancel_stream"})
            runtime.append_stream_audio(b"late")
            self.assertEqual(runtime.STATE.stream_audio, b"")

    def test_eos_at_token_limit_is_not_reported_as_exhaustion(self):
        generated = np.array([[4, 5, 9]])
        self.assertFalse(runtime.decode_exhausted(generated, 3, [9, 10]))
        self.assertTrue(runtime.decode_exhausted(generated, 3, 11))
        self.assertFalse(runtime.decode_exhausted(generated, 4, 11))

    @contextmanager
    def mock_model_dependencies(self, load_error=None):
        """Exercise loading decisions without model downloads or GPU allocation."""
        model = mock.Mock()
        model.to.return_value = model
        loader = mock.Mock(return_value=model)
        if load_error is not None:
            loader.side_effect = [load_error, model]
        int4_config = mock.Mock(side_effect=lambda **kwargs: types.SimpleNamespace(**kwargs))
        modules = {
            "torch": types.SimpleNamespace(
                bfloat16="bf16", float32="fp32",
                backends=types.SimpleNamespace(
                    cuda=types.SimpleNamespace(matmul=types.SimpleNamespace()),
                    cudnn=types.SimpleNamespace(),
                ),
                set_float32_matmul_precision=lambda _: None,
            ),
            "transformers": types.SimpleNamespace(
                AutoProcessor=types.SimpleNamespace(from_pretrained=lambda _, **kwargs: types.SimpleNamespace(chat_template="template")),
                AutoModelForMultimodalLM=types.SimpleNamespace(from_pretrained=loader),
                TorchAoConfig=lambda config: config,
            ),
            "torchao": types.ModuleType("torchao"),
            "torchao.quantization": types.SimpleNamespace(Int4WeightOnlyConfig=int4_config),
        }
        with mock.patch.dict(sys.modules, modules), \
                mock.patch.object(runtime, "_MODEL", None), \
                mock.patch.object(runtime, "GraphDecoder"), \
                mock.patch.object(runtime, "_warmup_model", return_value=0.1), \
                mock.patch.object(runtime, "log_err"):
            yield loader, int4_config

    def test_cuda_int4_uses_supported_windows_packing_configuration(self):
        with self.mock_model_dependencies() as (_, int4_config):
            runtime._try_load("model", "cuda", "int4")
            self.assertEqual(int4_config.call_args.kwargs, {
                "int4_packing_format": "tile_packed_to_4d", "version": 2,
            })

    def test_cuda_int4_places_weights_on_cuda_before_packing(self):
        with self.mock_model_dependencies() as (loader, _):
            runtime._try_load("model", "cuda", "int4")
            self.assertEqual(loader.call_args.kwargs.get("device_map"), {"": "cuda"})
            self.assertTrue(loader.call_args.kwargs["local_files_only"])
            self.assertFalse(loader.call_args.kwargs["trust_remote_code"])


    def test_non_attention_load_errors_are_not_retried(self):
        for message in ("CUDA out of memory", "int4 conversion kernel unavailable"):
            with self.subTest(message=message), self.mock_model_dependencies(RuntimeError(message)) as (loader, _):
                with self.assertRaisesRegex(RuntimeError, message):
                    runtime._try_load("model", "cuda", "int4")
                self.assertEqual(loader.call_count, 1)

    def test_unsupported_sdpa_retries_with_eager_attention(self):
        for message in (
            "Qwen3ASR does not support an attention implementation through torch.nn.functional.scaled_dot_product_attention yet.",
            "SDPA is not supported for this model",
        ):
            with self.subTest(message=message), self.mock_model_dependencies(ValueError(message)) as (loader, _):
                self.assertEqual(runtime._try_load("model", "cuda", "int4"), 0.1)
                self.assertEqual([call.kwargs["attn_implementation"] for call in loader.call_args_list], ["sdpa", "eager"])

    def warmup_wrapper(self):
        parameter = types.SimpleNamespace(device=types.SimpleNamespace(type="cuda"), dtype="bf16")
        model = types.SimpleNamespace(parameters=lambda: iter([parameter]))
        return runtime.Qwen3AsrModel(model, object(), "cuda")

    def test_all_failed_warmup_shapes_reject_broken_model(self):
        with mock.patch.object(runtime, "_torch", return_value=mock.Mock()), \
                mock.patch.object(runtime, "_warm_once", side_effect=RuntimeError("int4 kernel failed")), \
                mock.patch.object(runtime, "log_err"):
            with self.assertRaises(RuntimeError):
                runtime._warmup_model(self.warmup_wrapper())

    def test_one_successful_warmup_shape_still_admits_model(self):
        with mock.patch.object(runtime, "_torch", return_value=mock.Mock()), \
                mock.patch.object(runtime, "WARMUP_SECONDS", (1.0, 8.0)), \
                mock.patch.object(runtime, "_warm_once", side_effect=[RuntimeError("shape failed"), None]), \
                mock.patch.object(runtime.time, "time", side_effect=[0.0, 1.0, 2.0]), \
                mock.patch.object(runtime, "log_err"):
            self.assertEqual(runtime._warmup_model(self.warmup_wrapper()), 1.0 / 8.0)

    def test_raw_decode_fallback_removes_language_metadata_before_delimiters(self):
        class Processor:
            def decode(self, _tokens, return_format):
                if return_format == "parsed":
                    raise ValueError("parsed output unavailable")
                return ["language Hindi<asr_text>namaste</asr_text>"]
        self.assertEqual(runtime._parse_transcript(Processor(), object()), ("namaste", "Hindi"))

    def test_frame_energy_matches_reference_with_bounded_scratch_memory(self):
        audio = np.random.default_rng(7).uniform(-0.9, 0.9, 16_000 * 60).astype(np.float32)
        tracemalloc.start()
        try:
            energies = runtime.frame_energies(audio)
            _, peak = tracemalloc.get_traced_memory()
        finally:
            tracemalloc.stop()
        expected = np.array([
            np.sqrt(np.mean(audio[i:i + runtime.FRAME_WIN].astype(np.float64) ** 2))
            for i in range(0, audio.size - runtime.FRAME_WIN + 1, runtime.FRAME_HOP)
        ], dtype=np.float32)
        np.testing.assert_allclose(energies, expected, rtol=1e-6)
        self.assertLess(peak, audio.nbytes, "overlapping windows must not be materialized")

    def run_mock_load(self, free_query):
        attempts = []
        torch_stub = types.SimpleNamespace(cuda=types.SimpleNamespace(
            memory_reserved=lambda: 0,
            mem_get_info=lambda device: (4 * 1024 ** 3, 4 * 1024 ** 3),
        ))
        def unload():
            runtime._MODEL = None
        def load(_directory, device, precision):
            attempts.append((device, precision))
            runtime._MODEL = object()
            return 0.1
        with tempfile.TemporaryDirectory() as directory:
            pathlib.Path(directory, "config.json").write_text("{}", encoding="utf-8")
            with mock.patch.object(runtime, "STATE", runtime.RuntimeState()), \
                    mock.patch.object(runtime, "_MODEL", object()), \
                    mock.patch.object(runtime, "pick_device", return_value="cuda"), \
                    mock.patch.object(runtime, "_unload_model_blocking", side_effect=unload), \
                    mock.patch.object(runtime, "_device_vram_mb", side_effect=free_query), \
                    mock.patch.object(runtime, "weights_bytes", return_value=1_200_000_000), \
                    mock.patch.object(runtime, "_try_load", side_effect=load), \
                    mock.patch.object(runtime, "_measure_spill", return_value={"spilled": False}), \
                    mock.patch.object(runtime, "log_err"), \
                    mock.patch.dict(sys.modules, {"torch": torch_stub}):
                result = runtime.load_model_blocking(directory, "cuda", "auto")
                self.assertEqual(result["status"], "ok", result)
        return attempts

    def test_reload_releases_the_incumbent_before_reading_free_vram(self):
        def free_query():
            # The incumbent leaves no capacity; unloading it admits BF16.
            free = 4096 if runtime._MODEL is None else 0
            return {"free": free, "used": 4096 - free, "total": 4096}
        self.assertEqual(self.run_mock_load(free_query), [("cuda", "auto")])

    def test_missing_nvidia_smi_uses_cuda_live_free_memory(self):
        self.assertEqual(self.run_mock_load(lambda: None), [("cuda", "auto")])


if __name__ == "__main__":
    unittest.main()
