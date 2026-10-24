"""Model-free integrity and sherpa integration contract checks."""
import hashlib
import io
import pathlib
import sys
import tarfile
import tempfile
import threading
import unittest
from types import SimpleNamespace
from unittest.mock import patch
sys.path.insert(0, str(pathlib.Path(__file__).parent))
import omnilingual_runtime as omni


class OmnilingualTests(unittest.TestCase):
    def setUp(self):
        expected = {name: (hashlib.sha256(data).hexdigest(), len(data)) for name, data in
                    [('model.int8.onnx', b'model'), ('tokens.txt', b'token 0')]}
        self.files_patch = patch.object(omni, 'EXPECTED_FILES', expected)
        self.files_patch.start()
        self.addCleanup(self.files_patch.stop)

    def test_verified_warm_load_needs_no_archive_writes_or_free_space(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            with patch.object(omni, 'SHA256', self.archive(root)):
                omni.prepare_archive(root)
            (root / omni.ARCHIVE).unlink()
            before = (root / 'model.int8.onnx').stat().st_mtime_ns
            with patch.object(omni.tarfile, 'open', side_effect=AssertionError("must not extract")), patch.object(omni.shutil, 'disk_usage', side_effect=AssertionError("must not stage")):
                self.assertEqual(omni.prepare_archive(root), root)
            self.assertEqual((root / 'model.int8.onnx').stat().st_mtime_ns, before)

    def test_low_disk_repair_preserves_existing_files(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            (root / 'model.int8.onnx').write_bytes(b'original')
            with patch.object(omni, 'SHA256', self.archive(root)), patch.object(omni.shutil, 'disk_usage', return_value=SimpleNamespace(free=0)):
                with self.assertRaisesRegex(OSError, 'free disk space'):
                    omni.prepare_archive(root)
            self.assertEqual((root / 'model.int8.onnx').read_bytes(), b'original')
            self.assertFalse(list(root.glob('*.part')))


    def archive(self, root, symlink=False):
        path = root / omni.ARCHIVE
        with tarfile.open(path, 'w:bz2') as bundle:
            for name, data in [('model.int8.onnx', b'model'), ('tokens.txt', b'token 0')]:
                member = tarfile.TarInfo('../unsafe/' + name)
                member.size = len(data)
                if symlink:
                    member.type = tarfile.SYMTYPE
                    member.linkname = '/tmp/forbidden'
                bundle.addfile(member, io.BytesIO(data))
        return hashlib.sha256(path.read_bytes()).hexdigest()

    def test_only_named_members_copied_inside_root_and_reverified(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            sha = self.archive(root)
            with patch.object(omni, 'SHA256', sha):
                omni.prepare_archive(root)
                self.assertEqual((root / 'model.int8.onnx').read_bytes(), b'model')
                (root / 'model.int8.onnx').write_bytes(b'corrupted')
                omni.prepare_archive(root)
                self.assertEqual((root / 'model.int8.onnx').read_bytes(), b'model')
            (root / 'model.int8.onnx').unlink()
            with self.assertRaisesRegex(ValueError, 'SHA-256'):
                omni.prepare_archive(root)

    def test_links_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            with patch.object(omni, 'SHA256', self.archive(root, symlink=True)):
                with self.assertRaisesRegex(ValueError, 'regular'):
                    omni.prepare_archive(root)

    def test_real_sherpa_factory_contract_and_cancel(self):
        stream = SimpleNamespace(accept_waveform=lambda rate, samples: self.assertEqual(rate, 16000), result=SimpleNamespace(text=' नमस्ते '))
        recognizer = SimpleNamespace(create_stream=lambda: stream, decode_stream=lambda value: self.assertIs(value, stream))
        factory = unittest.mock.Mock(return_value=recognizer)
        fake = SimpleNamespace(OfflineRecognizer=SimpleNamespace(from_omnilingual_asr_ctc=factory))
        with patch.object(omni, 'require_dependencies'), patch.object(omni, 'prepare_archive', return_value=pathlib.Path('/models')), patch.dict(sys.modules, {'sherpa_onnx': fake}):
            cancelled = threading.Event()
            model = omni.OmnilingualModel('/models', 'cpu', cancelled)
            self.assertEqual(factory.call_args.kwargs['model'], '/models/model.int8.onnx')
            self.assertEqual(factory.call_args.kwargs['provider'], 'cpu')
            self.assertEqual(model.transcribe([0.1]), 'नमस्ते')
            cancelled.set()
            self.assertEqual(model.transcribe([0.1]), '')

    def test_cancelled_copy_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            event = threading.Event()
            event.set()
            with patch.object(omni, 'SHA256', self.archive(root)):
                with self.assertRaisesRegex(RuntimeError, 'cancelled'):
                    omni.prepare_archive(root, event)
            self.assertFalse(list(root.glob('*.part')))

    def test_sidecar_dispatches_without_qwen_imports(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('omni_test_runtime', pathlib.Path(__file__).with_name('qwen3_asr_runtime.py'))
        runtime = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runtime)
        with patch.object(runtime, 'load_omnilingual_blocking', return_value={'status': 'ok'}) as load:
            self.assertEqual(runtime.load_model_blocking('/models', 'cpu', 'int8', omni.MODEL_ID), {'status': 'ok'})
            load.assert_called_once_with('/models', 'cpu', 'int8')
        self.assertEqual(runtime._warm_groups(omni.MODEL_ID), {'sherpa'})


if __name__ == '__main__':
    unittest.main()
