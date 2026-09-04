#!/usr/bin/env python3
"""Focused regression tests for Pump's release identity and artifact contracts."""

from __future__ import annotations

import json
import struct
import tempfile
import unittest
import zlib
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import release_helper


SOURCE_SHA = "a" * 40
BUILD_ID = f"pump-v0.2.6-nightly.17-{SOURCE_SHA[:12]}"
RELEASED_AT = "2026-09-02T10:20:30Z"
TEAM_ID = "TEAM123456"
CLAP_NOTARY_ID = "12345678-1234-4123-8123-123456789abc"
VST3_NOTARY_ID = "abcdefab-cdef-4abc-8def-abcdefabcdef"


def png(width: int = 640, height: int = 400) -> bytes:
    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    scanlines = b"".join(b"\x00" + bytes(width * 3) for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(scanlines))
        + chunk(b"IEND", b"")
    )


class FakeTransport:
    def __init__(self, manifest_schema_versions=(2,)) -> None:
        self.manifest_schema_versions = list(manifest_schema_versions)
        self.calls = []

    def __call__(self, url, method, body, headers):
        self.calls.append((url, method, body, headers))
        if method == "GET":
            return (
                200,
                json.dumps(
                    {"release_upload": {"manifest_schema_versions": self.manifest_schema_versions}}
                ).encode(),
            )
        return 201, b""


def write_support_files(root: Path) -> tuple[Path, Path]:
    screenshot = root / "pump-default-640x400.png"
    screenshot.write_bytes(png())
    changelog = root / "CHANGELOG.md"
    changelog.write_text("# Pump release\n", encoding="utf-8")
    return screenshot, changelog


def write_archives(root: Path, publication_version: str, *, windows: bool) -> tuple[Path, Path, Path | None]:
    clap = root / f"pump-v{publication_version}-macos.clap.zip"
    vst3 = root / f"pump-v{publication_version}-macos.vst3.zip"
    clap.write_bytes(b"macOS CLAP")
    vst3.write_bytes(b"macOS VST3")
    windows_archive = None
    if windows:
        windows_archive = root / f"pump-v{publication_version}-windows-x86_64-unsigned.vst3.zip"
        windows_archive.write_bytes(b"Windows VST3")
    return clap, vst3, windows_archive


def build_release(
    root: Path,
    *,
    channel: str,
    publication_version: str,
    distribution: str,
    windows: bool,
    build_id: str,
) -> dict:
    root.mkdir(parents=True, exist_ok=True)
    screenshot, changelog = write_support_files(root)
    clap, vst3, windows_archive = write_archives(
        root, publication_version, windows=windows
    )
    manifest = release_helper.build_manifest(
        version=publication_version,
        build_id=build_id,
        channel=channel,
        released_at=RELEASED_AT,
        git_sha=SOURCE_SHA,
        clap=clap,
        vst3=vst3,
        screenshot=screenshot,
        changelog=changelog,
        distribution=distribution,
        signing_identity_class=(
            "Developer ID Application" if distribution == "production" else "ad hoc"
        ),
        notarized=distribution == "production",
        stapled=distribution == "production",
        signing_team_id=TEAM_ID if distribution == "production" else "",
        notary_submissions=(
            {"clap": CLAP_NOTARY_ID, "vst3": VST3_NOTARY_ID}
            if distribution == "production"
            else None
        ),
        windows_vst3=windows_archive,
    )
    (root / "release-manifest.json").write_bytes(release_helper.canonical_json(manifest))
    return manifest


class ReleaseHelperTests(unittest.TestCase):
    def test_nightly_publication_version_uses_package_version_and_workflow_sequence(self) -> None:
        self.assertEqual(
            release_helper.derive_publication_version("0.2.6", "nightly", 17),
            "0.2.6-nightly.17",
        )
        self.assertEqual(
            release_helper.derive_publication_version("0.2.6", "stable", 17),
            "0.2.6",
        )
        with self.assertRaisesRegex(ValueError, "positive canonical"):
            release_helper.derive_publication_version("0.2.6", "nightly", 0)
        with self.assertRaises(ValueError):
            release_helper.validate_publication_version(
                "0.2.6", "0.2.7-nightly.17", "nightly"
            )

    def test_release_history_decision_remains_channel_specific(self) -> None:
        sha_a = "a" * 40
        sha_b = "b" * 40
        document = {
            "releases": [
                {
                    "channel": "nightly",
                    "released_at": "2026-07-29T18:30:00Z",
                    "source": {
                        "repository": "PORTALSURFER/pump",
                        "git_sha": sha_a,
                    },
                },
                {
                    "channel": "nightly",
                    "released_at": "2026-07-29T20:00:00Z",
                    "source": {
                        "repository": "PORTALSURFER/pump",
                        "git_sha": sha_b,
                    },
                },
            ]
        }
        self.assertFalse(
            release_helper.should_release(
                source_sha=sha_b, document=document, channel="nightly"
            )
        )
        self.assertTrue(
            release_helper.should_release(
                source_sha=sha_a, document=document, channel="nightly"
            )
        )

    def test_stable_and_rc_retain_schema2_macos_only_contract(self) -> None:
        for channel in ("stable", "rc"):
            with self.subTest(channel=channel), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                manifest = build_release(
                    root,
                    channel=channel,
                    publication_version="0.2.6",
                    distribution="production",
                    windows=False,
                    build_id=f"pump-v0.2.6-{channel}-{SOURCE_SHA[:12]}",
                )
                self.assertEqual(manifest["schema_version"], 2)
                self.assertEqual(
                    {(artifact["platform"], artifact["format"]) for artifact in manifest["artifacts"]},
                    {("macos", "clap"), ("macos", "vst3")},
                )
                self.assertNotIn("security", manifest["artifacts"][0])
                release_helper.validate_manifest(manifest, root)
                release_helper.validate_publish_manifest(manifest, root)

    def test_preflight_schema2_remains_ad_hoc_and_mac_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = build_release(
                root,
                channel="stable",
                publication_version="0.2.6",
                distribution="preflight",
                windows=False,
                build_id=BUILD_ID,
            )
            self.assertEqual(manifest["schema_version"], 2)
            release_helper.validate_preflight_manifest(manifest, root)
            with self.assertRaisesRegex(ValueError, "production Developer ID"):
                release_helper.validate_publish_manifest(manifest, root)

    def test_production_nightly_requires_the_windows_sidecar_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaisesRegex(ValueError, "requires the Windows artifact"):
                build_release(
                    root,
                    channel="nightly",
                    publication_version="0.2.6-nightly.17",
                    distribution="production",
                    windows=False,
                    build_id=BUILD_ID,
                )

    def test_nightly_schema3_combines_three_artifacts_under_one_identity(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = build_release(
                root,
                channel="nightly",
                publication_version="0.2.6-nightly.17",
                distribution="production",
                windows=True,
                build_id=BUILD_ID,
            )
            self.assertEqual(manifest["schema_version"], 3)
            self.assertEqual(manifest["build_id"], BUILD_ID)
            self.assertEqual(
                [artifact["name"] for artifact in manifest["artifacts"]],
                [
                    "pump-v0.2.6-nightly.17-macos.clap.zip",
                    "pump-v0.2.6-nightly.17-macos.vst3.zip",
                    "pump-v0.2.6-nightly.17-windows-x86_64-unsigned.vst3.zip",
                ],
            )
            self.assertEqual(
                manifest["artifacts"][2]["security"],
                {"status": "unsigned", "certificate": None},
            )
            release_helper.validate_manifest(manifest, root)

            (root / manifest["artifacts"][2]["name"]).write_bytes(b"tampered")
            with self.assertRaisesRegex(ValueError, "hash/size mismatch"):
                release_helper.validate_manifest(manifest, root)

    def test_schema3_rejects_mismatched_macos_signing_teams(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = build_release(
                root,
                channel="nightly",
                publication_version="0.2.6-nightly.17",
                distribution="production",
                windows=True,
                build_id=BUILD_ID,
            )
            manifest["artifacts"][1]["security"]["team_id"] = "OTHERTEAM1"
            with self.assertRaisesRegex(ValueError, "signing teams differ"):
                release_helper.validate_manifest(manifest, root)

    def test_schema2_uploads_only_four_files_and_schema3_is_node_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = build_release(
                root,
                channel="stable",
                publication_version="0.2.6",
                distribution="production",
                windows=False,
                build_id=f"pump-v0.2.6-{SOURCE_SHA[:12]}",
            )
            transport = FakeTransport()
            release_helper._publish_validated_manifest(
                endpoint="https://portalsurfer.org",
                token="test-token",
                manifest=manifest,
                root=root,
                transport=transport,
            )
            self.assertEqual(len(transport.calls), 6)
            self.assertEqual(transport.calls[0][1], "GET")
            self.assertEqual(
                [call[0].rsplit("/", 1)[-1] for call in transport.calls[1:5]],
                [
                    "pump-v0.2.6-macos.clap.zip",
                    "pump-v0.2.6-macos.vst3.zip",
                    "pump-default-640x400.png",
                    "CHANGELOG.md",
                ],
            )
            self.assertTrue(transport.calls[-1][0].endswith("/commit"))

            nightly = build_release(
                root / "nightly",
                channel="nightly",
                publication_version="0.2.6-nightly.17",
                distribution="production",
                windows=True,
                build_id=BUILD_ID,
            )
            with self.assertRaisesRegex(ValueError, "schema 2"):
                release_helper.validate_publish_manifest(nightly, root / "nightly")

    def test_release_and_workflow_keep_shared_identity_without_version_bump(self) -> None:
        project = Path(__file__).parents[1]
        release_script = (project / "scripts" / "release.sh").read_text(encoding="utf-8")
        workflow = (project / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        self.assertIn("--publication-version", release_script)
        self.assertIn("--source-sha", release_script)
        self.assertIn("--windows-release-dir", release_script)
        self.assertIn("validate_publish_manifest", release_script)
        self.assertIn("python3 - \"$archive\"", release_script)
        self.assertIn("pump-v${publication_version}-windows-x86_64-unsigned.vst3.zip", release_script)
        for forbidden in (
            "scripts/bump_version.py",
            "git push origin HEAD:main",
            "release_helper.next_release_version",
            "git rev-list --count",
        ):
            self.assertNotIn(forbidden, release_script)
            self.assertNotIn(forbidden, workflow)
        self.assertIn("derive_publication_version", workflow)
        self.assertIn("github.run_number", workflow)
        self.assertIn("needs.prepare.outputs.source_sha", workflow)
        self.assertIn("needs.prepare.outputs.publication_version", workflow)
        self.assertIn("uses: ./.github/workflows/windows-release.yml", workflow)
        self.assertIn("inputs.channel == 'nightly'", workflow)

    def test_windows_and_preflight_workflows_preserve_security_boundaries(self) -> None:
        project = Path(__file__).parents[1]
        windows = (project / ".github" / "workflows" / "windows-release.yml").read_text(
            encoding="utf-8"
        )
        preflight = (project / ".github" / "workflows" / "release-preflight.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("runs-on: windows-2022", windows)
        self.assertIn("workflow_call:", windows)
        self.assertIn("RUST_TARGET: x86_64-pc-windows-msvc", windows)
        self.assertIn("dist/Pump-v${PACKAGE_VERSION}.vst3/Contents/x86_64-win/Pump-v${PACKAGE_VERSION}.vst3", windows)
        self.assertIn("pump-v${PUBLICATION_VERSION}-windows-x86_64-unsigned.vst3.zip", windows)
        self.assertIn("windows-artifact-manifest.json", windows)
        self.assertIn("permissions:\n  contents: read", windows)
        self.assertNotIn("id-token:", windows)
        self.assertNotIn("secrets.", windows)
        self.assertNotIn("PORTALSURFER_RELEASE_TOKEN", windows)
        self.assertIn("windows_integration:", preflight)
        self.assertIn("tests/release_pipeline_integration.py", preflight)
        self.assertIn("python3 tests/release_helper_test.py", preflight)
        self.assertIn("python3 tests/windows_release_helper_test.py", preflight)

    def test_nightly_scheduler_does_not_receive_release_credentials_or_mutate_source(self) -> None:
        workflow = (Path(__file__).parents[1] / ".github" / "workflows" / "nightly.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn("actions/workflows/release.yml/dispatches", workflow)
        self.assertIn('\\"channel\\":\\"nightly\\"', workflow)
        self.assertNotIn("PORTALSURFER_RELEASE_TOKEN", workflow)
        self.assertNotIn("APPLE_", workflow)
        self.assertNotIn("git push", workflow)
        self.assertNotIn("bump_version.py", workflow)


if __name__ == "__main__":
    unittest.main()
