import json
import struct
import tempfile
import unittest
import zlib
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import sys
import threading

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import release_helper


def png(width=912, height=684):
    def chunk(kind, payload):
        import zlib
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    scanlines = b"".join(b"\x00" + bytes(width * 3) for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(scanlines)) + chunk(b"IEND", b"")


class ReleaseHelperTests(unittest.TestCase):
    def test_manifest_and_png_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "pump-default-912x684.png"
            screenshot.write_bytes(png())
            clap, vst3, changelog = (root / name for name in ("clap.zip", "vst3.zip", "CHANGELOG.md"))
            clap.write_bytes(b"clap")
            vst3.write_bytes(b"vst3")
            changelog.write_text("# Release\n", encoding="utf-8")
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-abcdef012345", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            self.assertEqual(manifest["schema_version"], 2)
            self.assertEqual(manifest["source"]["dirty"], False)
            self.assertEqual((manifest["screenshot"]["logical_width"], manifest["screenshot"]["logical_height"]), (912, 684))
            self.assertEqual(json.loads(release_helper.canonical_json(manifest)), manifest)

    def test_capability_refusal_makes_zero_puts(self):
        calls = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                calls.append(self.command)
                body = b'{"release_upload":{"manifest_schema_versions":[1]}}'
                self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            def do_PUT(self):
                calls.append(self.command); self.send_response(500); self.end_headers()
            def log_message(self, *_): pass
        server = HTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                names = ["a.zip", "b.zip", "shot.png", "CHANGELOG.md"]
                digests = []
                for name in names:
                    path = root / name; path.write_bytes(png() if name == "shot.png" else name.encode()); digests.append(release_helper.file_digest(path)[0])
                manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=root / "a.zip", vst3=root / "b.zip", screenshot=root / "shot.png", changelog=root / "CHANGELOG.md", distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
                with self.assertRaisesRegex(RuntimeError, "schema 2"):
                    release_helper.publish_manifest(endpoint=f"http://127.0.0.1:{server.server_port}", token="secret", manifest=manifest, root=root)
        finally:
            server.shutdown(); thread.join(); server.server_close()
        self.assertEqual(calls, ["GET"])

    def test_v2_uploads_four_files_and_exact_manifest_commit(self):
        calls = []
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                if self.headers.get("Authorization") is not None:
                    raise AssertionError("capability GET must not send bearer token")
                body = b'{"release_upload":{"manifest_schema_versions":[2]}}'
                self.send_response(200); self.send_header("Content-Length", str(len(body))); self.end_headers(); self.wfile.write(body)
            def do_PUT(self):
                length = int(self.headers["Content-Length"]); body = self.rfile.read(length)
                calls.append((self.path, self.headers.get("Content-Type"), self.headers.get("X-PortalSurfer-Release-Version"), self.headers.get("X-PortalSurfer-Released-At"), body))
                self.send_response(201); self.end_headers()
            def log_message(self, *_): pass
        server = HTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = []
            for name, body in (("a.zip", b"a"), ("b.zip", b"b"), ("shot.png", png()), ("CHANGELOG.md", b"c")):
                (root / name).write_bytes(body); files.append((name, root / name, release_helper.file_digest(root / name)[0]))
            manifest = {"schema_version": 2, "product": "pump", "build_id": "pump-v0.2.0-test", "source": {"repository": "PORTALSURFER/pump", "git_sha": "a" * 40, "dirty": False}, "distribution": "production", "signing": {"identity_class": "Developer ID Application", "notarized": True, "stapled": True, "team_id": "TEAM123456", "notary_submissions": {"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"}}, "version": "0.2.0", "channel": "stable", "released_at": "2026-07-28T00:00:00Z", "artifacts": [{"format": "clap", "platform": "macos", "architectures": ["arm64"], "name": "a.zip", "sha256": files[0][2], "size_bytes": (root / "a.zip").stat().st_size, "media_type": "application/zip"}, {"format": "vst3", "platform": "macos", "architectures": ["arm64"], "name": "b.zip", "sha256": files[1][2], "size_bytes": (root / "b.zip").stat().st_size, "media_type": "application/zip"}], "screenshot": {"role": "default-ui", "name": "shot.png", "media_type": "image/png", "width": 912, "height": 684, "logical_width": 912, "logical_height": 684, "dpi_scale": 1.0, "source_git_sha": "a" * 40, "sha256": files[2][2], "size_bytes": (root / "shot.png").stat().st_size}, "changelog": {"name": "CHANGELOG.md", "format": "markdown", "media_type": "text/markdown; charset=utf-8", "sha256": files[3][2], "size_bytes": (root / "CHANGELOG.md").stat().st_size}}
            release_helper.publish_manifest(endpoint=f"http://127.0.0.1:{server.server_port}", token="secret", manifest=manifest, root=root)
        server.shutdown(); thread.join(); server.server_close()
        self.assertEqual(len(calls), 5)
        self.assertEqual(calls[-1][1], release_helper.MANIFEST_CONTENT_TYPE)
        self.assertEqual(calls[-1][2], "0.2.0")
        self.assertEqual(calls[-1][3], "2026-07-28T00:00:00Z")
        self.assertEqual(calls[-1][4], release_helper.canonical_json(manifest))

    def test_publish_rejects_more_than_two_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "shot.png"; screenshot.write_bytes(png())
            clap = root / "a.zip"; clap.write_bytes(b"a")
            vst3 = root / "b.zip"; vst3.write_bytes(b"b")
            changelog = root / "CHANGELOG.md"; changelog.write_text("release\n", encoding="utf-8")
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            manifest["artifacts"].append(dict(manifest["artifacts"][0]))
            with self.assertRaisesRegex(ValueError, "exactly CLAP and VST3"):
                release_helper.validate_publish_manifest(manifest, root)

    def test_production_manifest_rejects_ad_hoc_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "shot.png"; screenshot.write_bytes(png())
            clap = root / "a.zip"; clap.write_bytes(b"a")
            vst3 = root / "b.zip"; vst3.write_bytes(b"b")
            changelog = root / "CHANGELOG.md"; changelog.write_text("release\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "production manifests"):
                release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production")


if __name__ == "__main__":
    unittest.main()
