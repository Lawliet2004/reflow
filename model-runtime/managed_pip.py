"""Managed pip entrypoint: fixed HTTPS sources and payload-byte journaling.

All pip HTTP adapters pass this guard, including redirects and proxy use.
Completed cache hits do not produce network journal entries. No request URL,
credentials, package path or query is journaled. This is deliberately separate
from the model SDK guard and tested against the pinned pip versions.
"""
import json
import os
import sys
from datetime import datetime, timezone
from urllib.parse import urlsplit

HOSTS = frozenset(("pypi.org", "files.pythonhosted.org", "download.pytorch.org",
                   "download-r2.pytorch.org"))


def check_url(url):
    policy = os.environ["REFLOW_NETWORK_POLICY"]
    with open(policy, encoding="utf-8") as stream:
        if json.load(stream).get("offline_mode") is not False:
            raise RuntimeError("Offline mode is on")
    parsed = urlsplit(url)
    if (parsed.scheme != "https" or parsed.hostname not in HOSTS
            or parsed.port not in (None, 443) or parsed.username or parsed.password):
        raise RuntimeError("Speech packages are restricted to the approved PyPI/PyTorch HTTPS hosts")
    return parsed.hostname


def journal(host, count):
    path = os.path.join(os.path.dirname(os.environ["REFLOW_NETWORK_POLICY"]), "network-journal.jsonl")
    with open(path, "a", encoding="utf-8") as stream:
        stream.write(json.dumps({"timestamp": datetime.now(timezone.utc).isoformat(),
                                 "host": host, "bytes": count}, separators=(",", ":")) + "\n")


class JournalBody:
    def __init__(self, raw, url):
        self.raw, self.url = raw, url
        self.host = check_url(url)
        self.count = 0
        self.finished = False

    def __getattr__(self, name):
        return getattr(self.raw, name)

    def finish(self):
        if not self.finished:
            self.finished = True
            journal(self.host, self.count)

    def read(self, *args, **kwargs):
        check_url(self.url)
        data = self.raw.read(*args, **kwargs)
        self.count += len(data or b"")
        if not data or not args or args[0] is None:
            self.finish()
        return data

    def stream(self, *args, **kwargs):
        try:
            for chunk in self.raw.stream(*args, **kwargs):
                check_url(self.url)
                self.count += len(chunk)
                yield chunk
        finally:
            self.finish()

    def close(self):
        try:
            self.raw.close()
        finally:
            self.finish()


def install_guard():
    from pip._vendor.requests.adapters import HTTPAdapter
    original = HTTPAdapter.send

    def guarded_send(adapter, request, *args, **options):
        host = check_url(request.url)
        try:
            response = original(adapter, request, *args, **options)
        except Exception:
            journal(host, 0)
            raise
        response.raw = JournalBody(response.raw, request.url)
        return response

    HTTPAdapter.send = guarded_send


def main():
    install_guard()
    # pip's TLS verification remains enabled; configuration and package indexes
    # are supplied by the parent. Never accept inherited user/site config.
    from pip._internal.cli.main import main as pip_main
    return pip_main(sys.argv[1:])


if __name__ == "__main__":
    sys.exit(main())
