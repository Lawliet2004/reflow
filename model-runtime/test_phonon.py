"""Phonon integration boundaries, without downloading weights."""
import importlib.util
import pathlib
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(pathlib.Path(__file__).parent))
spec = importlib.util.spec_from_file_location("runtime", pathlib.Path(__file__).with_name("qwen3_asr_runtime.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class PhononTests(unittest.TestCase):
    def test_rejects_other_languages_before_altering_recording(self):
        with mock.patch.object(runtime, "_MODEL", mock.Mock(model_id="phonon-2")), mock.patch.object(runtime, "STATE", runtime.RuntimeState()):
            runtime.STATE.stream_audio.extend(b"preserve")
            result = runtime.handle({"cmd": "start_stream", "language": "hi"})
            self.assertEqual(result["status"], "error")
            self.assertIn("English", result["error"])
            self.assertEqual(runtime.STATE.stream_audio, b"preserve")

    def test_load_dispatches_to_phonon_instead_of_qwen(self):
        with mock.patch.object(runtime, "load_phonon_blocking", return_value={"status": "ok"}) as load:
            result = runtime.load_model_blocking("phonon-dir", "cpu", "auto", "phonon-2")
            self.assertEqual(result["status"], "ok")
            load.assert_called_once_with("phonon-dir", "cpu", "auto")

    def test_missing_dependencies_explain_how_to_install(self):
        import phonon_runtime
        with mock.patch.dict(sys.modules, {"fermion": None}):
            with self.assertRaisesRegex(RuntimeError, "requirements-phonon.txt"):
                phonon_runtime.require_dependencies()

    def test_verified_install_preserves_forced_cpu_for_autoload(self):
        import hashlib
        import types
        import phonon_runtime
        payload = b"test weights"
        with tempfile.TemporaryDirectory() as root:
            archive = pathlib.Path(root) / phonon_runtime.ARCHIVE
            def download(*args, **kwargs):
                self.assertEqual(kwargs["revision"], phonon_runtime.REVISION)
                self.assertIn("NOTICE", kwargs["allow_patterns"])
                archive.write_bytes(payload)
            hub = types.SimpleNamespace(snapshot_download=download)
            def synchronous_thread(target, **kwargs):
                return mock.Mock(start=target)
            with mock.patch.dict(sys.modules, {"huggingface_hub": hub}), mock.patch.object(phonon_runtime, "require_dependencies"), mock.patch.object(runtime, "require_network_online"), mock.patch.object(runtime, "install_network_guard"), mock.patch.object(runtime.threading, "Thread", side_effect=synchronous_thread), mock.patch.object(runtime, "STATE", runtime.RuntimeState()), mock.patch.object(runtime, "start_load") as load:
                result = runtime.handle({"cmd": "install_model", "model_id": "phonon-2", "repo": "FermionResearch/Phonon-2", "model_dir": root, "device": "cpu", "precision": "int8", "revision": phonon_runtime.REVISION, "weight_files": [{"filename": phonon_runtime.ARCHIVE, "sha256": hashlib.sha256(payload).hexdigest()}]})
                self.assertEqual(result["status"], "downloading")
                self.assertIsNone(runtime.STATE.download_error)
                load.assert_called_once_with(root, "cpu", "int8", "phonon-2", None)

    def test_unpack_refuses_paths_outside_model_directory(self):
        import io
        import json
        import tarfile
        import hashlib
        import types
        import phonon_runtime
        payload = b"bad"
        manifest = {"files": [{"path": "../escape", "original_sha256": hashlib.sha256(payload).hexdigest(), "original_bytes": len(payload)}]}
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w") as tar:
            for name, data in [("bps_manifest.json", json.dumps(manifest).encode()), ("../escape", payload)]:
                info = tarfile.TarInfo(name)
                info.size = len(data)
                tar.addfile(info, io.BytesIO(data))
        with tempfile.TemporaryDirectory() as root:
            archive = pathlib.Path(root) / "model.tar.zst"
            archive.write_bytes(stream.getvalue())
            decoder = types.SimpleNamespace(ZstdDecompressor=lambda: types.SimpleNamespace(stream_reader=lambda source: source))
            engine = types.SimpleNamespace(load=lambda name: types.SimpleNamespace(join_file=lambda data, transform: data))
            with mock.patch.dict(sys.modules, {"zstandard": decoder, "fermion._speech._engine": engine}):
                with self.assertRaisesRegex(ValueError, "path"):
                    phonon_runtime.unpack_model(archive, pathlib.Path(root) / "unpacked")
            self.assertFalse((pathlib.Path(root) / "escape").exists())


if __name__ == "__main__":
    unittest.main()
