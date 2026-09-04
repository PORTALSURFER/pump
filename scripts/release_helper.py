#!/usr/bin/env python3
"""Dependency-free manifest and PortalSurfer transport helpers for Pump."""

from __future__ import annotations

import ast
import datetime as dt
import hashlib
import json
import os
import platform
import re
import struct
import subprocess
import tempfile
import zlib
from pathlib import Path, PurePosixPath
from typing import Any, Callable, Mapping, Optional, Sequence
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


MANIFEST_SCHEMA = 2
MANIFEST_SCHEMA_V2 = 2
MANIFEST_SCHEMA_V3 = 3
MANIFEST_CONTENT_TYPES = {
    MANIFEST_SCHEMA_V2: "application/vnd.portalsurfer.release-manifest+json;version=2",
    MANIFEST_SCHEMA_V3: "application/vnd.portalsurfer.release-manifest+json;version=3",
}
MANIFEST_CONTENT_TYPE = MANIFEST_CONTENT_TYPES[MANIFEST_SCHEMA_V2]
PRODUCTION_ORIGIN = "https://portalsurfer.org"
PRODUCT = "pump"
REPOSITORY = "PORTALSURFER/pump"
SAFE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*\Z")
SAFE_BUILD_ID = re.compile(r"[a-z0-9][a-z0-9._-]{1,127}\Z")
SHA256 = re.compile(r"[0-9a-f]{64}\Z")
TEAM_ID = re.compile(r"[A-Z0-9]{10}\Z")
NOTARY_ID = re.compile(
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[1-5][0-9a-fA-F]{3}-"
    r"[89abAB][0-9a-fA-F]{3}-[0-9a-fA-F]{12}\Z"
)
BASE_VERSION = r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)"
SEMVER = re.compile(rf"{BASE_VERSION}(?:-[0-9A-Za-z.-]+)?\Z")
CORE_SEMVER = re.compile(rf"{BASE_VERSION}\Z")
RELEASE_CHANNELS = frozenset(("stable", "rc", "nightly"))
CHANNEL_VERSION_PATTERNS = {
    "stable": re.compile(rf"{BASE_VERSION}\Z"),
    "rc": re.compile(rf"{BASE_VERSION}\Z"),
    "nightly": re.compile(rf"{BASE_VERSION}-nightly\.[1-9][0-9]*\Z"),
}
NIGHTLY_NUMBER = re.compile(r"[1-9][0-9]*\Z")
NIGHTLY_VERSION = re.compile(
    rf"(?P<major>[0-9]+)\.(?P<minor>[0-9]+)\.(?P<patch>[0-9]+)-nightly\."
    rf"(?P<number>[1-9][0-9]*)\Z"
)
WORKFLOW_SEQUENCE = re.compile(r"[1-9][0-9]*\Z")
GIT_SHA = re.compile(r"[0-9a-f]{40}\Z")
SCREENSHOT_NAME = re.compile(r"pump-default-[0-9]+x[0-9]+\.png\Z")


def _parse_core_version(version: Any, *, field: str) -> tuple[int, int, int]:
    if not isinstance(version, str):
        raise ValueError(f"{field} must be a numeric semver")
    match = CORE_SEMVER.fullmatch(version)
    if match is None:
        raise ValueError(f"{field} must be a numeric semver")
    return tuple(int(part) for part in version.split("."))


def _format_core_version(version: tuple[int, int, int]) -> str:
    return ".".join(str(part) for part in version)


def _canonical_nightly_number(value: Any) -> str:
    if isinstance(value, bool):
        raise ValueError("nightly suffix must be a positive canonical decimal")
    if isinstance(value, int):
        if value <= 0:
            raise ValueError("nightly suffix must be a positive canonical decimal")
        return str(value)
    if isinstance(value, str) and NIGHTLY_NUMBER.fullmatch(value):
        return value
    raise ValueError("nightly suffix must be a positive canonical decimal")


def _parse_channel_version(version: Any, channel: Any, *, field: str) -> tuple[int, int, int]:
    if not isinstance(channel, str) or channel not in CHANNEL_VERSION_PATTERNS:
        raise ValueError(f"{field} has an invalid release channel")
    if not isinstance(version, str) or CHANNEL_VERSION_PATTERNS[channel].fullmatch(version) is None:
        if channel == "nightly":
            raise ValueError(f"{field} must be X.Y.Z-nightly.N with a positive canonical decimal suffix")
        raise ValueError(f"{field} must be a numeric semver")
    return _parse_core_version(version.split("-", 1)[0], field=field)


def validate_channel_version(version: str, channel: str) -> None:
    _parse_channel_version(version, channel, field="publication version")


def validate_publication_version(package_version: str, publication_version: str, channel: str) -> None:
    package_core = _parse_core_version(package_version, field="package version")
    publication_core = _parse_channel_version(publication_version, channel, field="publication version")
    if publication_core != package_core:
        raise ValueError(
            f"publication version {publication_version} does not match package version {package_version}"
        )
    if channel in {"stable", "rc"} and publication_version != package_version:
        raise ValueError(f"{channel} publication version must equal package version")


def derive_publication_version(package_version: str, channel: str, sequence: Any) -> str:
    if channel not in RELEASE_CHANNELS:
        raise ValueError(f"invalid release channel: {channel}")
    package_text = _format_core_version(_parse_core_version(package_version, field="package version"))
    if channel == "nightly":
        publication_version = f"{package_text}-nightly.{_canonical_nightly_number(sequence)}"
    else:
        publication_version = package_text
    validate_publication_version(package_text, publication_version, channel)
    return publication_version


def format_manifest_version(package_version: str, channel: str, nightly_number: Any = None) -> str:
    """Format the channel-specific version carried by a release manifest."""
    if channel not in RELEASE_CHANNELS:
        raise ValueError(f"invalid release channel: {channel}")
    package_text = _format_core_version(_parse_core_version(package_version, field="package version"))
    if channel == "nightly":
        return f"{package_text}-nightly.{_canonical_nightly_number(nightly_number)}"
    return package_text


def _parse_nightly_core_version(version: Any, *, field: str) -> tuple[int, int, int]:
    return _parse_channel_version(version, "nightly", field=field)


def validate_manifest_version(version: Any, channel: str) -> None:
    _parse_channel_version(version, channel, field="version")


def _parse_release_history_core_version(version: Any, *, channel: str) -> tuple[int, int, int]:
    if channel == "nightly" and isinstance(version, str) and NIGHTLY_VERSION.fullmatch(version):
        return _parse_nightly_core_version(version, field="release version")
    return _parse_core_version(version, field="release version")


def latest_release_source_sha(document: Any, *, channel: str) -> Optional[str]:
    """Return the newest validated source SHA for one release channel."""
    if channel not in RELEASE_CHANNELS:
        raise ValueError(f"invalid release channel: {channel}")
    if not isinstance(document, dict) or not isinstance(document.get("releases"), list):
        raise ValueError("release history must contain a releases array")
    latest: tuple[dt.datetime, str] | None = None
    for release in document["releases"]:
        if not isinstance(release, dict):
            raise ValueError("release history contains a non-object release")
        if release.get("channel") != channel:
            continue
        released_at = release.get("released_at")
        if not isinstance(released_at, str):
            raise ValueError("matching release is missing released_at")
        try:
            parsed = dt.datetime.fromisoformat(released_at.replace("Z", "+00:00"))
        except ValueError as error:
            raise ValueError("matching release released_at must be RFC3339") from error
        if parsed.tzinfo is None or parsed.utcoffset() is None:
            raise ValueError("matching release released_at must include a timezone")
        source = release.get("source")
        if not isinstance(source, dict) or source.get("repository") != REPOSITORY:
            raise ValueError("matching release source repository is invalid")
        source_sha = source.get("git_sha")
        if not isinstance(source_sha, str) or not GIT_SHA.fullmatch(source_sha):
            raise ValueError("matching release source git_sha is invalid")
        if latest is None or parsed > latest[0]:
            latest = (parsed, source_sha)
    return latest[1] if latest is not None else None


def should_release(
    *, source_sha: str, document: Any = None, channel: str = "nightly", only_if_changed: bool = True
) -> bool:
    if not isinstance(source_sha, str) or not GIT_SHA.fullmatch(source_sha):
        raise ValueError("checked-out source SHA is invalid")
    if not only_if_changed:
        return True
    latest = latest_release_source_sha(document, channel=channel)
    return latest is None or latest != source_sha


def latest_release_version(document: Any) -> Optional[str]:
    """Return the highest numeric core version in release history."""
    if not isinstance(document, dict) or not isinstance(document.get("releases"), list):
        raise ValueError("release history must contain a releases array")
    latest: tuple[int, int, int] | None = None
    for release in document["releases"]:
        if not isinstance(release, dict):
            raise ValueError("release history contains a non-object release")
        channel = release.get("channel")
        if channel not in RELEASE_CHANNELS:
            raise ValueError("release history contains an invalid channel")
        candidate = _parse_release_history_core_version(release.get("version"), channel=channel)
        if latest is None or candidate > latest:
            latest = candidate
    return _format_core_version(latest) if latest is not None else None


def next_release_version(package_version: str, document: Any) -> str:
    """Retained compatibility helper for callers that inspect release history."""
    current = _parse_core_version(package_version, field="package version")
    latest_text = latest_release_version(document)
    latest = _parse_core_version(latest_text, field="latest release version") if latest_text else None
    if latest is not None and current > latest:
        return _format_core_version(current)
    base = latest if latest is not None and latest > current else current
    return _format_core_version((base[0], base[1], base[2] + 1))


def validate_release_fields(
    version: str,
    released_at: str,
    names: list[str],
    hashes: list[str],
    sizes: list[int],
    channel: str = "stable",
) -> None:
    validate_manifest_version(version, channel)
    try:
        parsed = dt.datetime.fromisoformat(released_at.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("released_at must be RFC3339") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("released_at must include a timezone")
    if len(names) != len(set(names)) or any(not SAFE_NAME.fullmatch(name) for name in names):
        raise ValueError("release file names must be unique safe basenames")
    if any(not isinstance(value, str) or not SHA256.fullmatch(value) for value in hashes):
        raise ValueError("release hashes must be lowercase SHA-256")
    if any(not isinstance(size, int) or isinstance(size, bool) or size <= 0 for size in sizes):
        raise ValueError("release sizes must be positive integers")


def canonical_json(value: Any) -> bytes:
    """Encode JSON deterministically for manifests and commit bodies."""
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n").encode(
        "utf-8"
    )


def file_digest(path: Path) -> tuple[str, int]:
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    return digest.hexdigest(), size


def _validate_regular_file(path: Path, label: str) -> None:
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{label} must be a regular file: {path}")


def _validated_file_digest(path: Path, label: str) -> tuple[str, int]:
    _validate_regular_file(path, label)
    digest, size = file_digest(path)
    if size <= 0 or not SHA256.fullmatch(digest):
        raise ValueError(f"{label} is empty or has an invalid hash")
    return digest, size


def validate_png(path: Path, width: int = 640, height: int = 400) -> dict[str, Any]:
    """Validate the structural PNG contract used by Pump's default capture."""
    _validate_regular_file(path, "screenshot")
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    offset = 8
    seen_ihdr = seen_iend = seen_idat = False
    dimensions: tuple[int, int] | None = None
    dpi = 1.0
    while offset + 12 <= len(data):
        length = struct.unpack(">I", data[offset : offset + 4])[0]
        kind = data[offset + 4 : offset + 8]
        end = offset + 12 + length
        if end > len(data):
            raise ValueError(f"{path} has a truncated PNG chunk")
        payload = data[offset + 8 : offset + 8 + length]
        crc = struct.unpack(">I", data[offset + 8 + length : end])[0]
        if crc != (zlib.crc32(kind + payload) & 0xFFFFFFFF):
            raise ValueError(f"{path} has an invalid {kind.decode('ascii', 'replace')} CRC")
        if kind == b"IHDR":
            if offset != 8 or seen_ihdr or length != 13:
                raise ValueError(f"{path} has an invalid IHDR")
            seen_ihdr = True
            dimensions = struct.unpack(">II", payload[:8])
            if payload[8] != 8 or payload[9] not in (2, 6) or payload[10:] != b"\x00\x00\x00":
                raise ValueError(f"{path} must use 8-bit RGB or RGBA pixels")
        elif kind == b"pHYs" and length == 9:
            x, y, unit = struct.unpack(">IIB", payload)
            if unit == 1:
                if x == 0 or y == 0 or x != y:
                    raise ValueError(f"{path} has a non-1.0 pixel scale")
                dpi = 1.0
        elif kind == b"IDAT":
            if not seen_ihdr:
                raise ValueError(f"{path} IDAT precedes IHDR")
            seen_idat = True
        elif kind == b"IEND":
            if length != 0 or end != len(data):
                raise ValueError(f"{path} has an invalid IEND")
            seen_iend = True
            break
        offset = end
    if not seen_ihdr or not seen_idat or not seen_iend or dimensions != (width, height):
        raise ValueError(f"{path} must be {width}x{height} with IHDR, IDAT, and IEND")
    digest, size = file_digest(path)
    return {"width": width, "height": height, "dpi": dpi, "hash": digest, "size": size}


def _validate_common(
    *, product: str, repository: str, version: str, build_id: str, channel: str, released_at: str, git_sha: str
) -> None:
    if product != PRODUCT or repository != REPOSITORY:
        raise ValueError("unknown or mismatched Portal product")
    if not isinstance(version, str) or SEMVER.fullmatch(version) is None or "+" in version:
        raise ValueError("version must be SemVer without build metadata")
    validate_manifest_version(version, channel)
    if not isinstance(build_id, str) or not SAFE_BUILD_ID.fullmatch(build_id):
        raise ValueError("source SHA or build id is invalid")
    if not isinstance(git_sha, str) or not GIT_SHA.fullmatch(git_sha):
        raise ValueError("source SHA or build id is invalid")
    try:
        parsed = dt.datetime.fromisoformat(released_at.replace("Z", "+00:00"))
    except ValueError as error:
        raise ValueError("released_at must be RFC3339") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise ValueError("released_at must include a timezone")


def _screenshot_metadata(screenshot: Path, git_sha: str) -> dict[str, Any]:
    if screenshot.name != "pump-default-640x400.png" or not SCREENSHOT_NAME.fullmatch(screenshot.name):
        raise ValueError("screenshot name must be pump-default-640x400.png")
    info = validate_png(screenshot)
    return {
        "role": "default-ui",
        "name": screenshot.name,
        "media_type": "image/png",
        "width": info["width"],
        "height": info["height"],
        "logical_width": info["width"],
        "logical_height": info["height"],
        "dpi_scale": info["dpi"],
        "source_git_sha": git_sha,
        "sha256": info["hash"],
        "size_bytes": info["size"],
    }


def _changelog_metadata(changelog: Path) -> dict[str, Any]:
    if changelog.name != "CHANGELOG.md":
        raise ValueError("CHANGELOG.md is missing or invalid")
    digest, size = _validated_file_digest(changelog, "CHANGELOG.md")
    return {
        "name": "CHANGELOG.md",
        "format": "markdown",
        "media_type": "text/markdown; charset=utf-8",
        "sha256": digest,
        "size_bytes": size,
    }


def _schema2_signing(
    *,
    distribution: str,
    signing_identity_class: str,
    notarized: bool,
    stapled: bool,
    signing_team_id: str,
    notary_submissions: Optional[dict[str, str]],
) -> dict[str, Any]:
    submissions = notary_submissions or {}
    if distribution == "production":
        if (
            signing_identity_class != "Developer ID Application"
            or notarized is not True
            or stapled is not True
            or not TEAM_ID.fullmatch(signing_team_id)
            or set(submissions) != {"clap", "vst3"}
            or any(not isinstance(value, str) or not NOTARY_ID.fullmatch(value) for value in submissions.values())
        ):
            raise ValueError("production manifests require Developer ID signing and a stapled notarization")
    elif distribution == "preflight":
        if signing_identity_class != "ad hoc" or notarized or stapled or signing_team_id or submissions:
            raise ValueError("preflight manifests require ad hoc, non-notarized provenance")
    else:
        raise ValueError("invalid distribution")
    return {
        "identity_class": signing_identity_class,
        "notarized": notarized,
        "stapled": stapled,
        "team_id": signing_team_id,
        "notary_submissions": submissions,
    }


def _schema2_artifact(*, format_name: str, path: Path, version: str, channel: str) -> dict[str, Any]:
    core_version = _format_core_version(_parse_nightly_core_version(version, field="nightly version")) if channel == "nightly" else version
    expected_name = f"pump-v{core_version}-macos.{format_name}.zip"
    if path.name != expected_name or not SAFE_NAME.fullmatch(path.name):
        raise ValueError(f"{format_name} artifact must be named {expected_name}")
    digest, size = _validated_file_digest(path, f"{format_name} artifact")
    return {
        "format": format_name,
        "platform": "macos",
        "architectures": ["arm64"],
        "name": path.name,
        "media_type": "application/zip",
        "sha256": digest,
        "size_bytes": size,
    }


def _build_schema2_manifest(
    *,
    version: str,
    build_id: str,
    channel: str,
    released_at: str,
    git_sha: str,
    clap: Path,
    vst3: Path,
    screenshot: Path,
    changelog: Path,
    distribution: str,
    signing_identity_class: str,
    notarized: bool,
    stapled: bool,
    signing_team_id: str,
    notary_submissions: Optional[dict[str, str]],
) -> dict[str, Any]:
    _validate_common(
        product=PRODUCT,
        repository=REPOSITORY,
        version=version,
        build_id=build_id,
        channel=channel,
        released_at=released_at,
        git_sha=git_sha,
    )
    signing = _schema2_signing(
        distribution=distribution,
        signing_identity_class=signing_identity_class,
        notarized=notarized,
        stapled=stapled,
        signing_team_id=signing_team_id,
        notary_submissions=notary_submissions,
    )
    artifacts = [
        _schema2_artifact(format_name="clap", path=clap, version=version, channel=channel),
        _schema2_artifact(format_name="vst3", path=vst3, version=version, channel=channel),
    ]
    screenshot_metadata = _screenshot_metadata(screenshot, git_sha)
    changelog_metadata = _changelog_metadata(changelog)
    names = [artifact["name"] for artifact in artifacts] + [screenshot.name, changelog.name]
    if len(names) != len(set(names)):
        raise ValueError("release file names must be unique")
    return {
        "schema_version": MANIFEST_SCHEMA_V2,
        "product": PRODUCT,
        "build_id": build_id,
        "version": version,
        "channel": channel,
        "released_at": released_at,
        "source": {"repository": REPOSITORY, "git_sha": git_sha, "dirty": False},
        "distribution": distribution,
        "signing": signing,
        "artifacts": artifacts,
        "screenshot": screenshot_metadata,
        "changelog": changelog_metadata,
    }


def _build_schema3_manifest(
    *,
    version: str,
    build_id: str,
    channel: str,
    released_at: str,
    git_sha: str,
    clap: Path,
    vst3: Path,
    windows_vst3: Path,
    screenshot: Path,
    changelog: Path,
    signing_team_id: str,
    notary_submissions: Optional[dict[str, str]],
) -> dict[str, Any]:
    _validate_common(
        product=PRODUCT,
        repository=REPOSITORY,
        version=version,
        build_id=build_id,
        channel=channel,
        released_at=released_at,
        git_sha=git_sha,
    )
    expected_build_id = f"pump-v{version}-{git_sha[:12]}"
    if channel != "nightly" or build_id != expected_build_id:
        raise ValueError(f"schema 3 build id must be {expected_build_id}")
    if not TEAM_ID.fullmatch(signing_team_id):
        raise ValueError("schema 3 requires a valid Apple team ID")
    submissions = notary_submissions or {}
    if set(submissions) != {"clap", "vst3"} or any(
        not isinstance(value, str) or not NOTARY_ID.fullmatch(value) for value in submissions.values()
    ):
        raise ValueError("schema 3 signing/notarization evidence is incomplete")

    expected_names = {
        "clap": f"pump-v{version}-macos.clap.zip",
        "vst3": f"pump-v{version}-macos.vst3.zip",
        "windows": f"pump-v{version}-windows-x86_64-unsigned.vst3.zip",
    }
    paths = {"clap": clap, "vst3": vst3, "windows": windows_vst3}
    hashes: dict[str, tuple[str, int]] = {}
    for key, path in paths.items():
        if path.name != expected_names[key] or not SAFE_NAME.fullmatch(path.name):
            raise ValueError(f"{key} artifact name does not match the schema 3 identity")
        hashes[key] = _validated_file_digest(path, f"{key} artifact")

    artifacts = [
        {
            "format": "clap",
            "platform": "macos",
            "architectures": ["arm64"],
            "name": clap.name,
            "media_type": "application/zip",
            "sha256": hashes["clap"][0],
            "size_bytes": hashes["clap"][1],
            "security": {
                "status": "signed",
                "certificate": "Developer ID Application",
                "team_id": signing_team_id,
                "notarized": True,
                "stapled": True,
                "notary_submission": submissions["clap"],
            },
        },
        {
            "format": "vst3",
            "platform": "macos",
            "architectures": ["arm64"],
            "name": vst3.name,
            "media_type": "application/zip",
            "sha256": hashes["vst3"][0],
            "size_bytes": hashes["vst3"][1],
            "security": {
                "status": "signed",
                "certificate": "Developer ID Application",
                "team_id": signing_team_id,
                "notarized": True,
                "stapled": True,
                "notary_submission": submissions["vst3"],
            },
        },
        {
            "format": "vst3",
            "platform": "windows",
            "architectures": ["x86_64"],
            "name": windows_vst3.name,
            "media_type": "application/zip",
            "sha256": hashes["windows"][0],
            "size_bytes": hashes["windows"][1],
            "security": {"status": "unsigned", "certificate": None},
        },
    ]
    screenshot_metadata = _screenshot_metadata(screenshot, git_sha)
    changelog_metadata = _changelog_metadata(changelog)
    names = [artifact["name"] for artifact in artifacts] + [screenshot.name, changelog.name]
    if len(names) != len(set(names)):
        raise ValueError("release file names must be unique")
    return {
        "schema_version": MANIFEST_SCHEMA_V3,
        "product": PRODUCT,
        "build_id": build_id,
        "version": version,
        "channel": channel,
        "released_at": released_at,
        "source": {"repository": REPOSITORY, "git_sha": git_sha, "dirty": False},
        "distribution": "production",
        "artifacts": artifacts,
        "screenshot": screenshot_metadata,
        "changelog": changelog_metadata,
    }


def build_manifest(
    *,
    version: str,
    build_id: str,
    channel: str,
    released_at: str,
    git_sha: str,
    clap: Path,
    vst3: Path,
    screenshot: Path,
    changelog: Path,
    distribution: str = "production",
    signing_identity_class: str = "Developer ID Application",
    notarized: bool = True,
    stapled: bool = True,
    signing_team_id: str = "",
    notary_submissions: Optional[dict[str, str]] = None,
    windows_vst3: Optional[Path] = None,
) -> dict[str, Any]:
    if windows_vst3 is None:
        if channel == "nightly" and distribution == "production":
            raise ValueError("production Pump nightly requires the Windows artifact")
        return _build_schema2_manifest(
            version=version,
            build_id=build_id,
            channel=channel,
            released_at=released_at,
            git_sha=git_sha,
            clap=clap,
            vst3=vst3,
            screenshot=screenshot,
            changelog=changelog,
            distribution=distribution,
            signing_identity_class=signing_identity_class,
            notarized=notarized,
            stapled=stapled,
            signing_team_id=signing_team_id,
            notary_submissions=notary_submissions,
        )
    if channel != "nightly" or distribution != "production":
        raise ValueError("schema 3 is only available for production Pump nightlies")
    return _build_schema3_manifest(
        version=version,
        build_id=build_id,
        channel=channel,
        released_at=released_at,
        git_sha=git_sha,
        clap=clap,
        vst3=vst3,
        windows_vst3=windows_vst3,
        screenshot=screenshot,
        changelog=changelog,
        signing_team_id=signing_team_id,
        notary_submissions=notary_submissions,
    )


def _validate_schema2_manifest(manifest: Mapping[str, Any], root: Path, *, require_production: bool) -> None:
    required = {
        "schema_version",
        "product",
        "build_id",
        "version",
        "channel",
        "released_at",
        "source",
        "distribution",
        "signing",
        "artifacts",
        "screenshot",
        "changelog",
    }
    if set(manifest) != required or manifest.get("schema_version") != MANIFEST_SCHEMA_V2:
        raise ValueError("manifest schema or fields are invalid")
    source = manifest["source"]
    if not isinstance(source, dict) or set(source) != {"repository", "git_sha", "dirty"} or source.get("dirty") is not False:
        raise ValueError("manifest source is invalid")
    _validate_common(
        product=manifest["product"],
        repository=source.get("repository"),
        version=manifest["version"],
        build_id=manifest["build_id"],
        channel=manifest["channel"],
        released_at=manifest["released_at"],
        git_sha=source.get("git_sha"),
    )
    distribution = manifest["distribution"]
    signing = manifest["signing"]
    expected_preflight = {
        "identity_class": "ad hoc",
        "notarized": False,
        "stapled": False,
        "team_id": "",
        "notary_submissions": {},
    }
    if not isinstance(signing, dict) or set(signing) != set(expected_preflight):
        raise ValueError("manifest signing fields are invalid")
    if distribution == "production":
        if (
            signing["identity_class"] != "Developer ID Application"
            or signing["notarized"] is not True
            or signing["stapled"] is not True
            or not TEAM_ID.fullmatch(signing["team_id"])
            or set(signing["notary_submissions"]) != {"clap", "vst3"}
            or any(not isinstance(value, str) or not NOTARY_ID.fullmatch(value) for value in signing["notary_submissions"].values())
        ):
            raise ValueError("production signing evidence is invalid")
        if not require_production:
            raise ValueError("preflight requires ad hoc non-notarized provenance")
    elif distribution == "preflight":
        if signing != expected_preflight:
            raise ValueError("preflight signing evidence is invalid")
        if require_production:
            raise ValueError("publish requires production Developer ID notarized provenance")
    else:
        raise ValueError("manifest distribution is invalid")

    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list) or len(artifacts) != 2:
        raise ValueError("publish requires exactly CLAP and VST3 artifacts")
    formats = {artifact.get("format") for artifact in artifacts if isinstance(artifact, dict)}
    if formats != {"clap", "vst3"} or not all(isinstance(artifact, dict) for artifact in artifacts):
        raise ValueError("publish requires exactly CLAP and VST3 artifacts")
    expected_core = (
        _format_core_version(_parse_nightly_core_version(manifest["version"], field="nightly version"))
        if manifest["channel"] == "nightly"
        else manifest["version"]
    )
    expected_names = {
        "clap": f"pump-v{expected_core}-macos.clap.zip",
        "vst3": f"pump-v{expected_core}-macos.vst3.zip",
    }
    entries: list[Mapping[str, Any]] = []
    for artifact in artifacts:
        if set(artifact) != {"format", "platform", "architectures", "name", "media_type", "sha256", "size_bytes"}:
            raise ValueError("macOS artifact metadata is invalid")
        if (
            artifact["platform"] != "macos"
            or artifact["architectures"] != ["arm64"]
            or artifact["media_type"] != "application/zip"
            or artifact["name"] != expected_names[artifact["format"]]
        ):
            raise ValueError("macOS artifact metadata is invalid")
        entries.append(artifact)

    screenshot = manifest["screenshot"]
    screenshot_fields = {
        "role", "name", "media_type", "width", "height", "logical_width", "logical_height",
        "dpi_scale", "source_git_sha", "sha256", "size_bytes",
    }
    if (
        not isinstance(screenshot, dict)
        or set(screenshot) != screenshot_fields
        or screenshot["role"] != "default-ui"
        or screenshot["name"] != "pump-default-640x400.png"
        or screenshot["media_type"] != "image/png"
        or screenshot["width"] != 640
        or screenshot["height"] != 400
        or screenshot["logical_width"] != 640
        or screenshot["logical_height"] != 400
        or screenshot["dpi_scale"] != 1.0
        or screenshot["source_git_sha"] != source["git_sha"]
        or not SHA256.fullmatch(screenshot["sha256"])
        or not isinstance(screenshot["size_bytes"], int)
        or isinstance(screenshot["size_bytes"], bool)
        or screenshot["size_bytes"] <= 0
    ):
        raise ValueError("screenshot metadata is invalid")
    changelog = manifest["changelog"]
    if (
        not isinstance(changelog, dict)
        or set(changelog) != {"name", "format", "media_type", "sha256", "size_bytes"}
        or changelog["name"] != "CHANGELOG.md"
        or changelog["format"] != "markdown"
        or changelog["media_type"] != "text/markdown; charset=utf-8"
        or not SHA256.fullmatch(changelog["sha256"])
        or not isinstance(changelog["size_bytes"], int)
        or isinstance(changelog["size_bytes"], bool)
        or changelog["size_bytes"] <= 0
    ):
        raise ValueError("changelog metadata is invalid")

    all_entries = [*entries, screenshot, changelog]
    validate_release_fields(
        manifest["version"],
        manifest["released_at"],
        [entry["name"] for entry in all_entries],
        [entry["sha256"] for entry in all_entries],
        [entry["size_bytes"] for entry in all_entries],
        channel=manifest["channel"],
    )
    for entry in all_entries:
        path = root / entry["name"]
        _validate_regular_file(path, "release file")
        digest, size = file_digest(path)
        if digest != entry["sha256"] or size != entry["size_bytes"]:
            raise ValueError(f"manifest hash/size mismatch: {entry['name']}")
    png_info = validate_png(root / screenshot["name"])
    if png_info["width"] != 640 or png_info["height"] != 400:
        raise ValueError("screenshot dimensions do not match its manifest")
    manifest_path = root / "release-manifest.json"
    if manifest_path.is_file():
        _validate_regular_file(manifest_path, "release manifest")
        if manifest_path.read_bytes() != canonical_json(dict(manifest)):
            raise ValueError("release-manifest.json is not canonical JSON")


def _validate_schema3_manifest(manifest: Mapping[str, Any], root: Path) -> None:
    required = {
        "schema_version", "product", "build_id", "version", "channel", "released_at", "source",
        "distribution", "artifacts", "screenshot", "changelog",
    }
    if set(manifest) != required or manifest.get("schema_version") != MANIFEST_SCHEMA_V3:
        raise ValueError("schema 3 manifest fields are invalid")
    source = manifest["source"]
    if not isinstance(source, dict) or set(source) != {"repository", "git_sha", "dirty"} or source.get("dirty") is not False:
        raise ValueError("manifest source is invalid")
    _validate_common(
        product=manifest["product"],
        repository=source.get("repository"),
        version=manifest["version"],
        build_id=manifest["build_id"],
        channel=manifest["channel"],
        released_at=manifest["released_at"],
        git_sha=source.get("git_sha"),
    )
    expected_build_id = f"pump-v{manifest['version']}-{source['git_sha'][:12]}"
    if manifest["channel"] != "nightly" or manifest["distribution"] != "production":
        raise ValueError("schema 3 is only available for production Pump nightlies")
    if manifest["build_id"] != expected_build_id:
        raise ValueError(f"schema 3 build id must be {expected_build_id}")

    artifacts = manifest["artifacts"]
    if not isinstance(artifacts, list) or len(artifacts) != 3:
        raise ValueError("schema 3 manifest must contain exactly three artifacts")
    expected_targets = {
        ("macos", "arm64", "clap"): f"pump-v{manifest['version']}-macos.clap.zip",
        ("macos", "arm64", "vst3"): f"pump-v{manifest['version']}-macos.vst3.zip",
        ("windows", "x86_64", "vst3"): f"pump-v{manifest['version']}-windows-x86_64-unsigned.vst3.zip",
    }
    seen_targets: set[tuple[str, str, str]] = set()
    mac_team_id: str | None = None
    names: set[str] = set()
    for artifact in artifacts:
        required_artifact_fields = {
            "format", "platform", "architectures", "name", "media_type", "sha256", "size_bytes", "security"
        }
        if (
            not isinstance(artifact, dict)
            or set(artifact) != required_artifact_fields
            or not isinstance(artifact.get("format"), str)
            or not isinstance(artifact.get("platform"), str)
            or artifact.get("media_type") != "application/zip"
            or artifact.get("architectures") not in (["arm64"], ["x86_64"])
            or not isinstance(artifact.get("name"), str)
        ):
            raise ValueError("schema 3 artifact metadata is invalid")
        target = (artifact["platform"], artifact["architectures"][0], artifact["format"])
        if target not in expected_targets or target in seen_targets or artifact["name"] != expected_targets[target] or artifact["name"] in names:
            raise ValueError("schema 3 artifact identity is invalid")
        security = artifact["security"]
        if target[0] == "windows":
            if security != {"status": "unsigned", "certificate": None}:
                raise ValueError("schema 3 Windows security metadata is invalid")
        else:
            if (
                not isinstance(security, dict)
                or set(security) != {"status", "certificate", "team_id", "notarized", "stapled", "notary_submission"}
                or security["status"] != "signed"
                or security["certificate"] != "Developer ID Application"
                or not TEAM_ID.fullmatch(security["team_id"])
                or security["notarized"] is not True
                or security["stapled"] is not True
                or not isinstance(security["notary_submission"], str)
                or not NOTARY_ID.fullmatch(security["notary_submission"])
            ):
                raise ValueError("schema 3 macOS security metadata is invalid")
            if mac_team_id is None:
                mac_team_id = security["team_id"]
            elif security["team_id"] != mac_team_id:
                raise ValueError("schema 3 macOS signing teams differ")
        if not SHA256.fullmatch(artifact["sha256"]) or not isinstance(artifact["size_bytes"], int) or isinstance(artifact["size_bytes"], bool) or artifact["size_bytes"] <= 0:
            raise ValueError("schema 3 artifact hash or size is invalid")
        path = root / artifact["name"]
        _validate_regular_file(path, "schema 3 artifact")
        digest, size = file_digest(path)
        if digest != artifact["sha256"] or size != artifact["size_bytes"]:
            raise ValueError(f"manifest hash/size mismatch: {artifact['name']}")
        seen_targets.add(target)
        names.add(artifact["name"])
    if seen_targets != set(expected_targets):
        raise ValueError("schema 3 artifact targets are incomplete")

    screenshot = manifest["screenshot"]
    screenshot_fields = {
        "role", "name", "media_type", "width", "height", "logical_width", "logical_height",
        "dpi_scale", "source_git_sha", "sha256", "size_bytes",
    }
    if (
        not isinstance(screenshot, dict)
        or set(screenshot) != screenshot_fields
        or screenshot["role"] != "default-ui"
        or screenshot["name"] in names
        or screenshot["name"] != "pump-default-640x400.png"
        or screenshot["media_type"] != "image/png"
        or screenshot["width"] != 640
        or screenshot["height"] != 400
        or screenshot["logical_width"] != 640
        or screenshot["logical_height"] != 400
        or screenshot["dpi_scale"] != 1.0
        or screenshot["source_git_sha"] != source["git_sha"]
        or not SHA256.fullmatch(screenshot["sha256"])
        or not isinstance(screenshot["size_bytes"], int)
        or isinstance(screenshot["size_bytes"], bool)
        or screenshot["size_bytes"] <= 0
    ):
        raise ValueError("screenshot metadata is invalid")
    _validate_regular_file(root / screenshot["name"], "screenshot")
    digest, size = file_digest(root / screenshot["name"])
    if digest != screenshot["sha256"] or size != screenshot["size_bytes"]:
        raise ValueError("manifest hash/size mismatch: pump-default-640x400.png")
    validate_png(root / screenshot["name"])
    names.add(screenshot["name"])

    changelog = manifest["changelog"]
    if (
        not isinstance(changelog, dict)
        or set(changelog) != {"name", "format", "media_type", "sha256", "size_bytes"}
        or changelog["name"] != "CHANGELOG.md"
        or changelog["name"] in names
        or changelog["format"] != "markdown"
        or changelog["media_type"] != "text/markdown; charset=utf-8"
        or not SHA256.fullmatch(changelog["sha256"])
        or not isinstance(changelog["size_bytes"], int)
        or isinstance(changelog["size_bytes"], bool)
        or changelog["size_bytes"] <= 0
    ):
        raise ValueError("changelog metadata is invalid")
    _validate_regular_file(root / changelog["name"], "CHANGELOG.md")
    digest, size = file_digest(root / changelog["name"])
    if digest != changelog["sha256"] or size != changelog["size_bytes"]:
        raise ValueError("manifest hash/size mismatch: CHANGELOG.md")

    manifest_path = root / "release-manifest.json"
    if manifest_path.is_file():
        _validate_regular_file(manifest_path, "release manifest")
        if manifest_path.read_bytes() != canonical_json(dict(manifest)):
            raise ValueError("release-manifest.json is not canonical JSON")


def validate_manifest(manifest: Mapping[str, Any], root: Path) -> None:
    if not isinstance(manifest, Mapping):
        raise ValueError("manifest must be an object")
    schema = manifest.get("schema_version")
    if schema == MANIFEST_SCHEMA_V2:
        _validate_schema2_manifest(
            manifest, root, require_production=manifest.get("distribution") == "production"
        )
    elif schema == MANIFEST_SCHEMA_V3:
        _validate_schema3_manifest(manifest, root)
    else:
        raise ValueError("manifest schema or fields are invalid")


def _validate_exact_manifest_names(manifest: Mapping[str, Any]) -> None:
    if manifest.get("schema_version") == MANIFEST_SCHEMA_V3:
        version = manifest.get("version")
        expected = {
            "clap-macos": f"pump-v{version}-macos.clap.zip",
            "vst3-macos": f"pump-v{version}-macos.vst3.zip",
            "vst3-windows": f"pump-v{version}-windows-x86_64-unsigned.vst3.zip",
        }
        actual = {
            f"{artifact.get('format')}-{artifact.get('platform')}": artifact.get("name")
            for artifact in manifest.get("artifacts", [])
        }
        if actual != expected:
            raise ValueError("manifest artifact names do not match the exact Pump ZIP contract")
    else:
        channel = manifest.get("channel")
        version = manifest.get("version")
        if channel == "nightly":
            version = _format_core_version(_parse_nightly_core_version(version, field="nightly version"))
        expected = {"clap": f"pump-v{version}-macos.clap.zip", "vst3": f"pump-v{version}-macos.vst3.zip"}
        actual = {artifact.get("format"): artifact.get("name") for artifact in manifest.get("artifacts", [])}
        if actual != expected:
            raise ValueError("manifest artifact names do not match the exact Pump ZIP contract")
    if manifest.get("screenshot", {}).get("role") != "default-ui" or manifest.get("screenshot", {}).get("name") != "pump-default-640x400.png" or manifest.get("changelog", {}).get("name") != "CHANGELOG.md":
        raise ValueError("manifest support-file roles or names do not match the Pump release contract")


def _request(url: str, method: str, body: Optional[bytes], headers: dict[str, str]) -> tuple[int, bytes]:
    request = Request(url, method=method, data=body, headers=headers)
    try:
        with urlopen(request, timeout=60) as response:
            return response.status, response.read()
    except (HTTPError, URLError) as error:
        if isinstance(error, HTTPError):
            detail = error.read().decode("utf-8", "replace")
            raise RuntimeError(f"{method} {url} failed ({error.code}): {detail[:400]}") from error
        raise RuntimeError(f"{method} {url} failed: {error.reason}") from error


Transport = Callable[[str, str, Optional[bytes], dict[str, str]], tuple[int, bytes]]


def _publish_validated_manifest(
    *, endpoint: str, token: str, manifest: Mapping[str, Any], root: Path, transport: Transport
) -> None:
    status, payload = transport(
        f"{endpoint}/plugins/api/v1/products/{PRODUCT}/releases", "GET", None, {"Accept": "application/json"}
    )
    if status < 200 or status >= 300:
        raise RuntimeError(f"capability check failed ({status})")
    try:
        capability = json.loads(payload)
    except json.JSONDecodeError as error:
        raise RuntimeError("capability response was not JSON") from error
    versions = capability.get("release_upload", {}).get("manifest_schema_versions", [])
    if MANIFEST_SCHEMA_V2 not in versions:
        raise RuntimeError("server does not support release manifest schema 2; no files were uploaded")

    files: list[tuple[str, Path, str]] = []
    for artifact in manifest["artifacts"]:
        files.append((artifact["name"], root / artifact["name"], artifact["sha256"]))
    for entry in (manifest["screenshot"], manifest["changelog"]):
        files.append((entry["name"], root / entry["name"], entry["sha256"]))
    metadata = {
        "Authorization": f"Bearer {token}",
        "Content-Type": "application/octet-stream",
        "X-PortalSurfer-Release-Version": manifest["version"],
        "X-PortalSurfer-Release-Channel": manifest["channel"],
        "X-PortalSurfer-Released-At": manifest["released_at"],
    }
    for name, path, digest in files:
        data = path.read_bytes()
        transport(
            f"{endpoint}/plugins/api/v1/products/{PRODUCT}/release-uploads/{manifest['build_id']}/staging/files/{name}",
            "PUT",
            data,
            {**metadata, "Content-Length": str(len(data)), "X-PortalSurfer-Sha256": digest},
        )
    body = canonical_json(dict(manifest))
    transport(
        f"{endpoint}/plugins/api/v1/products/{PRODUCT}/release-uploads/{manifest['build_id']}/commit",
        "PUT",
        body,
        {
            "Authorization": f"Bearer {token}",
            "Content-Type": MANIFEST_CONTENT_TYPE,
            "Content-Length": str(len(body)),
            "X-PortalSurfer-Manifest-Sha256": hashlib.sha256(body).hexdigest(),
            "X-PortalSurfer-Release-Version": manifest["version"],
            "X-PortalSurfer-Release-Channel": manifest["channel"],
            "X-PortalSurfer-Released-At": manifest["released_at"],
        },
    )


def _run_checked(args: Sequence[str], *, cwd: Path, capture_output: bool = False) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(list(args), cwd=str(cwd), check=True, capture_output=capture_output, text=True)
    except FileNotFoundError as error:
        raise RuntimeError(f"required release command is unavailable: {args[0]}") from error
    except subprocess.CalledProcessError as error:
        detail = (error.stderr or error.stdout or "").strip()
        raise ValueError(f"release command failed: {' '.join(args)}{': ' + detail if detail else ''}") from error


def _repo_output(args: Sequence[str], repo_root: Path) -> str:
    return _run_checked(args, cwd=repo_root, capture_output=True).stdout.strip()


def _validate_canonical_source(manifest: Mapping[str, Any], repo_root: Path) -> None:
    try:
        branch = _repo_output(("git", "symbolic-ref", "--quiet", "--short", "HEAD"), repo_root)
    except ValueError:
        branch = ""
    if branch != "main":
        raise ValueError("production release source must be a non-detached main checkout")
    dirty_status = _run_checked(
        ("git", "status", "--porcelain", "--untracked-files=all"), cwd=repo_root, capture_output=True
    ).stdout.rstrip("\r\n")
    if dirty_status:
        entries = " | ".join(dirty_status.splitlines())
        raise ValueError(f"production release source must be clean; git status entries: {entries}")
    _run_checked(("git", "fetch", "origin", "main", "--quiet"), cwd=repo_root)
    head = _repo_output(("git", "rev-parse", "HEAD"), repo_root)
    canonical_main = _repo_output(("git", "rev-parse", "refs/remotes/origin/main"), repo_root)
    source_sha = manifest["source"]["git_sha"]
    if head != source_sha or canonical_main != source_sha:
        raise ValueError("production release source must match HEAD, origin/main, and manifest source SHA")


def _assert_no_symlinks(path: Path) -> None:
    if path.is_symlink():
        raise ValueError(f"release ZIP contains an unexpected symlink: {path.name}")


def _audit_zip(path: Path, format_name: str, expected_team: str, *, cwd: Path, require_production: bool = True) -> None:
    if platform.system() != "Darwin":
        raise ValueError("production ZIP audits require macOS")
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"release ZIP is not a regular file: {path.name}")
    with tempfile.TemporaryDirectory(prefix="pump-release-audit-") as temporary:
        extracted = Path(temporary)
        _run_checked(("/usr/bin/ditto", "-x", "-k", str(path), str(extracted)), cwd=cwd)
        bundle = extracted / f"pump.{format_name}"
        contents = bundle / "Contents"
        required = {
            bundle,
            contents,
            contents / "Info.plist",
            contents / "PkgInfo",
            contents / "MacOS",
            contents / "MacOS" / "pump",
        }
        allowed = required | {
            contents / "CodeResources",
            contents / "_CodeSignature",
            contents / "_CodeSignature" / "CodeResources",
        }
        code_resources = contents / "CodeResources"
        if code_resources.exists() and (code_resources.is_symlink() or not code_resources.is_file()):
            raise ValueError(f"{format_name} ZIP Contents/CodeResources must be a regular file")
        if not bundle.is_dir() or not contents.is_dir():
            raise ValueError(f"{format_name} ZIP bundle layout is invalid")
        for current, directories, files in os.walk(extracted, followlinks=False):
            current_path = Path(current)
            _assert_no_symlinks(current_path)
            for name in directories + files:
                child = current_path / name
                _assert_no_symlinks(child)
                if child not in allowed:
                    raise ValueError(f"{format_name} ZIP contains unexpected topology: {child.relative_to(extracted)}")
        if not all(path.exists() for path in required) or not (contents / "MacOS" / "pump").is_file() or not os.access(contents / "MacOS" / "pump", os.X_OK):
            raise ValueError(f"{format_name} ZIP bundle layout is invalid")
        info = contents / "Info.plist"
        binary = contents / "MacOS" / "pump"
        _run_checked(("/usr/bin/plutil", "-lint", str(info)), cwd=cwd)
        identifier = _run_checked(("/usr/bin/plutil", "-extract", "CFBundleIdentifier", "raw", "-o", "-", str(info)), cwd=cwd, capture_output=True).stdout.strip()
        if identifier != f"com.portalsurfer.pump.{format_name}":
            raise ValueError(f"{format_name} ZIP bundle identifier is invalid")
        package_type = _run_checked(("/usr/bin/plutil", "-extract", "CFBundlePackageType", "raw", "-o", "-", str(info)), cwd=cwd, capture_output=True).stdout.strip()
        if package_type != "BNDL":
            raise ValueError(f"{format_name} ZIP package type is invalid")
        _run_checked(("codesign", "--verify", "--deep", "--strict", str(bundle)), cwd=cwd)
        if require_production:
            details = _run_checked(("codesign", "-dv", "--verbose=4", str(bundle)), cwd=cwd, capture_output=True)
            signing_details = f"{details.stdout}\n{details.stderr}"
            if not any(line.startswith("Authority=Developer ID Application:") for line in signing_details.splitlines()):
                raise ValueError(f"{format_name} ZIP is not signed by a Developer ID Application authority")
            team_ids = [line.removeprefix("TeamIdentifier=") for line in signing_details.splitlines() if line.startswith("TeamIdentifier=")]
            if team_ids != [expected_team]:
                raise ValueError(f"{format_name} ZIP Developer ID signing team does not match manifest")
            _run_checked(("xcrun", "stapler", "validate", str(bundle)), cwd=cwd)
            _run_checked(("codesign", "-vvvv", "-R=notarized", "--check-notarization", str(bundle)), cwd=cwd)
        architectures = _run_checked(("lipo", "-archs", str(binary)), cwd=cwd, capture_output=True).stdout.strip()
        if architectures != "arm64":
            raise ValueError(f"{format_name} ZIP binary must contain exactly arm64")
        symbols = _run_checked(("/usr/bin/nm", "-gU", str(binary)), cwd=cwd, capture_output=True).stdout
        required_symbols = ("_clap_entry",) if format_name == "clap" else ("_GetPluginFactory", "_bundleEntry", "_bundleExit")
        if any(symbol not in symbols for symbol in required_symbols):
            raise ValueError(f"{format_name} ZIP required export is missing")


def validate_publish_manifest(manifest: Mapping[str, Any], root: Path, *, require_production: bool = True) -> None:
    if manifest.get("schema_version") != MANIFEST_SCHEMA_V2 or manifest.get("product") != PRODUCT:
        raise ValueError("publish requires Pump manifest schema 2")
    _validate_schema2_manifest(manifest, root, require_production=require_production)


def publish_release(
    *, endpoint: str, token: str, manifest_path: Path, root: Optional[Path] = None, repo_root: Optional[Path] = None
) -> None:
    """Validate and publish one production schema-2 Pump manifest."""
    if endpoint != PRODUCTION_ORIGIN:
        raise ValueError(f"production publishing requires exact origin {PRODUCTION_ORIGIN}")
    if not token:
        raise ValueError("PORTALSURFER_RELEASE_TOKEN is required for --publish")
    manifest_path = Path(manifest_path)
    artifact_root = Path(root) if root is not None else manifest_path.parent
    source_root = Path(repo_root) if repo_root is not None else Path.cwd()
    _validate_regular_file(manifest_path, "release manifest")
    try:
        manifest_bytes = manifest_path.read_bytes()
        manifest = json.loads(manifest_bytes)
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"could not load release manifest: {manifest_path}") from error
    if not isinstance(manifest, dict) or canonical_json(manifest) != manifest_bytes:
        raise ValueError("release manifest is not canonical JSON")
    validate_publish_manifest(manifest, artifact_root)
    _validate_exact_manifest_names(manifest)
    _validate_canonical_source(manifest, source_root)
    expected_team = manifest["signing"]["team_id"]
    for artifact in manifest["artifacts"]:
        _audit_zip(artifact_root / artifact["name"], artifact["format"], expected_team, cwd=source_root)
    for entry in [*manifest["artifacts"], manifest["screenshot"], manifest["changelog"]]:
        path = artifact_root / entry["name"]
        if path.is_symlink():
            raise ValueError(f"release file must not be a symlink: {entry['name']}")
        digest, size = file_digest(path)
        if digest != entry["sha256"] or size != entry["size_bytes"]:
            raise ValueError(f"release bytes changed after ZIP audit: {entry['name']}")
    _publish_validated_manifest(endpoint=endpoint, token=token, manifest=manifest, root=artifact_root, transport=_request)


def validate_preflight_manifest(manifest: Mapping[str, Any], root: Path) -> None:
    """Validate preflight provenance and the stable schema-2 file contract."""
    if manifest.get("distribution") != "preflight":
        raise ValueError("preflight requires ad hoc non-notarized provenance")
    validate_manifest(manifest, root)
    _validate_exact_manifest_names(manifest)


if __name__ == "__main__":
    print("This helper is imported by scripts/release.sh; it does not publish by itself.", file=__import__("sys").stderr)
    raise SystemExit(2)
