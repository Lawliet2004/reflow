"""Model-free tests of the runtime pack builder and catalog trust boundary."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location("runtime_packs", Path(__file__).with_name("runtime-packs.py"))
PACKS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKS)


class RuntimePackTests(unittest.TestCase):
    def test_split_archives_retain_exact_bytes_and_reject_missing_corrupt_or_reordered_parts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "fixture.tar.gz"
            payload = bytes(range(251)) * 13
            archive.write_bytes(payload)
            pin = {"asset": archive.name, "bytes": len(payload), "sha256": PACKS.digest(archive)}
            self.assertEqual(PACKS.split_archive(archive, len(payload)), [])
            self.assertTrue(archive.exists())
            pin["parts"] = PACKS.split_archive(archive, 1024)
            self.assertFalse(archive.exists())
            self.assertEqual(len(pin["parts"]), 4)
            self.assertEqual(sum(p["bytes"] for p in pin["parts"]), len(payload))
            self.assertTrue(all(p["bytes"] <= 1024 for p in pin["parts"]))
            PACKS.verify_pack_archive(root, pin)
            pin["parts"].reverse()
            with self.assertRaisesRegex(ValueError, "archive differs"):
                PACKS.verify_pack_archive(root, pin)
            pin["parts"].reverse()
            path = root / pin["parts"][1]["asset"]
            original = path.read_bytes()
            path.write_bytes(b"x" * len(original))
            with self.assertRaisesRegex(ValueError, "archive differs"):
                PACKS.verify_pack_archive(root, pin)
            path.unlink()
            with self.assertRaises(FileNotFoundError):
                PACKS.verify_pack_archive(root, pin)

    def test_release_matrix_covers_each_reviewed_profile_once_on_its_native_runner(self):
        rows = PACKS.matrix()["include"]
        expected = {p.stem for p in PACKS.LOCKS.glob("*.txt")
                    if any(p.name.startswith(t + "-") for t in PACKS.RELEASE_TARGETS)}
        actual = [f"{r['target']}-{r['profile']}-{r['variant']}" for r in rows]
        self.assertEqual(set(actual), expected)
        self.assertEqual(len(actual), len(set(actual)))
        self.assertEqual(len(rows), 20)
        self.assertEqual([r["profile"] for r in rows if r["target"] == "x86_64-apple-darwin"], ["zipformer"])

    def test_cross_compiling_a_wheelhouse_is_refused_before_any_download(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "native host"):
                PACKS.build(Path(directory), "unsupported-host", "zipformer", "none", Path(directory), None)

    def test_offline_probe_rejects_socket_connections(self):
        result = subprocess.run([sys.executable, "-c", PACKS.DENY_NETWORK +
                                 "\nsocket.create_connection(('127.0.0.1', 9))"], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Network is disabled", result.stderr)

    def test_offline_pip_uses_the_module_entrypoint_without_losing_arguments_or_network_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "pip").mkdir()
            (root / "pip/__init__.py").write_text("")
            # pip refuses self-upgrades on Windows when argv[0] is 'pip'.
            # A normal python -m pip launch instead identifies __main__.py.
            (root / "pip/__main__.py").write_text(
                "import pathlib, socket, sys\n"
                "assert pathlib.Path(sys.argv[0]).name == '__main__.py', sys.argv\n"
                "assert sys.argv[1:] == ['install', '--upgrade', 'pip==26.2.2'], sys.argv\n"
                "try:\n socket.create_connection(('127.0.0.1', 9))\n"
                "except RuntimeError as error:\n assert 'Network is disabled' in str(error)\n"
                "else:\n raise AssertionError('network guard missing')\n"
            )
            with patch.object(PACKS.subprocess, "run") as child:
                PACKS.run(sys.executable, ["-m", "pip", "install", "--upgrade", "pip==26.2.2"], offline=True)
            command = child.call_args.args[0]
            result = subprocess.run(command, cwd=root, env=PACKS.clean_environment(), capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_speech_probes_reject_successful_package_imports_with_broken_backend_apis(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for module in ("numpy", "huggingface_hub", "httpx", "fermion", "scipy", "soundfile", "safetensors", "zstandard"):
                (root / f"{module}.py").write_text("")
            (root / "torch.py").write_text(
                "class Tensor:\n def add(self, value): return self\n def item(self): return 2\n"
                "def ones(*args): return Tensor()\n")
            (root / "transformers.py").write_text(
                "def __getattr__(name):\n if name.startswith('__'): raise AttributeError(name)\n"
                " raise ImportError('Broken lazy API: ' + name)\n")
            for profile, missing in [("qwen", "AutoProcessor"), ("phonon", "fermion._speech")]:
                with self.subTest(profile=profile):
                    result = subprocess.run([sys.executable, "-c", PACKS.DENY_NETWORK + PACKS.check_code(profile)],
                                            cwd=root, env=PACKS.clean_environment(), capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(missing, result.stderr)

    def test_license_inventory_keeps_license_and_notice_text_and_rejects_missing_metadata(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = root / "example-1-py3-none-any.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("example-1.dist-info/METADATA", "Name: example\nVersion: 1\nLicense-Expression: MIT\n")
                archive.writestr("example-1.dist-info/licenses/LICENSE", "Copyright example; license text")
                archive.writestr("example-1.dist-info/licenses/NOTICE", "Retain this notice")
            inventory = PACKS.license_inventory(root)
            self.assertEqual(inventory[0]["name"], "example")
            self.assertEqual(inventory[0]["license_files"][1]["text"], "Retain this notice")
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("no-metadata", "")
            with self.assertRaisesRegex(ValueError, "metadata"):
                PACKS.license_inventory(root)

    def test_license_inventory_uses_root_metadata_and_retains_vendored_notices(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = root / "example-1-py3-none-any.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("example-1.dist-info/METADATA", "Name: example\nVersion: 1\nLicense-Expression: MIT\n")
                archive.writestr("example/_vendor/helper-2.dist-info/METADATA", "Name: helper\nVersion: 2\nLicense-Expression: BSD-3-Clause\n")
                archive.writestr("example/_vendor/helper-2.dist-info/LICENSE", "Bundled helper notice")
            inventory = PACKS.license_inventory(root)
            self.assertEqual((inventory[0]["name"], inventory[0]["version"]), ("example", "1"))
            self.assertEqual(inventory[0]["license_files"][0]["text"], "Bundled helper notice")
            for entries in [
                ["example/_vendor/helper-2.dist-info/METADATA"],
                ["example-1.dist-info/METADATA", "other-2.dist-info/METADATA"],
            ]:
                with zipfile.ZipFile(wheel, "w") as archive:
                    for name in entries:
                        archive.writestr(name, "Name: example\nVersion: 1\nLicense-Expression: MIT\n")
                with self.assertRaisesRegex(ValueError, "metadata"):
                    PACKS.license_inventory(root)

    def test_missing_license_requires_an_exact_pinned_supplement(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            wheel = root / "tokenizers-0.22.2-py3-none-any.whl"
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("tokenizers-0.22.2.dist-info/METADATA", "Name: tokenizers\nVersion: 0.22.2\n")
            inventory = PACKS.license_inventory(root)[0]
            self.assertEqual(inventory["license"], "Apache-2.0")
            notice = inventory["license_files"][0]
            self.assertIn("Apache License", notice["text"])
            self.assertIn("/f383101a26663708484cac0727792aad74f78234/", notice["source"])
            changed = dict(PACKS.LICENSE_SUPPLEMENTS["tokenizers==0.22.2"], sha256="0" * 64)
            with patch.dict(PACKS.LICENSE_SUPPLEMENTS, {"tokenizers==0.22.2": changed}):
                with self.assertRaisesRegex(ValueError, "license hash changed"):
                    PACKS.license_inventory(root)
            with zipfile.ZipFile(wheel, "w") as archive:
                archive.writestr("tokenizers-0.22.3.dist-info/METADATA", "Name: tokenizers\nVersion: 0.22.3\n")
            with self.assertRaisesRegex(ValueError, "needs review"):
                PACKS.license_inventory(root)

    def test_catalog_requires_complete_matching_packs_and_default_mode_rehashes_archives(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            commit = "a" * 40
            target = "x86_64-unknown-linux-gnu"
            key = f"{target}-zipformer-none"
            asset = f"speech-runtime-{key}.tar.gz"
            (root / asset).write_bytes(b"synthetic catalog fixture; not a usable runtime")
            python = next(p for p in PACKS.BUILDS["pins"] if p["target"] == target)
            pin = {"schema": 1, "target": target, "profile": "zipformer", "variant": "none", "asset": asset,
                   "sha256": PACKS.digest(root / asset), "bytes": (root / asset).stat().st_size,
                   "unpacked_bytes": 100, "requirements_sha256": PACKS.digest(PACKS.LOCKS / f"{key}.txt"),
                   "pip_lock_sha256": PACKS.digest(PACKS.LOCKS / "pip.txt"), "pbs_sha256": python["sha256"],
                   "source_commit": commit}
            descriptor = root / asset.replace(".tar.gz", ".json")
            descriptor.write_text(json.dumps(pin))
            output = root / "runtime-packs.json"
            def collect(partial=True, metadata_only=False):
                PACKS.catalog(root, "v0.5.0", "example/reflow", commit, output, partial, metadata_only)
            with self.assertRaisesRegex(ValueError, "Missing runtime packs"):
                collect(partial=False)
            collect()
            self.assertTrue(json.loads(output.read_text())["packs"][0]["url"].endswith(asset))
            (root / asset).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "archive differs"):
                collect()
            # Only trusted CI aggregation may omit archive reads; the final
            # publication path always invokes the strict default above.
            collect(metadata_only=True)
            for field, value in [("source_commit", "b" * 40), ("requirements_sha256", "f" * 64),
                                 ("pbs_sha256", "f" * 64), ("bytes", 0), ("unpacked_bytes", True),
                                 ("asset", "../escape.tar.gz")]:
                changed = dict(pin, **{field: value})
                descriptor.write_text(json.dumps(changed))
                with self.assertRaises(ValueError, msg=field):
                    collect(metadata_only=True)
            descriptor.write_text(json.dumps(pin))
            with self.assertRaisesRegex(ValueError, "exact source commit"):
                PACKS.catalog(root, "v0.5.0", "example/reflow", "main", output, True)

            (root / asset).write_bytes(b"synthetic catalog fixture; not a usable runtime")
            pin["parts"] = PACKS.split_archive(root / asset, 20)
            descriptor.write_text(json.dumps(pin))
            collect()
            published = json.loads(output.read_text())["packs"][0]
            self.assertTrue(published["parts"][0]["url"].endswith(".part001"))
            for field, value in [("asset", "../escape"), ("bytes", True),
                                 ("sha256", "wrong")]:
                changed = json.loads(json.dumps(pin))
                changed["parts"][0][field] = value
                descriptor.write_text(json.dumps(changed))
                with self.assertRaises(ValueError):
                    collect(metadata_only=True)
            descriptor.write_text(json.dumps(pin))
            (root / pin["parts"][0]["asset"]).write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "archive differs"):
                collect()


if __name__ == "__main__":
    unittest.main()
