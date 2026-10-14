"""No live connections: exercise sidecar Airplane guards and journaled transport."""
import hashlib
import importlib.util
import json
import os
import pathlib
import socket
import ssl
import subprocess
import sys
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
    def test_metadata_only_download_is_reported_as_incomplete_and_preserves_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "config.json").write_text("{}", encoding="utf-8")
            partial = root / ".cache" / "weights.incomplete"
            partial.parent.mkdir()
            partial.write_bytes(b"partial download")
            with self.assertRaisesRegex(RuntimeError, "Download is incomplete.*retry Download"):
                runtime.verify_weight_files(directory, [{"filename": "model.safetensors", "sha256": "a" * 64}])
            self.assertEqual(partial.read_bytes(), b"partial download")
            self.assertTrue((root / "config.json").is_file())

    def test_hub_timeouts_default_to_sixty_seconds_and_respect_overrides(self):
        environment = dict(os.environ)
        environment.pop("HF_HUB_DOWNLOAD_TIMEOUT", None)
        environment.pop("HF_HUB_ETAG_TIMEOUT", None)
        code = ("import sys; sys.path.insert(0, sys.argv[1]); import qwen3_asr_runtime; "
                "from huggingface_hub import constants; "
                "print(constants.HF_HUB_DOWNLOAD_TIMEOUT, constants.HF_HUB_ETAG_TIMEOUT)")
        for overrides, expected in [({}, "60 60"), ({"HF_HUB_DOWNLOAD_TIMEOUT": "75", "HF_HUB_ETAG_TIMEOUT": "80"}, "75 80")]:
            with self.subTest(overrides=overrides):
                result = subprocess.run([sys.executable, "-c", code, str(pathlib.Path(__file__).parent)],
                                        env=dict(environment, **overrides), capture_output=True,
                                        text=True, timeout=10, check=True)
                self.assertEqual(result.stdout.strip(), expected)

    def test_real_hub_snapshot_recovers_connection_and_temporary_dns_then_verifies(self):
        import huggingface_hub
        payload = b"verified fixture weights"
        digest = hashlib.sha256(payload).hexdigest()
        revision = "a" * 40
        calls = []
        def transport(request):
            calls.append(request)
            if len(calls) == 1:
                raise httpx.ConnectError("fixture connection failure", request=request)
            if len(calls) == 2:
                raise socket.gaierror(socket.EAI_AGAIN, "Temporary failure in name resolution")
            if "/api/models/" in request.url.path:
                return httpx.Response(200, request=request, json={
                    "id": "fixture/model", "sha": revision,
                    "siblings": [{"rfilename": "weights.bin"}],
                })
            headers = {"x-repo-commit": revision, "etag": digest,
                       "content-length": str(len(payload))}
            return httpx.Response(200, request=request, headers=headers,
                                  stream=httpx.ByteStream(b"" if request.method == "HEAD" else payload))
        with tempfile.TemporaryDirectory() as directory:
            policy = pathlib.Path(directory, "network-policy.json")
            policy.write_text('{"offline_mode":false}', encoding="utf-8")
            model = pathlib.Path(directory, "model")
            with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}), \
                    mock.patch.object(httpx.HTTPTransport, "handle_request", side_effect=transport), \
                    mock.patch.object(runtime.time, "sleep") as sleep, \
                    mock.patch.object(runtime, "log_err"):
                runtime.install_network_guard()
                runtime.download_snapshot_with_retry("fixture/model", revision=revision,
                                                     local_dir=str(model), allow_patterns=["weights.bin"], max_workers=1)
                runtime.verify_weight_files(str(model), [{"filename": "weights.bin", "sha256": digest}])
                self.assertEqual((model / "weights.bin").read_bytes(), payload)
                self.assertEqual(sleep.call_args_list, [mock.call(1), mock.call(2)])
                self.assertTrue(all(request.url.host == "huggingface.co" for request in calls))
                self.assertTrue(all(request.extensions["timeout"]["read"] == 60 for request in calls[3:]))
            huggingface_hub.close_session()

    def test_transient_retry_is_bounded_and_rechecks_airplane_policy(self):
        import huggingface_hub
        with mock.patch.object(runtime, "require_network_online"), \
                mock.patch.object(huggingface_hub, "snapshot_download", side_effect=httpx.ConnectError("temporary")) as download, \
                mock.patch.object(runtime.time, "sleep") as sleep, mock.patch.object(runtime, "log_err"):
            with self.assertRaises(httpx.ConnectError):
                runtime.download_snapshot_with_retry("fixture/model")
            self.assertEqual(download.call_count, 3)
            self.assertEqual(sleep.call_args_list, [mock.call(1), mock.call(2)])
        with tempfile.TemporaryDirectory() as directory:
            policy = pathlib.Path(directory, "network-policy.json")
            policy.write_text('{"offline_mode":false}', encoding="utf-8")
            def airplane_mode(_):
                policy.write_text('{"offline_mode":true}', encoding="utf-8")
            with mock.patch.dict(os.environ, {"REFLOW_NETWORK_POLICY": str(policy)}), \
                    mock.patch.object(huggingface_hub, "snapshot_download", side_effect=httpx.ConnectError("temporary")) as download, \
                    mock.patch.object(runtime.time, "sleep", side_effect=airplane_mode), \
                    mock.patch.object(runtime, "log_err"):
                with self.assertRaisesRegex(RuntimeError, "Offline mode"):
                    runtime.download_snapshot_with_retry("fixture/model")
                self.assertEqual(download.call_count, 1)

    def test_policy_integrity_http_and_permanent_dns_failures_are_not_retried(self):
        import huggingface_hub
        request = httpx.Request("GET", "https://huggingface.co/pinned")
        response = httpx.Response(403, request=request)
        errors = [RuntimeError("Offline mode is on"), RuntimeError("Downloads are restricted to pinned artifact hosts"),
                  ValueError("SHA-256 mismatch"), socket.gaierror(socket.EAI_NONAME, "Name does not exist"),
                  ssl.SSLCertVerificationError("certificate verification failed"),
                  httpx.HTTPStatusError("Forbidden", request=request, response=response)]
        for error in errors:
            with self.subTest(error=error), mock.patch.object(runtime, "require_network_online"), \
                    mock.patch.object(huggingface_hub, "snapshot_download", side_effect=error) as download, \
                    mock.patch.object(runtime.time, "sleep") as sleep:
                with self.assertRaises(type(error)):
                    runtime.download_snapshot_with_retry("fixture/model")
                self.assertEqual(download.call_count, 1)
                sleep.assert_not_called()

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
