"""Network boundary checks; no package installs or real outbound requests."""
import io
import json
import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import managed_pip


class ManagedPipTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.policy = Path(self.temp.name) / "network-policy.json"
        self.policy.write_text('{"offline_mode": false}')
        self.env = patch.dict(os.environ, REFLOW_NETWORK_POLICY=str(self.policy))
        self.env.start()
        self.addCleanup(self.env.stop)

    def test_sources_fail_closed_and_recheck_mid_body(self):
        for url in ("http://pypi.org/simple", "https://evil.example/", "https://user@pypi.org/", "https://pypi.org:444/"):
            with self.assertRaises(RuntimeError):
                managed_pip.check_url(url)
        body = managed_pip.JournalBody(io.BytesIO(b"abcdef"), "https://files.pythonhosted.org/private?token=secret")
        self.assertEqual(body.read(3), b"abc")
        self.policy.write_text('{"offline_mode": true}')
        with self.assertRaises(RuntimeError):
            body.read(3)
        body.close()
        record = json.loads((self.policy.parent / "network-journal.jsonl").read_text())
        self.assertEqual(record["bytes"], 3)
        self.assertEqual(record["host"], "files.pythonhosted.org")
        self.assertEqual(set(record), {"timestamp", "host", "bytes"})

    def test_adapter_accepts_cachecontrol_positional_signature(self):
        from pip._vendor.requests.adapters import HTTPAdapter
        original = HTTPAdapter.send
        response = SimpleNamespace(raw=io.BytesIO(b"package"))
        try:
            with patch.object(HTTPAdapter, "send", return_value=response) as send:
                managed_pip.install_guard()
                adapter = HTTPAdapter()
                request = SimpleNamespace(url="https://pypi.org/simple/pip/")
                actual = adapter.send(request, False, 15, True, None, {})
                send.assert_called_once_with(adapter, request, False, 15, True, None, {})
                self.assertEqual(actual.raw.read(), b"package")
                actual.raw.close()
            records = (self.policy.parent / "network-journal.jsonl").read_text().splitlines()
            self.assertEqual(len(records), 1)
            self.assertEqual(json.loads(records[0])["bytes"], 7)
        finally:
            HTTPAdapter.send = original

    def test_cuda_dependency_host_is_allowed_and_journaled(self):
        body = managed_pip.JournalBody(io.BytesIO(b"wheel"), "https://pypi.nvidia.com/nvidia-cublas/package.whl")
        self.assertEqual(body.read(), b"wheel")
        body.close()
        record = json.loads((self.policy.parent / "network-journal.jsonl").read_text())
        self.assertEqual(record["host"], "pypi.nvidia.com")
        self.assertEqual(record["bytes"], 5)
        for url in ("http://pypi.nvidia.com/", "https://pypi.nvidia.com.evil.example/",
                    "https://user@pypi.nvidia.com/", "https://pypi.nvidia.com:444/"):
            with self.assertRaises(RuntimeError):
                managed_pip.check_url(url)

    def test_rejected_source_diagnostic_omits_credentials_and_path(self):
        with self.assertRaises(RuntimeError) as caught:
            managed_pip.check_url("https://user:secret@unapproved.example/private?token=secret")
        message = str(caught.exception)
        self.assertIn("unapproved.example", message)
        for private in ("user", "secret", "private", "token"):
            self.assertNotIn(private, message)
