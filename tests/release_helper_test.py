import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import release_helper


def png(width=912, height=684):
    def chunk(kind, payload):
        import zlib
        return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    scanlines = b"".join(b"\x00" + bytes(width * 3) for _ in range(height))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(scanlines)) + chunk(b"IEND", b"")


class FakeTransport:
    """Deterministic in-memory PortalSurfer transport; never opens a socket."""

    def __init__(self, manifest_schema_versions=(2,)):
        self.manifest_schema_versions = list(manifest_schema_versions)
        self.calls = []

    def __call__(self, url, method, body, headers):
        self.calls.append((url, method, body, headers))
        if method == "GET":
            payload = json.dumps({"release_upload": {"manifest_schema_versions": self.manifest_schema_versions}}).encode()
            return 200, payload
        return 201, b""


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
        transport = FakeTransport(manifest_schema_versions=(1,))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            names = ["a.zip", "b.zip", "shot.png", "CHANGELOG.md"]
            for name in names:
                (root / name).write_bytes(png() if name == "shot.png" else name.encode())
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=root / "a.zip", vst3=root / "b.zip", screenshot=root / "shot.png", changelog=root / "CHANGELOG.md", distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            with self.assertRaisesRegex(RuntimeError, "schema 2"):
                release_helper.publish_manifest(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest=manifest, root=root, transport=transport)
        self.assertEqual([call[1] for call in transport.calls], ["GET"])

    def test_v2_uploads_four_files_and_exact_manifest_commit(self):
        transport = FakeTransport()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = []
            for name, body in (("a.zip", b"a"), ("b.zip", b"b"), ("shot.png", png()), ("CHANGELOG.md", b"c")):
                (root / name).write_bytes(body); files.append((name, root / name, release_helper.file_digest(root / name)[0]))
            manifest = {"schema_version": 2, "product": "pump", "build_id": "pump-v0.2.0-test", "source": {"repository": "PORTALSURFER/pump", "git_sha": "a" * 40, "dirty": False}, "distribution": "production", "signing": {"identity_class": "Developer ID Application", "notarized": True, "stapled": True, "team_id": "TEAM123456", "notary_submissions": {"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"}}, "version": "0.2.0", "channel": "stable", "released_at": "2026-07-28T00:00:00Z", "artifacts": [{"format": "clap", "platform": "macos", "architectures": ["arm64"], "name": "a.zip", "sha256": files[0][2], "size_bytes": (root / "a.zip").stat().st_size, "media_type": "application/zip"}, {"format": "vst3", "platform": "macos", "architectures": ["arm64"], "name": "b.zip", "sha256": files[1][2], "size_bytes": (root / "b.zip").stat().st_size, "media_type": "application/zip"}], "screenshot": {"role": "default-ui", "name": "shot.png", "media_type": "image/png", "width": 912, "height": 684, "logical_width": 912, "logical_height": 684, "dpi_scale": 1.0, "source_git_sha": "a" * 40, "sha256": files[2][2], "size_bytes": (root / "shot.png").stat().st_size}, "changelog": {"name": "CHANGELOG.md", "format": "markdown", "media_type": "text/markdown; charset=utf-8", "sha256": files[3][2], "size_bytes": (root / "CHANGELOG.md").stat().st_size}}
            release_helper.publish_manifest(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest=manifest, root=root, transport=transport)
        self.assertEqual(len(transport.calls), 6)
        self.assertEqual(transport.calls[-1][3]["Content-Type"], release_helper.MANIFEST_CONTENT_TYPE)
        self.assertEqual(transport.calls[-1][3]["X-PortalSurfer-Release-Version"], "0.2.0")
        self.assertEqual(transport.calls[-1][3]["X-PortalSurfer-Released-At"], "2026-07-28T00:00:00Z")
        self.assertEqual(transport.calls[-1][2], release_helper.canonical_json(manifest))

    def test_publish_rejects_invalid_endpoints_before_transport(self):
        invalid_endpoints = (
            "http://portalsurfer.org",
            "https://portalsurfer.org/",
            "https://portalsurfer.org:443",
            "https://user@portalsurfer.org",
            "https://portalsurfer.org/path",
        )
        for endpoint in invalid_endpoints:
            with self.subTest(endpoint=endpoint):
                transport = FakeTransport()
                with self.assertRaisesRegex(ValueError, "exact origin https://portalsurfer.org"):
                    release_helper.publish_manifest(endpoint=endpoint, token="secret", manifest={}, root=Path("."), transport=transport)
                self.assertEqual(transport.calls, [])

    def test_publish_block_audits_exact_zips_before_manifest_publish(self):
        script = (Path(__file__).parents[1] / "scripts" / "release.sh").read_text(encoding="utf-8")
        publish_block = script.split('if [[ "${mode}" == publish ]]; then', 1)[1]
        publish_manifest_invocation = publish_block.index("publish_manifest(\n")
        audited_block = publish_block[:publish_manifest_invocation]
        self.assertIn('audit_zip clap "${release_dir}/pump-v${version}-macos.clap.zip" "${signing_team_id}"', audited_block)
        self.assertIn('audit_zip vst3 "${release_dir}/pump-v${version}-macos.vst3.zip" "${signing_team_id}"', audited_block)

    def test_publish_rejects_tampered_final_zip_before_transport(self):
        transport = FakeTransport()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "shot.png"; screenshot.write_bytes(png())
            clap = root / "a.zip"; clap.write_bytes(b"clap")
            vst3 = root / "b.zip"; vst3.write_bytes(b"vst3")
            changelog = root / "CHANGELOG.md"; changelog.write_text("release\n", encoding="utf-8")
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            clap.write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "on-disk bytes do not match manifest"):
                release_helper.publish_manifest(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest=manifest, root=root, transport=transport)
        self.assertEqual(transport.calls, [])

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
