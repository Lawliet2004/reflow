"""Zipformer integration boundaries, without sherpa-onnx or weights."""
import importlib.util
import pathlib
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
spec = importlib.util.spec_from_file_location("runtime", pathlib.Path(__file__).with_name("qwen3_asr_runtime.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class FakeStream:
    def __init__(self):
        self.waveforms = []
        self.input_done = False

    def accept_waveform(self, rate, samples):
        assert rate == 16000
        self.waveforms.append(samples)

    def input_finished(self):
        self.input_done = True


class FakeRecognizer:
    """Scriptable stand-in for sherpa_onnx.OnlineRecognizer."""

    def __init__(self, results, endpoints):
        self.streams = []
        self.results = iter(results)
        self.endpoints = iter(endpoints)
        self.reset_calls = 0

    def create_stream(self):
        stream = FakeStream()
        self.streams.append(stream)
        return stream

    def is_ready(self, _stream):
        return False

    def decode_stream(self, _stream):
        raise AssertionError("is_ready() is False; decode must not run")

    def is_endpoint(self, _stream):
        return next(self.endpoints)

    def get_result_all(self, _stream):
        return type("Result", (), {"text": next(self.results)})

    def reset(self, _stream):
        self.reset_calls += 1


def fake_model(recognizer):
    """A ZipformerModel wired to a fake recognizer, skipping file checks."""
    import numpy as np
    from zipformer_runtime import ZipformerModel

    model = ZipformerModel.__new__(ZipformerModel)
    model.recognizer = recognizer
    model.cancel_event = threading.Event()
    model.lock = threading.RLock()
    model.stream = None
    model.committed = []
    model._head_pad = np.zeros(160, dtype=np.float32)
    return model


class ZipformerTests(unittest.TestCase):
    def test_rejects_other_languages_before_altering_recording(self):
        with mock.patch.object(runtime, "_MODEL", mock.Mock(model_id="zipformer-20m")), mock.patch.object(runtime, "STATE", runtime.RuntimeState()):
            runtime.STATE.stream_audio.extend(b"preserve")
            result = runtime.handle({"cmd": "start_stream", "language": "hi"})
            self.assertEqual(result["status"], "error")
            self.assertIn("English", result["error"])
            self.assertEqual(runtime.STATE.stream_audio, b"preserve")

    def test_load_dispatches_to_zipformer_instead_of_qwen(self):
        with mock.patch.object(runtime, "load_zipformer_blocking", return_value={"status": "ok"}) as load:
            result = runtime.load_model_blocking("zip-dir", "auto", "auto", "zipformer-20m")
            self.assertEqual(result["status"], "ok")
            load.assert_called_once_with("zip-dir", "auto", "auto")

    def test_missing_dependencies_explain_how_to_install(self):
        import zipformer_runtime
        with mock.patch.dict(sys.modules, {"sherpa_onnx": None}):
            with self.assertRaisesRegex(RuntimeError, "requirements-zipformer.txt"):
                zipformer_runtime.require_dependencies()

    def test_verified_install_downloads_only_int8_files(self):
        import hashlib
        import types
        import zipformer_runtime
        payload = b"test weights"
        with tempfile.TemporaryDirectory() as root:
            def download(*args, **kwargs):
                self.assertEqual(kwargs["revision"], zipformer_runtime.REVISION)
                for name in kwargs["allow_patterns"]:
                    self.assertNotIn(".fp32", name)
                    self.assertNotEqual(name, "*.onnx")
                for name in zipformer_runtime.FILES:
                    self.assertIn(name, kwargs["allow_patterns"])
                    (pathlib.Path(root) / name).write_bytes(payload)
            hub = types.SimpleNamespace(snapshot_download=download)
            def synchronous_thread(target, **kwargs):
                return mock.Mock(start=target)
            weight_files = [
                {"filename": name, "sha256": hashlib.sha256(payload).hexdigest()}
                for name in zipformer_runtime.FILES
            ]
            with mock.patch.dict(sys.modules, {"huggingface_hub": hub}), mock.patch.dict(sys.modules, {"zipformer_runtime": types.SimpleNamespace(require_dependencies=lambda: None)}), mock.patch.object(runtime, "require_network_online"), mock.patch.object(runtime, "install_network_guard"), mock.patch.object(runtime.threading, "Thread", side_effect=synchronous_thread), mock.patch.object(runtime, "STATE", runtime.RuntimeState()), mock.patch.object(runtime, "start_load") as load:
                result = runtime.handle({"cmd": "install_model", "model_id": "zipformer-20m", "repo": "csukuangfj/sherpa-onnx-streaming-zipformer-en-20M-2023-02-17", "model_dir": root, "device": "cpu", "precision": "int8", "revision": zipformer_runtime.REVISION, "weight_files": weight_files})
                self.assertEqual(result["status"], "downloading")
                self.assertIsNone(runtime.STATE.download_error)
                load.assert_called_once_with(root, "cpu", "int8", "zipformer-20m", None)

    def test_endpoint_commits_text_and_reset_keeps_streaming(self):
        # Call order in accept_pcm16: endpoint result, then live hypothesis;
        # finish_stream takes one final result.
        recognizer = FakeRecognizer(
            results=["hello", "", "world", "world"],
            endpoints=[True, False],
        )
        model = fake_model(recognizer)
        model.start_stream()
        pcm = b"\x00\x00" * 1600
        self.assertEqual(model.accept_pcm16(pcm), "hello")
        self.assertEqual(model.committed, ["hello"])
        self.assertEqual(recognizer.reset_calls, 1)
        # Next chunk: hypothesis extends past the committed segment.
        self.assertEqual(model.accept_pcm16(pcm), "hello world")
        self.assertEqual(model.committed, ["hello"])
        self.assertEqual(model.finish_stream(), "hello world")
        self.assertIsNone(model.stream)

    def test_finish_stream_on_silence_returns_committed_text(self):
        recognizer = FakeRecognizer(results=["tail"], endpoints=[False])
        model = fake_model(recognizer)
        model.start_stream()
        self.assertEqual(model.finish_stream(), "tail")
        # A second stop returns nothing rather than re-emitting the segment.
        self.assertEqual(model.finish_stream(), "")

    def test_cancel_drops_in_flight_state(self):
        recognizer = FakeRecognizer(results=["", "", "", ""], endpoints=iter(False for _ in range(10)))
        model = fake_model(recognizer)
        model.start_stream()
        model.accept_pcm16(b"\x00\x00" * 1600)
        model.cancel_stream()
        self.assertIsNone(model.stream)
        self.assertEqual(model.committed, [])
        self.assertEqual(model.partial_text(), "")


if __name__ == "__main__":
    unittest.main()
