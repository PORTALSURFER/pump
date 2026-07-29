import json
import os
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest import mock
import zlib
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import release_helper


def png(width=720, height=540):
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
    def test_release_workflow_artifact_uses_checked_out_source_sha(self):
        workflow = (Path(__file__).parents[1] / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        checkout = workflow.index("- name: Checkout exact main source")
        capture = workflow.index("- name: Capture checked-out source SHA")
        upload = workflow.index("- name: Upload immutable bundle for inspection")
        self.assertLess(checkout, capture)
        self.assertLess(capture, upload)
        capture_block = workflow[capture:upload]
        self.assertIn("id: source_sha", capture_block)
        self.assertIn('git rev-parse HEAD', capture_block)
        upload_block = workflow[upload:]
        self.assertIn("name: pump-release-${{ inputs.channel }}-${{ steps.source_sha.outputs.sha }}", upload_block)
        self.assertNotIn("${{ github.sha }}", upload_block)

    def test_manifest_and_png_contract(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "pump-default-720x540.png"
            screenshot.write_bytes(png())
            clap, vst3, changelog = (root / name for name in ("clap.zip", "vst3.zip", "CHANGELOG.md"))
            clap.write_bytes(b"clap")
            vst3.write_bytes(b"vst3")
            changelog.write_text("# Release\n", encoding="utf-8")
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-abcdef012345", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            self.assertEqual(manifest["schema_version"], 2)
            self.assertEqual(manifest["source"]["dirty"], False)
            self.assertEqual((manifest["screenshot"]["logical_width"], manifest["screenshot"]["logical_height"]), (720, 540))
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
                release_helper._publish_validated_manifest(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest=manifest, root=root, transport=transport)
        self.assertEqual([call[1] for call in transport.calls], ["GET"])

    def test_v2_uploads_four_files_and_exact_manifest_commit(self):
        transport = FakeTransport()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = []
            for name, body in (("a.zip", b"a"), ("b.zip", b"b"), ("shot.png", png()), ("CHANGELOG.md", b"c")):
                (root / name).write_bytes(body); files.append((name, root / name, release_helper.file_digest(root / name)[0]))
            manifest = {"schema_version": 2, "product": "pump", "build_id": "pump-v0.2.0-test", "source": {"repository": "PORTALSURFER/pump", "git_sha": "a" * 40, "dirty": False}, "distribution": "production", "signing": {"identity_class": "Developer ID Application", "notarized": True, "stapled": True, "team_id": "TEAM123456", "notary_submissions": {"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"}}, "version": "0.2.0", "channel": "stable", "released_at": "2026-07-28T00:00:00Z", "artifacts": [{"format": "clap", "platform": "macos", "architectures": ["arm64"], "name": "a.zip", "sha256": files[0][2], "size_bytes": (root / "a.zip").stat().st_size, "media_type": "application/zip"}, {"format": "vst3", "platform": "macos", "architectures": ["arm64"], "name": "b.zip", "sha256": files[1][2], "size_bytes": (root / "b.zip").stat().st_size, "media_type": "application/zip"}], "screenshot": {"role": "default-ui", "name": "shot.png", "media_type": "image/png", "width": 720, "height": 540, "logical_width": 720, "logical_height": 540, "dpi_scale": 1.0, "source_git_sha": "a" * 40, "sha256": files[2][2], "size_bytes": (root / "shot.png").stat().st_size}, "changelog": {"name": "CHANGELOG.md", "format": "markdown", "media_type": "text/markdown; charset=utf-8", "sha256": files[3][2], "size_bytes": (root / "CHANGELOG.md").stat().st_size}}
            release_helper._publish_validated_manifest(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest=manifest, root=root, transport=transport)
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
                    release_helper.publish_release(endpoint=endpoint, token="secret", manifest_path=Path("missing.json"), root=Path("."), repo_root=Path("."))

    def test_publish_block_audits_exact_zips_before_manifest_publish(self):
        script = (Path(__file__).parents[1] / "scripts" / "release.sh").read_text(encoding="utf-8")
        publish_block = script.split('if [[ "${mode}" == publish ]]; then', 1)[1]
        self.assertIn("from release_helper import publish_release", publish_block)
        self.assertNotIn("publish_manifest", publish_block)
        self.assertNotIn("final audit of exact publish bytes", publish_block)

    def test_release_build_temp_parent_is_created_before_mktemp(self):
        script = (Path(__file__).parents[1] / "scripts" / "release.sh").read_text(encoding="utf-8")
        parent_creation = 'mkdir -p "${repo_root}/target"'
        temp_creation = 'tmp_root="$(mktemp -d "${repo_root}/target/release-build.XXXXXX")"'
        self.assertLess(script.index(parent_creation), script.index(temp_creation))

    def test_release_and_ci_cargo_invocations_are_lockfile_stable(self):
        root = Path(__file__).parents[1]
        for name in ("scripts/release.sh", "scripts/ci.sh"):
            with self.subTest(script=name):
                lines = (root / name).read_text(encoding="utf-8").splitlines()
                cargo_lines = [
                    line
                    for line in lines
                    if any(
                        f"cargo {command}" in line
                        for command in ("clippy", "test", "build", "rustc")
                    )
                ]
                self.assertTrue(cargo_lines)
                self.assertTrue(
                    all("--locked" in line for line in cargo_lines),
                    f"every Cargo invocation in {name} must use --locked",
                )

    def test_release_script_disables_python_bytecode_output(self):
        root = Path(__file__).parents[1]
        script = (root / "scripts" / "release.sh").read_text(encoding="utf-8")
        self.assertIn("export PYTHONDONTWRITEBYTECODE=1", script)
        with tempfile.TemporaryDirectory() as directory:
            temporary = Path(directory)
            shutil.copy2(root / "scripts" / "release_helper.py", temporary / "release_helper.py")
            environment = os.environ.copy()
            environment["PYTHONPATH"] = str(temporary)
            environment["PYTHONDONTWRITEBYTECODE"] = "1"
            subprocess.run(
                [sys.executable, "-c", "import release_helper"],
                cwd=temporary,
                env=environment,
                check=True,
            )
            self.assertFalse(list(temporary.rglob("*.pyc")))

    def test_release_keychain_is_registered_and_restored(self):
        script = (Path(__file__).parents[1] / "scripts" / "release.sh").read_text(encoding="utf-8")
        capture = "security list-keychains -d user | sed 's/[[:space:]]*\"//g; s/\"$//' > \"${original_keychains_file}\""
        register = 'security list-keychains -d user -s "${release_keychain}" "${original_keychains[@]}" >/dev/null'
        restore = 'security list-keychains -d user -s "${original_keychains[@]}" >/dev/null 2>&1 || true'
        self.assertIn(capture, script)
        self.assertIn(register, script)
        self.assertIn(restore, script)
        self.assertLess(script.index(capture), script.index('security create-keychain'))
        self.assertLess(script.index('security create-keychain'), script.index(register))
        self.assertLess(script.index(register), script.index('security import'))
        self.assertLess(script.index(register), script.index('codesign_identity='))
        self.assertLess(script.index(restore), script.index('security delete-keychain'))

    def test_release_notarization_checks_cover_live_and_extracted_bundles(self):
        script = (Path(__file__).parents[1] / "scripts" / "release.sh").read_text(encoding="utf-8")
        check = 'codesign -vvvv -R=notarized --check-notarization'
        self.assertEqual(script.count(check), 2)
        self.assertNotIn("spctl", script)
        live = script.index(check)
        extracted = script.index(check, live + 1)
        self.assertLess(script.index('xcrun stapler validate "${bundle_dir}"'), live)
        self.assertLess(script.index('xcrun stapler validate "${bundle}"'), extracted)
        self.assertLess(live, script.index('local team_id', live))
        self.assertLess(extracted, script.index('codesign_details=', extracted))

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
            (root / "release-manifest.json").write_bytes(release_helper.canonical_json(manifest))
            with self.assertRaisesRegex(ValueError, "on-disk bytes do not match manifest"):
                with mock.patch.object(release_helper, "_audit_zip", side_effect=ValueError("ZIP audit failed")), mock.patch.object(release_helper, "_validate_canonical_source"):
                    release_helper.publish_release(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest_path=root / "release-manifest.json", root=root, repo_root=root)
        self.assertEqual(transport.calls, [])

    def test_public_wrapper_rejects_raw_zip_without_request(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            screenshot = root / "pump-default-720x540.png"; screenshot.write_bytes(png())
            clap = root / "pump-v0.2.0-macos.clap.zip"; clap.write_bytes(b"raw unsigned bytes")
            vst3 = root / "pump-v0.2.0-macos.vst3.zip"; vst3.write_bytes(b"raw unsigned bytes")
            changelog = root / "CHANGELOG.md"; changelog.write_text("release\n", encoding="utf-8")
            manifest = release_helper.build_manifest(version="0.2.0", build_id="pump-v0.2.0-test", channel="stable", released_at="2026-07-28T00:00:00Z", git_sha="a" * 40, clap=clap, vst3=vst3, screenshot=screenshot, changelog=changelog, distribution="production", signing_identity_class="Developer ID Application", notarized=True, stapled=True, signing_team_id="TEAM123456", notary_submissions={"clap": "12345678-1234-4123-8123-123456789abc", "vst3": "abcdefab-cdef-4abc-8def-abcdefabcdef"})
            manifest_path = root / "release-manifest.json"
            manifest_path.write_bytes(release_helper.canonical_json(manifest))
            requests = []
            with self.assertRaisesRegex(ValueError, "ZIP audit failed"), mock.patch.object(release_helper, "_request", side_effect=lambda *args: requests.append(args)), mock.patch.object(release_helper, "_validate_canonical_source"), mock.patch.object(release_helper, "_audit_zip", side_effect=ValueError("ZIP audit failed")):
                release_helper.publish_release(endpoint=release_helper.PRODUCTION_ORIGIN, token="secret", manifest_path=manifest_path, root=root, repo_root=root)
            self.assertEqual(requests, [])

    def test_zip_audit_runs_argument_safe_mac_checks_in_order(self):
        helper_source = (Path(__file__).parents[1] / "scripts" / "release_helper.py").read_text(encoding="utf-8")
        self.assertNotIn("spctl", helper_source)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "pump-v0.2.0-macos.clap.zip"
            archive.write_bytes(b"placeholder")
            calls = []

            def run(args, *, cwd, capture_output=False):
                calls.append(tuple(args))
                if args[0] == "/usr/bin/ditto":
                    extracted = Path(args[-1])
                    binary = extracted / "pump.clap" / "Contents" / "MacOS" / "pump"
                    binary.parent.mkdir(parents=True)
                    (binary.parent.parent / "Info.plist").write_text("plist", encoding="utf-8")
                    (binary.parent.parent / "PkgInfo").write_text("BNDL????", encoding="utf-8")
                    (binary.parent.parent / "CodeResources").write_text("signed resources", encoding="utf-8")
                    binary.write_bytes(b"arm64")
                    os.chmod(binary, 0o755)
                    return subprocess.CompletedProcess(args, 0, "", "")
                if args[0] == "/usr/bin/plutil" and "CFBundleIdentifier" in args:
                    return subprocess.CompletedProcess(args, 0, "com.portalsurfer.pump.clap\n", "")
                if args[0] == "/usr/bin/plutil" and "CFBundlePackageType" in args:
                    return subprocess.CompletedProcess(args, 0, "BNDL\n", "")
                if args[0] == "codesign" and args[1] == "-dv":
                    return subprocess.CompletedProcess(args, 0, "", "Authority=Developer ID Application: PORTALSURFER\nTeamIdentifier=TEAM123456\n")
                if args[0] == "lipo":
                    return subprocess.CompletedProcess(args, 0, "arm64\n", "")
                if args[0] == "/usr/bin/nm":
                    return subprocess.CompletedProcess(args, 0, "_clap_entry\n", "")
                return subprocess.CompletedProcess(args, 0, "", "")

            with mock.patch.object(release_helper.platform, "system", return_value="Darwin"), mock.patch.object(release_helper, "_run_checked", side_effect=run):
                release_helper._audit_zip(archive, "clap", "TEAM123456", cwd=root)
            self.assertEqual([call[0] for call in calls], ["/usr/bin/ditto", "/usr/bin/plutil", "/usr/bin/plutil", "/usr/bin/plutil", "codesign", "codesign", "xcrun", "codesign", "lipo", "/usr/bin/nm"])
            self.assertEqual(calls[7][1:4], ("-vvvv", "-R=notarized", "--check-notarization"))

    def test_zip_audit_rejects_direct_code_resources_for_vst3(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "pump-v0.2.0-macos.vst3.zip"
            archive.write_bytes(b"placeholder")

            def run(args, *, cwd, capture_output=False):
                if args[0] == "/usr/bin/ditto":
                    extracted = Path(args[-1])
                    binary = extracted / "pump.vst3" / "Contents" / "MacOS" / "pump"
                    binary.parent.mkdir(parents=True)
                    (binary.parent.parent / "Info.plist").write_text("plist", encoding="utf-8")
                    (binary.parent.parent / "PkgInfo").write_text("BNDL????", encoding="utf-8")
                    (binary.parent.parent / "CodeResources").write_text("signed resources", encoding="utf-8")
                    binary.write_bytes(b"arm64")
                    os.chmod(binary, 0o755)
                return subprocess.CompletedProcess(args, 0, "", "")

            with self.assertRaisesRegex(ValueError, r"vst3 ZIP contains unexpected topology: pump\.vst3/Contents/CodeResources"), mock.patch.object(release_helper.platform, "system", return_value="Darwin"), mock.patch.object(release_helper, "_run_checked", side_effect=run):
                release_helper._audit_zip(archive, "vst3", "TEAM123456", cwd=root)

    def test_zip_audit_rejects_clap_code_resources_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "pump-v0.2.0-macos.clap.zip"
            archive.write_bytes(b"placeholder")

            def run(args, *, cwd, capture_output=False):
                if args[0] == "/usr/bin/ditto":
                    extracted = Path(args[-1])
                    binary = extracted / "pump.clap" / "Contents" / "MacOS" / "pump"
                    binary.parent.mkdir(parents=True)
                    (binary.parent.parent / "Info.plist").write_text("plist", encoding="utf-8")
                    (binary.parent.parent / "PkgInfo").write_text("BNDL????", encoding="utf-8")
                    (binary.parent.parent / "CodeResources").mkdir()
                    binary.write_bytes(b"arm64")
                    os.chmod(binary, 0o755)
                return subprocess.CompletedProcess(args, 0, "", "")

            with self.assertRaisesRegex(ValueError, r"clap ZIP Contents/CodeResources must be a regular file"), mock.patch.object(release_helper.platform, "system", return_value="Darwin"), mock.patch.object(release_helper, "_run_checked", side_effect=run):
                release_helper._audit_zip(archive, "clap", "TEAM123456", cwd=root)

    def test_zip_audit_rejects_wrong_team_before_stapler(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive = root / "pump-v0.2.0-macos.clap.zip"; archive.write_bytes(b"placeholder")

            def run(args, *, cwd, capture_output=False):
                if args[0] == "/usr/bin/ditto":
                    extracted = Path(args[-1]); binary = extracted / "pump.clap" / "Contents" / "MacOS" / "pump"; binary.parent.mkdir(parents=True)
                    (binary.parent.parent / "Info.plist").write_text("plist", encoding="utf-8"); (binary.parent.parent / "PkgInfo").write_text("BNDL????", encoding="utf-8"); binary.write_bytes(b"arm64"); os.chmod(binary, 0o755)
                if args[0] == "/usr/bin/plutil" and "CFBundleIdentifier" in args: return subprocess.CompletedProcess(args, 0, "com.portalsurfer.pump.clap\n", "")
                if args[0] == "/usr/bin/plutil" and "CFBundlePackageType" in args: return subprocess.CompletedProcess(args, 0, "BNDL\n", "")
                if args[0] == "codesign" and args[1] == "-dv": return subprocess.CompletedProcess(args, 0, "", "Authority=Developer ID Application: PORTALSURFER\nTeamIdentifier=OTHERTEAM\n")
                return subprocess.CompletedProcess(args, 0, "", "")

            with self.assertRaisesRegex(ValueError, "team does not match"), mock.patch.object(release_helper.platform, "system", return_value="Darwin"), mock.patch.object(release_helper, "_run_checked", side_effect=run):
                release_helper._audit_zip(archive, "clap", "TEAM123456", cwd=root)

    def test_source_gate_fetches_origin_before_comparing_refs(self):
        sha = "a" * 40
        manifest = {"source": {"git_sha": sha}}
        calls = []

        def run(args, *, cwd, capture_output=False):
            calls.append(tuple(args))
            output = {
                ("git", "symbolic-ref", "--quiet", "--short", "HEAD"): "main\n",
                ("git", "status", "--porcelain", "--untracked-files=all"): "",
                ("git", "rev-parse", "HEAD"): f"{sha}\n",
                ("git", "rev-parse", "refs/remotes/origin/main"): f"{sha}\n",
            }.get(tuple(args), "")
            return subprocess.CompletedProcess(args, 0, output, "")

        with mock.patch.object(release_helper, "_run_checked", side_effect=run):
            release_helper._validate_canonical_source(manifest, Path("/repo"))
        self.assertEqual(calls[2], ("git", "fetch", "origin", "main", "--quiet"))

    def test_source_gate_reports_exact_dirty_status_entries(self):
        sha = "a" * 40
        manifest = {"source": {"git_sha": sha}}
        status = " M scripts/release.sh\n?? target/release-note.txt\n"

        def run(args, *, cwd, capture_output=False):
            output = status if args[:2] == ("git", "status") else "main\n"
            return subprocess.CompletedProcess(args, 0, output, "")

        with mock.patch.object(release_helper, "_run_checked", side_effect=run):
            with self.assertRaisesRegex(
                ValueError,
                r"production release source must be clean; git status entries:  M scripts/release\.sh \| \?\? target/release-note\.txt",
            ):
                release_helper._validate_canonical_source(manifest, Path("/repo"))

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
