import io
import threading
import types
import unittest

from test_zipformer_e2e import read_reply


class ReplyTests(unittest.TestCase):
    def test_matches_requested_reply_and_reports_closed_sidecar(self):
        proc = types.SimpleNamespace(stdout=io.BytesIO(b'{"id":1}\n{"id":2,"status":"ok"}\n'))
        self.assertEqual(read_reply(proc, 2, 1), {"id": 2, "status": "ok"})
        with self.assertRaisesRegex(RuntimeError, "closed"):
            read_reply(proc, timeout=1)

    def test_stalled_stdout_obeys_deadline(self):
        release = threading.Event()

        class StalledOutput:
            def __iter__(self):
                release.wait(2)
                return iter(())

        proc = types.SimpleNamespace(stdout=StalledOutput())
        try:
            with self.assertRaises(TimeoutError):
                read_reply(proc, timeout=0.02)
        finally:
            release.set()

    def test_invalid_json_reports_protocol_error(self):
        proc = types.SimpleNamespace(stdout=io.BytesIO(b'not json\n'))
        with self.assertRaisesRegex(RuntimeError, "invalid sidecar reply"):
            read_reply(proc, timeout=1)


if __name__ == '__main__':
    unittest.main()
