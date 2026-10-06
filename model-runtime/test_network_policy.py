"""No live connections: exercise sidecar Airplane guards and journaled transport."""
import importlib.util
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

import httpx

spec = importlib.util.spec_from_file_location(
    "network_runtime", pathlib.Path(__file__).with_name("qwen3_asr_runtime.py")
)
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class NetworkPolicyTests(unittest.TestCase):
    def test_hf_weight_redirect_reaches_official_cdn_and_journals_bytes(self):
        import huggingface_hub
        for host in ("us.aws.cdn.hf.co", "us.gcp.cdn.hf.co"):
            with self.subTest(host=host), tempfile.TemporaryDirectory() as directory:
                policy = pathlib.Path(directory, "network-policy.json")
                policy.write_text('{"offline_mode":false}', encoding="utf-8")
                with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}):
                    runtime.install_network_guard()
                    responses = [
                        httpx.Response(302, headers={"location": f"https://{host}/weights?signature=secret"},
                                       stream=httpx.ByteStream(b"")),
                        httpx.Response(200, stream=httpx.ByteStream(b"weights")),
                    ]
                    with mock.patch.object(httpx.HTTPTransport, "handle_request", side_effect=responses) as send:
                        result = huggingface_hub.get_session().get("https://huggingface.co/model/resolve/pinned/weights")
                        self.assertEqual(result.content, b"weights")
                        self.assertEqual(send.call_count, 2)
                    rows = pathlib.Path(directory, "network-journal.jsonl").read_text(encoding="utf-8")
                    self.assertNotIn("secret", rows)
                    entry = json.loads(rows.splitlines()[-1])
                    self.assertEqual(entry["host"], host)
                    self.assertEqual(entry["bytes"], 7)
                huggingface_hub.close_session()

    def test_cdn_allowlist_rejects_lookalikes_and_insecure_urls(self):
        with tempfile.TemporaryDirectory() as directory:
            policy = pathlib.Path(directory, "network-policy.json")
            policy.write_text('{"offline_mode":false}', encoding="utf-8")
            with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}):
                for url in ("https://us.aws.cdn.hf.co.attacker.test/weights",
                            "https://attacker.hf.co/weights", "http://us.aws.cdn.hf.co/weights",
                            "https://us.aws.cdn.hf.co:444/weights", "https://user@us.aws.cdn.hf.co/weights"):
                    with self.subTest(url=url), self.assertRaises(RuntimeError):
                        runtime.check_download_request(url)

    def test_missing_or_invalid_policy_blocks_before_installer_starts(self):
        with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": "missing-policy-file"}):
            with mock.patch.object(runtime, "install_network_guard") as install:
                result = runtime.start_install("irrelevant", "Qwen/repo", revision="a" * 40,
                                               weight_files=[{"filename": "model", "sha256": "b" * 64}])
                self.assertEqual(result, {"status": "error", "error": "Offline mode is on"})
                install.assert_not_called()

    def test_policy_toggle_is_seen_by_existing_sidecar_and_unpinned_hosts_blocked(self):
        with tempfile.TemporaryDirectory() as directory:
            policy = pathlib.Path(directory, "network-policy.json")
            with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}):
                policy.write_text('{"offline_mode":false}', encoding="utf-8")
                self.assertEqual(runtime.check_download_request("https://huggingface.co/pinned"), "huggingface.co")
                with self.assertRaises(RuntimeError):
                    runtime.check_download_request("https://huggingface.co.attacker.test/path")
                policy.write_text('{"offline_mode":true}', encoding="utf-8")
                with self.assertRaisesRegex(RuntimeError, "Offline mode is on"):
                    runtime.check_download_request("https://huggingface.co/pinned")
                self.assertFalse(pathlib.Path(directory, "network-journal.jsonl").exists())

    def test_public_hf_transport_counts_body_without_storing_url_or_token(self):
        import huggingface_hub
        with tempfile.TemporaryDirectory() as directory:
            policy = pathlib.Path(directory, "network-policy.json")
            policy.write_text('{"offline_mode":false}', encoding="utf-8")
            response = httpx.Response(200, stream=httpx.ByteStream(b"weights"))
            with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}):
                runtime.install_network_guard()
                with mock.patch.object(httpx.HTTPTransport, "handle_request", return_value=response) as send:
                    result = huggingface_hub.get_session().get("https://huggingface.co/private-model?token=secret")
                    self.assertEqual(result.content, b"weights")
                    send.assert_called_once()
                rows = pathlib.Path(directory, "network-journal.jsonl").read_text(encoding="utf-8")
                self.assertNotIn("secret", rows)
                self.assertNotIn("private-model", rows)
                entry = json.loads(rows.strip())
                self.assertEqual(entry["host"], "huggingface.co")
                self.assertEqual(entry["bytes"], 7)
                self.assertEqual(set(entry), {"host", "bytes", "timestamp"})
                policy.write_text('{"offline_mode":true}', encoding="utf-8")
                with mock.patch.object(httpx.HTTPTransport, "handle_request") as send:
                    with self.assertRaisesRegex(RuntimeError, "Offline mode is on"):
                        huggingface_hub.get_session().get("https://huggingface.co/pinned")
                    send.assert_not_called()
        huggingface_hub.close_session()


if __name__ == "__main__":
    unittest.main()
