#!/usr/bin/env python3
"""Coordinate one protected Pump nightly release.

The nightly scheduler cannot push directly to ``main`` because that branch is
protected.  This coordinator prepares one version-only pull request, runs the
required checks explicitly (a token-created pull request may not start them),
merges it with the exact head SHA, and then dispatches the protected preflight
and release workflows for the merged commit.

The GitHub and git adapters are deliberately small and injectable so the state
machine can be tested without mutating a repository or calling GitHub.
"""

from __future__ import annotations

import dataclasses
import datetime as dt
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Any, Callable, Iterable, Mapping, Optional, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import Request, urlopen

import release_helper

try:
    import tomllib
except ModuleNotFoundError:  # Python 3.10 and the macOS system Python 3.9.
    tomllib = None  # type: ignore[assignment]


class _TomlDecodeError(ValueError):
    pass


def _load_toml(text: str) -> dict[str, Any]:
    if tomllib is not None:
        try:
            value = tomllib.loads(text)
        except tomllib.TOMLDecodeError as error:
            raise _TomlDecodeError(str(error)) from error
        if not isinstance(value, dict):
            raise _TomlDecodeError("TOML root is not a table")
        return value
    # The coordinator only needs the package version from Cargo.toml and the
    # name/version pair from Cargo.lock.  Keep this compatibility parser narrow
    # so Python 3.9 can run the scheduler tests without another dependency.
    package_match = re.search(r"(?ms)^\[package\]\s*(.*?)(?=^\[|\Z)", text)
    if package_match is not None:
        version_match = re.search(r"(?m)^version\s*=\s*['\"]([^'\"]+)['\"]\s*$", package_match.group(1))
        if version_match is None:
            raise _TomlDecodeError("package version is missing")
        return {"package": {"version": version_match.group(1)}}
    packages: list[dict[str, str]] = []
    blocks = re.finditer(r"(?ms)^\[\[package\]\]\s*(.*?)(?=^\[\[package\]\]|\Z)", text)
    for block in blocks:
        fields: dict[str, str] = {}
        for key in ("name", "version"):
            match = re.search(rf"(?m)^{key}\s*=\s*['\"]([^'\"]+)['\"]\s*$", block.group(1))
            if match is not None:
                fields[key] = match.group(1)
        if fields:
            packages.append(fields)
    if not packages:
        raise _TomlDecodeError("TOML package entries are missing")
    return {"package": packages}


PRODUCT = "pump"
REPOSITORY = "PORTALSURFER/pump"
BASE_BRANCH = "main"
VERSION_BRANCH = "codex/nightly-version-bump"
VERSION_PR_MARKER = "<!-- portalsurfer-nightly-version-bump -->"
VERSION_METADATA_PREFIX = "nightly-version-bump-version="
VERSION_METADATA_RE = re.compile(
    rf"(?m)^{re.escape(VERSION_METADATA_PREFIX)}(?P<version>[0-9]+\.[0-9]+\.[0-9]+)\s*$"
)
CI_WORKFLOW = "ci.yml"
PREFLIGHT_WORKFLOW = "release-preflight.yml"
RELEASE_WORKFLOW = "release.yml"
DEFAULT_RELEASES_URL = "https://portalsurfer.org/plugins/api/v1/products/pump/releases"
DEFAULT_POLL_SECONDS = 10.0
DEFAULT_TIMEOUT_SECONDS = 2 * 60 * 60
MAX_TIMEOUT_SECONDS = 3 * 60 * 60


class CoordinatorError(RuntimeError):
    """A fail-closed coordinator error that should be visible in Actions."""


@dataclasses.dataclass(frozen=True)
class NightlyPlan:
    """Pure decision returned by :func:`plan_nightly`."""

    action: str
    current_version: str
    target_version: Optional[str]
    latest_version: Optional[str]
    latest_source_sha: Optional[str]


@dataclasses.dataclass(frozen=True)
class WorkflowRun:
    """The subset of a GitHub workflow run trusted by the coordinator."""

    run_id: int
    run_number: int
    run_attempt: int
    head_sha: str
    head_branch: str
    event: str
    status: str
    conclusion: Optional[str]
    created_at: str


def _positive_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise CoordinatorError(f"{label} must be a positive integer")
    return value


def _sha(value: Any, label: str) -> str:
    if not isinstance(value, str) or release_helper.GIT_SHA.fullmatch(value) is None:
        raise CoordinatorError(f"{label} must be a lowercase 40-character SHA")
    return value


def _timestamp(value: Any, label: str) -> dt.datetime:
    if not isinstance(value, str):
        raise CoordinatorError(f"{label} must be RFC3339")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise CoordinatorError(f"{label} must be RFC3339") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise CoordinatorError(f"{label} must include a timezone")
    return parsed


def plan_nightly(
    *, package_version: str, source_sha: str, document: Any, force: bool = False
) -> NightlyPlan:
    """Return the next scheduler action without mutating any external state."""

    try:
        target = release_helper.plan_nightly_version(
            package_version=package_version,
            source_sha=source_sha,
            document=document,
            force=force,
        )
        latest_version = release_helper.latest_release_version(document)
        latest_source_sha = release_helper.latest_release_source_sha(document, channel="nightly")
    except (TypeError, ValueError) as error:
        raise CoordinatorError(f"could not plan nightly version: {error}") from error
    if target is None:
        return NightlyPlan(
            action="skip",
            current_version=package_version,
            target_version=None,
            latest_version=latest_version,
            latest_source_sha=latest_source_sha,
        )
    action = "release_pending" if target == package_version else "bump"
    return NightlyPlan(
        action=action,
        current_version=package_version,
        target_version=target,
        latest_version=latest_version,
        latest_source_sha=latest_source_sha,
    )


def _parse_run(value: Mapping[str, Any], *, expected_sha: str, expected_branch: str) -> WorkflowRun:
    run_id = _positive_int(value.get("id"), "workflow run id")
    run_number = _positive_int(value.get("run_number"), "workflow run number")
    run_attempt = _positive_int(value.get("run_attempt"), "workflow run attempt")
    head_sha = _sha(value.get("head_sha"), "workflow run head SHA")
    head_branch = value.get("head_branch")
    event = value.get("event")
    status = value.get("status")
    conclusion = value.get("conclusion")
    created_at = value.get("created_at")
    _timestamp(created_at, "workflow run created_at")
    if (
        head_sha != expected_sha
        or head_branch != expected_branch
        or event != "workflow_dispatch"
        or not isinstance(status, str)
        or not status
        or status == "completed"
        and (conclusion is not None and (not isinstance(conclusion, str) or not conclusion))
    ):
        raise CoordinatorError("workflow run identity is invalid")
    if status != "completed" and conclusion not in {None, ""}:
        raise CoordinatorError("incomplete workflow run has a conclusion")
    if status == "completed" and not isinstance(conclusion, str):
        raise CoordinatorError("completed workflow run is missing its conclusion")
    return WorkflowRun(
        run_id=run_id,
        run_number=run_number,
        run_attempt=run_attempt,
        head_sha=head_sha,
        head_branch=head_branch,
        event=event,
        status=status,
        conclusion=conclusion,
        created_at=created_at,
    )


def _run_sort_key(run: WorkflowRun) -> tuple[int, int, dt.datetime, int]:
    return (run.run_number, run.run_attempt, _timestamp(run.created_at, "workflow run created_at"), run.run_id)


def _latest_runs(
    values: Iterable[Mapping[str, Any]], *, expected_sha: str, expected_branch: str
) -> list[WorkflowRun]:
    runs: list[WorkflowRun] = []
    for value in values:
        if not isinstance(value, Mapping):
            raise CoordinatorError("workflow runs response contains a non-object")
        # The API query filters to workflow_dispatch, but filtering again here
        # keeps an ignored push run from becoming release evidence.
        if value.get("event") != "workflow_dispatch":
            continue
        runs.append(_parse_run(value, expected_sha=expected_sha, expected_branch=expected_branch))
    runs.sort(key=_run_sort_key)
    return runs


class GitHubApi:
    """Minimal authenticated REST adapter used by the coordinator."""

    def __init__(self, *, api_url: str, repository: str, token: str) -> None:
        parsed = urlsplit(api_url or "")
        if parsed.scheme not in {"http", "https"} or not parsed.netloc or parsed.query or parsed.fragment:
            raise CoordinatorError("GitHub API URL is invalid")
        if repository != REPOSITORY:
            raise CoordinatorError("GitHub repository is invalid")
        if not token:
            raise CoordinatorError("GitHub token is missing")
        owner, repo = repository.split("/", 1)
        self.base_url = api_url.rstrip("/")
        self.repository = repository
        self.repository_path = f"/repos/{quote(owner, safe='')}/{quote(repo, safe='')}"
        self.token = token

    def request(
        self,
        method: str,
        path: str,
        *,
        query: Sequence[tuple[str, str]] = (),
        payload: Any = None,
        expected: Sequence[int] = (200,),
    ) -> Any:
        if not path.startswith("/"):
            raise CoordinatorError("GitHub API path is invalid")
        url = f"{self.base_url}{self.repository_path}{path}"
        if query:
            url = f"{url}?{urlencode(list(query))}"
        body = None
        headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {self.token}",
            "X-GitHub-Api-Version": "2022-11-28",
        }
        if payload is not None:
            body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = Request(url, method=method, data=body, headers=headers)
        try:
            with urlopen(request, timeout=30) as response:
                status = response.getcode()
                response_body = response.read()
        except HTTPError as error:
            raise CoordinatorError(f"GitHub API returned HTTP {error.code}: {method} {path}") from error
        except (OSError, URLError, TimeoutError) as error:
            raise CoordinatorError(f"GitHub API request failed: {method} {path}") from error
        if status not in expected:
            raise CoordinatorError(f"GitHub API returned HTTP {status}: {method} {path}")
        if not response_body:
            return None
        try:
            return json.loads(response_body)
        except (TypeError, json.JSONDecodeError) as error:
            raise CoordinatorError(f"GitHub API returned malformed JSON: {method} {path}") from error

    def list_open_pull_requests(self) -> list[dict[str, Any]]:
        value = self.request(
            "GET",
            "/pulls",
            query=[("state", "open"), ("base", BASE_BRANCH), ("per_page", "100")],
        )
        if not isinstance(value, list) or any(not isinstance(item, dict) for item in value):
            raise CoordinatorError("open pull request response is invalid")
        return value

    def get_pull_request(self, number: int) -> dict[str, Any]:
        value = self.request("GET", f"/pulls/{_positive_int(number, 'pull request number')}")
        if not isinstance(value, dict):
            raise CoordinatorError("pull request response is invalid")
        return value

    def create_pull_request(self, *, title: str, body: str, head: str, base: str) -> dict[str, Any]:
        value = self.request(
            "POST",
            "/pulls",
            payload={"title": title, "body": body, "head": head, "base": base},
            expected=(201,),
        )
        if not isinstance(value, dict):
            raise CoordinatorError("created pull request response is invalid")
        return value

    def branch_sha(self, branch: str) -> Optional[str]:
        encoded = quote(branch, safe="")
        try:
            value = self.request("GET", f"/git/ref/heads/{encoded}")
        except CoordinatorError as error:
            if "HTTP 404" in str(error):
                return None
            raise
        if not isinstance(value, dict) or not isinstance(value.get("object"), dict):
            raise CoordinatorError("branch ref response is invalid")
        return _sha(value["object"].get("sha"), "branch ref SHA")

    def merge_pull_request(
        self, *, number: int, head_sha: str, title: str, commit_message: str = ""
    ) -> str:
        payload = {
            "sha": _sha(head_sha, "pull request head SHA"),
            "merge_method": "squash",
            "commit_title": title,
        }
        if commit_message:
            payload["commit_message"] = commit_message
        value = self.request(
            "PUT",
            f"/pulls/{_positive_int(number, 'pull request number')}/merge",
            payload=payload,
        )
        if not isinstance(value, dict) or value.get("merged") is not True:
            raise CoordinatorError("protected version pull request was not merged")
        return _sha(value.get("sha"), "merge commit SHA")

    def dispatch_workflow(
        self,
        workflow: str,
        *,
        ref: str,
        inputs: Optional[Mapping[str, str]] = None,
    ) -> None:
        payload: dict[str, Any] = {"ref": ref}
        if inputs:
            payload["inputs"] = dict(inputs)
        value = self.request(
            "POST",
            f"/actions/workflows/{quote(workflow, safe='')}/dispatches",
            payload=payload,
            expected=(204,),
        )
        if value is not None:
            raise CoordinatorError("workflow dispatch returned an unexpected body")

    def workflow_runs(self, workflow: str, *, branch: str, head_sha: str) -> list[dict[str, Any]]:
        value = self.request(
            "GET",
            f"/actions/workflows/{quote(workflow, safe='')}/runs",
            query=[
                ("branch", branch),
                ("event", "workflow_dispatch"),
                ("head_sha", _sha(head_sha, "workflow head SHA")),
                ("per_page", "100"),
            ],
        )
        if not isinstance(value, dict) or not isinstance(value.get("workflow_runs"), list):
            raise CoordinatorError("workflow runs response is invalid")
        if any(not isinstance(item, dict) for item in value["workflow_runs"]):
            raise CoordinatorError("workflow runs response contains a non-object")
        return list(value["workflow_runs"])


class GitRepository:
    """Subprocess adapter for the checked-out scheduler repository."""

    def __init__(self, root: Path) -> None:
        self.root = Path(root)

    def run(self, args: Sequence[str], *, check: bool = True) -> str:
        result = subprocess.run(
            ["git", *args], cwd=self.root, text=True, capture_output=True, check=False
        )
        if check and result.returncode != 0:
            raise CoordinatorError(f"git {' '.join(args)} failed: {result.stderr.strip()}")
        return result.stdout.strip()

    def ensure_clean(self) -> None:
        status = self.run(["status", "--porcelain", "--untracked-files=all"])
        if status:
            raise CoordinatorError(f"scheduler checkout is not clean: {status}")

    def fetch_main(self) -> None:
        self.run(["fetch", "origin", BASE_BRANCH, "--quiet"])

    def fetch_version_branch(self) -> None:
        self.run(["fetch", "origin", VERSION_BRANCH, "--quiet"])

    def main_sha(self) -> str:
        return _sha(self.run(["rev-parse", f"refs/remotes/origin/{BASE_BRANCH}"]), "main SHA")

    def package_version(self, ref: Optional[str] = None) -> str:
        if ref is None:
            data = (self.root / "Cargo.toml").read_bytes()
        else:
            raw = self.run(["show", f"{ref}:Cargo.toml"])
            data = raw.encode("utf-8")
        try:
            value = _load_toml(data.decode("utf-8"))["package"]["version"]
        except (KeyError, TypeError, _TomlDecodeError) as error:
            raise CoordinatorError("Cargo.toml package version is invalid") from error
        if not isinstance(value, str):
            raise CoordinatorError("Cargo.toml package version is invalid")
        return value

    def commit_message(self, ref: Optional[str] = None) -> str:
        """Read the exact commit message used to identify an owned bump."""
        return self.run(
            ["show", "-s", "--format=%B", ref or f"refs/remotes/origin/{BASE_BRANCH}"]
        )

    def commit_parent(self, ref: str) -> Optional[str]:
        """Return a commit's single parent, or ``None`` for a root commit."""
        fields = self.run(["rev-list", "--parents", "-n", "1", ref]).split()
        if not fields:
            raise CoordinatorError("commit parent response is empty")
        _sha(fields[0], "commit SHA")
        if len(fields) > 2:
            raise CoordinatorError("version branch commit must have one parent")
        return _sha(fields[1], "commit parent SHA") if len(fields) == 2 else None

    def file_at(self, ref: str, path: str) -> bytes:
        """Read a repository file without the text adapter trimming newlines."""
        result = subprocess.run(
            ["git", "show", f"{ref}:{path}"],
            cwd=self.root,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            raise CoordinatorError(f"git show failed for {path}: {result.stderr.decode(errors='replace').strip()}")
        return result.stdout

    def _expected_version_files(self, *, base_sha: str, target_version: str) -> tuple[bytes, bytes]:
        bump = self.root / "scripts" / "bump_version.py"
        if not bump.is_file():
            raise CoordinatorError("version bump script is missing")
        with tempfile.TemporaryDirectory(prefix="pump-nightly-bump-") as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_bytes(self.file_at(base_sha, "Cargo.toml"))
            (root / "Cargo.lock").write_bytes(self.file_at(base_sha, "Cargo.lock"))
            result = subprocess.run(
                [sys.executable, str(bump), target_version],
                cwd=root,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                raise CoordinatorError(f"version bump failed: {result.stderr.strip()}")
            return (root / "Cargo.toml").read_bytes(), (root / "Cargo.lock").read_bytes()

    def owned_version_branch(self, branch_sha: str, *, target_version: str) -> bool:
        """Check marker, files, and package version without assuming its base."""
        branch_sha = _sha(branch_sha, "version branch SHA")
        parent = self.commit_parent(branch_sha)
        if parent is None:
            return False
        changed = set(self.changed_paths(parent, branch_sha))
        if changed != {"Cargo.toml", "Cargo.lock"}:
            return False
        try:
            expected_manifest, expected_lock = self._expected_version_files(
                base_sha=parent, target_version=target_version
            )
        except CoordinatorError:
            return False
        message = self.commit_message(branch_sha)
        match = VERSION_METADATA_RE.search(message)
        return (
            VERSION_PR_MARKER in message
            and match is not None
            and match.group("version") == target_version
            and self.package_version(branch_sha) == target_version
            and self.lock_package_version(branch_sha) == target_version
            and self.file_at(branch_sha, "Cargo.toml") == expected_manifest
            and self.file_at(branch_sha, "Cargo.lock") == expected_lock
        )

    def lock_package_version(self, ref: str) -> str:
        """Read the Pump package version from Cargo.lock at ``ref``."""
        try:
            packages = _load_toml(self.file_at(ref, "Cargo.lock").decode("utf-8"))["package"]
            matches = [
                package
                for package in packages
                if isinstance(package, dict) and package.get("name") == PRODUCT
            ]
            if len(matches) != 1 or not isinstance(matches[0].get("version"), str):
                raise ValueError
            return matches[0]["version"]
        except (KeyError, TypeError, ValueError, _TomlDecodeError) as error:
            raise CoordinatorError("Cargo.lock Pump package version is invalid") from error

    def validate_version_branch(
        self, *, base_sha: str, target_version: str, branch_sha: str
    ) -> str:
        """Classify an existing branch as exact, stale-owned, or reject it.

        ``stale-owned`` is safe to replace with a force-with-lease because the
        commit contains our marker and exact metadata.  Any other content is
        treated as foreign or tampered with, including edits hidden inside the
        two allowed files.
        """
        base_sha = _sha(base_sha, "base SHA")
        branch_sha = _sha(branch_sha, "version branch SHA")
        expected_manifest, expected_lock = self._expected_version_files(
            base_sha=base_sha, target_version=target_version
        )
        parent = self.commit_parent(branch_sha)
        message = self.commit_message(branch_sha)
        match = VERSION_METADATA_RE.search(message)
        marker_ok = (
            VERSION_PR_MARKER in message
            and match is not None
            and match.group("version") == target_version
        )
        if not marker_ok:
            raise CoordinatorError("version branch is not an owned nightly bump")
        if parent is None:
            raise CoordinatorError("version branch has no base commit")
        changed = set(self.changed_paths(parent, branch_sha))
        if changed != {"Cargo.toml", "Cargo.lock"}:
            raise CoordinatorError("version branch contains unexpected files")
        expected_parent_manifest, expected_parent_lock = self._expected_version_files(
            base_sha=parent, target_version=target_version
        )
        if (
            self.file_at(branch_sha, "Cargo.toml") != expected_parent_manifest
            or self.file_at(branch_sha, "Cargo.lock") != expected_parent_lock
        ):
            raise CoordinatorError("version branch version patch differs from the expected base patch")
        if parent == base_sha:
            if (
                self.file_at(branch_sha, "Cargo.toml") != expected_manifest
                or self.file_at(branch_sha, "Cargo.lock") != expected_lock
            ):
                raise CoordinatorError("version branch version patch differs from the expected base patch")
            return "exact"
        return "stale-owned"

    def remote_branch_sha(self, branch: str) -> Optional[str]:
        result = self.run(["ls-remote", "origin", f"refs/heads/{branch}"], check=False)
        if not result:
            return None
        fields = result.split()
        if len(fields) != 2 or fields[1] != f"refs/heads/{branch}":
            raise CoordinatorError("version branch ref response is invalid")
        return _sha(fields[0], "version branch SHA")

    def changed_paths(self, base_sha: str, branch_sha: str) -> list[str]:
        output = self.run(["diff", "--name-only", f"{base_sha}...{branch_sha}"])
        return [line for line in output.splitlines() if line]

    def is_ancestor(self, ancestor: str, descendant: str) -> bool:
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", ancestor, descendant],
            cwd=self.root,
            text=True,
            capture_output=True,
            check=False,
        )
        return result.returncode == 0

    def delete_remote_branch(self, *, branch: str, expected_sha: str) -> None:
        """Delete a branch with Git's atomic force-with-lease check."""
        expected_sha = _sha(expected_sha, "version branch SHA")
        self.run(
            [
                "push",
                f"--force-with-lease=refs/heads/{branch}:{expected_sha}",
                "origin",
                f":{branch}",
            ]
        )

    def prepare_version_branch(
        self,
        *,
        base_sha: str,
        target_version: str,
        existing_branch_sha: Optional[str],
    ) -> str:
        """Create/update the owned two-file branch and return its exact SHA."""
        self.ensure_clean()
        base_sha = _sha(base_sha, "base SHA")
        bump = self.root / "scripts" / "bump_version.py"
        if not bump.is_file():
            raise CoordinatorError("version bump script is missing")
        if existing_branch_sha is not None:
            existing_branch_sha = _sha(existing_branch_sha, "existing version branch SHA")
            self.run(["fetch", "origin", VERSION_BRANCH, "--quiet"])
            state = self.validate_version_branch(
                base_sha=base_sha,
                target_version=target_version,
                branch_sha=existing_branch_sha,
            )
            if state == "exact":
                return existing_branch_sha
        self.run(["switch", "--detach", base_sha])
        self.run(["switch", "-C", VERSION_BRANCH, base_sha])
        result = subprocess.run(
            [sys.executable, str(bump), target_version],
            cwd=self.root,
            text=True,
            capture_output=True,
            check=False,
        )
        if result.returncode != 0:
            raise CoordinatorError(f"version bump failed: {result.stderr.strip()}")
        changed = set(self.run(["diff", "--name-only"]).splitlines())
        if changed != {"Cargo.toml", "Cargo.lock"}:
            raise CoordinatorError("version bump changed files outside Cargo.toml and Cargo.lock")
        self.run(["diff", "--check"])
        self.run(["config", "user.name", "github-actions[bot]"])
        self.run(["config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"])
        self.run(["add", "Cargo.toml", "Cargo.lock"])
        self.run(
            [
                "commit",
                "-m",
                f"chore(release): prepare Pump v{target_version} nightly",
                "-m",
                f"{VERSION_PR_MARKER}\n{VERSION_METADATA_PREFIX}{target_version}\nbase-sha={base_sha}",
            ]
        )
        push = ["push"]
        if existing_branch_sha is not None:
            push.append(f"--force-with-lease=refs/heads/{VERSION_BRANCH}:{existing_branch_sha}")
        push.extend(["origin", f"HEAD:{VERSION_BRANCH}"])
        self.run(push)
        return _sha(self.run(["rev-parse", "HEAD"]), "version branch SHA")


def _fetch_release_document(url: str) -> Any:
    parsed = urlsplit(url or "")
    if parsed.scheme not in {"http", "https"} or not parsed.netloc or parsed.query or parsed.fragment:
        raise CoordinatorError("release history URL is invalid")
    request = Request(url, method="GET", headers={"Accept": "application/json"})
    try:
        with urlopen(request, timeout=30) as response:
            body = response.read()
    except (HTTPError, OSError, URLError, TimeoutError) as error:
        raise CoordinatorError("release history request failed") from error
    try:
        return json.loads(body)
    except (TypeError, json.JSONDecodeError) as error:
        raise CoordinatorError("release history response is invalid JSON") from error


class NightlyCoordinator:
    """Idempotent protected nightly state machine."""

    def __init__(
        self,
        *,
        git: Any,
        api: Any,
        release_document: Any,
        release_document_loader: Optional[Callable[[], Any]] = None,
        sleep: Callable[[float], None] = time.sleep,
        monotonic: Callable[[], float] = time.monotonic,
        poll_seconds: float = DEFAULT_POLL_SECONDS,
        timeout_seconds: float = DEFAULT_TIMEOUT_SECONDS,
    ) -> None:
        if poll_seconds <= 0 or timeout_seconds <= 0 or timeout_seconds > MAX_TIMEOUT_SECONDS:
            raise CoordinatorError("coordinator timeout configuration is invalid")
        self.git = git
        self.api = api
        self.release_document = release_document
        self.release_document_loader = release_document_loader or (lambda: self.release_document)
        self.sleep = sleep
        self.monotonic = monotonic
        self.poll_seconds = poll_seconds
        self.timeout_seconds = timeout_seconds
        self.started = self.monotonic()

    def _check_timeout(self) -> None:
        if self.monotonic() - self.started > self.timeout_seconds:
            raise CoordinatorError("nightly coordinator timed out")

    def _assert_main(self, expected_sha: str) -> None:
        expected_sha = _sha(expected_sha, "expected main SHA")
        self.git.fetch_main()
        actual = _sha(self.git.main_sha(), "current main SHA")
        if actual != expected_sha:
            raise CoordinatorError(f"main moved during nightly coordination: expected {expected_sha}, got {actual}")

    def _owned_pending_version(self, package_version: str, *, ref: str) -> Optional[str]:
        """Return a version from our merge metadata when publication did not land.

        The first release has no public version baseline.  If its version-only
        merge succeeded but publication failed, the next scheduler run must
        resume that package version instead of interpreting the empty history as
        a request for another patch.  A commit is trusted only when its exact
        message carries our marker and its version equals the checked-out
        package version.
        """
        try:
            message = self.git.commit_message(ref=ref)
        except AttributeError as error:
            raise CoordinatorError("git adapter cannot inspect pending bump metadata") from error
        if not isinstance(message, str) or VERSION_PR_MARKER not in message:
            return None
        match = VERSION_METADATA_RE.search(message)
        if match is None:
            raise CoordinatorError("nightly version metadata is malformed")
        version = match.group("version")
        if version != package_version:
            raise CoordinatorError("nightly version metadata does not match package version")
        return version

    def _refresh_release_document(self) -> Any:
        try:
            document = self.release_document_loader()
        except Exception as error:
            raise CoordinatorError("could not refresh release history") from error
        self.release_document = document
        return document

    def _public_release_is_published(self, source_sha: str) -> bool:
        """Check the public record for a valid nightly of this prepared source."""
        package_version = self.git.package_version(ref=source_sha)
        document = self._refresh_release_document()
        try:
            # Validate the complete public document before deciding whether a
            # retry is needed.  A transport or schema error must never trigger
            # another production publish attempt.
            release_helper.latest_release_version(document)
            release_helper.latest_release_source_sha(document, channel="nightly")
        except (TypeError, ValueError) as error:
            raise CoordinatorError(f"public release history is invalid: {error}") from error
        releases = document["releases"]
        invalid_matching_release: Optional[str] = None
        for release in releases:
            if release.get("channel") != "nightly":
                continue
            source = release.get("source")
            if not isinstance(source, Mapping):
                continue
            if source.get("repository") != REPOSITORY or source.get("git_sha") != source_sha:
                continue
            publication_version = release.get("version")
            try:
                release_helper.validate_publication_version(
                    package_version, publication_version, "nightly"
                )
            except (TypeError, ValueError):
                invalid_matching_release = (
                    "public nightly has the prepared source with an invalid package version"
                )
                continue
            expected_build_id = f"pump-v{publication_version}-{source_sha[:12]}"
            if release.get("build_id") != expected_build_id:
                invalid_matching_release = (
                    "public nightly has the prepared source with an invalid build identity"
                )
                continue
            return True
        if invalid_matching_release is not None:
            raise CoordinatorError(invalid_matching_release)
        return False

    def _release_is_published(self, run: WorkflowRun, source_sha: str) -> bool:
        """Require workflow success when public publication is not already present."""
        if run.status != "completed" or run.conclusion != "success":
            return False
        return self._public_release_is_published(source_sha)

    def _require_release_published(self, run: WorkflowRun, source_sha: str) -> None:
        # A publisher may have uploaded the manifest before a later artifact
        # step failed.  Public identity is authoritative even when the run is
        # not successful, so do this check before retrying a failed workflow.
        if self._public_release_is_published(source_sha):
            return
        # A concurrent publisher may have started after the first workflow
        # query.  Let the normal selector pin to that active run before
        # dispatching another production attempt.
        active_or_existing = self._ensure_workflow(
            RELEASE_WORKFLOW,
            branch=BASE_BRANCH,
            head_sha=source_sha,
            inputs={"channel": "nightly", "publish": "true", "only_if_changed": "false"},
            before_dispatch=lambda: self._assert_main(source_sha),
        )
        if self._release_is_published(active_or_existing, source_sha):
            return
        retry = self._ensure_workflow(
            RELEASE_WORKFLOW,
            branch=BASE_BRANCH,
            head_sha=source_sha,
            inputs={"channel": "nightly", "publish": "true", "only_if_changed": "false"},
            require_new=True,
            before_dispatch=lambda: self._assert_main(source_sha),
        )
        if not self._release_is_published(retry, source_sha):
            raise CoordinatorError("release workflow succeeded without exact public publication")

    def _pull_request_for_branch(self) -> Optional[dict[str, Any]]:
        matches = []
        for pr in self.api.list_open_pull_requests():
            if not isinstance(pr, Mapping):
                raise CoordinatorError("pull request response contains a non-object")
            head = pr.get("head")
            base = pr.get("base")
            if not isinstance(head, Mapping) or not isinstance(base, Mapping):
                raise CoordinatorError("pull request ref metadata is invalid")
            if head.get("ref") == VERSION_BRANCH and base.get("ref") == BASE_BRANCH:
                matches.append(dict(pr))
        if len(matches) > 1:
            raise CoordinatorError("multiple nightly version pull requests are open")
        if not matches:
            return None
        pr = matches[0]
        head = pr["head"]
        repository = head.get("repo")
        if not isinstance(repository, Mapping) or repository.get("full_name") != REPOSITORY:
            raise CoordinatorError("nightly version pull request is from a foreign repository")
        body = pr.get("body")
        if not isinstance(body, str) or VERSION_PR_MARKER not in body:
            raise CoordinatorError("nightly version pull request is not owned by the coordinator")
        number = _positive_int(pr.get("number"), "pull request number")
        head_sha = _sha(head.get("sha"), "pull request head SHA")
        return {"number": number, "head_sha": head_sha, "body": body, "title": pr.get("title", "")}

    def _cleanup_owned_branch(self, expected_sha: str) -> None:
        """Remove the merged bot branch with a race-safe lease."""
        actual = self.api.branch_sha(VERSION_BRANCH)
        if actual is None:
            return
        if actual != _sha(expected_sha, "version branch SHA"):
            raise CoordinatorError("version branch changed before cleanup")
        try:
            self.git.delete_remote_branch(branch=VERSION_BRANCH, expected_sha=actual)
        except AttributeError as error:
            raise CoordinatorError("git adapter cannot perform leased version branch cleanup") from error

    def _ensure_workflow(
        self,
        workflow: str,
        *,
        branch: str,
        head_sha: str,
        inputs: Optional[Mapping[str, str]] = None,
        require_new: bool = False,
        before_dispatch: Optional[Callable[[], None]] = None,
    ) -> WorkflowRun:
        """Reuse a successful exact run, wait active runs, or dispatch one retry."""
        head_sha = _sha(head_sha, "workflow head SHA")
        dispatched_attempts = 0
        awaiting_new_run = require_new
        previous_run_ids: set[int] = set()
        required_run_id: Optional[int] = None
        force_dispatch_pending = require_new
        while True:
            self._check_timeout()
            raw_runs = self.api.workflow_runs(workflow, branch=branch, head_sha=head_sha)
            runs = _latest_runs(raw_runs, expected_sha=head_sha, expected_branch=branch)
            if force_dispatch_pending:
                previous_run_ids = {run.run_id for run in runs}
                if before_dispatch is not None:
                    before_dispatch()
                self.api.dispatch_workflow(workflow, ref=branch, inputs=inputs)
                dispatched_attempts += 1
                force_dispatch_pending = False
                awaiting_new_run = True
                self.sleep(self.poll_seconds)
                continue
            if required_run_id is not None:
                selected = [run for run in runs if run.run_id == required_run_id]
                candidate = selected[-1] if selected else None
            elif awaiting_new_run:
                new_runs = [run for run in runs if run.run_id not in previous_run_ids]
                candidate = new_runs[-1] if new_runs else None
            else:
                candidate = runs[-1] if runs else None
            if candidate is not None and candidate.status == "completed":
                if candidate.conclusion == "success":
                    return candidate
                if dispatched_attempts >= 1:
                    raise CoordinatorError(
                        f"{workflow} failed for {branch} {head_sha} after a retry"
                    )
                previous_run_ids = {run.run_id for run in runs}
                required_run_id = None
                if before_dispatch is not None:
                    before_dispatch()
                self.api.dispatch_workflow(workflow, ref=branch, inputs=inputs)
                dispatched_attempts += 1
                awaiting_new_run = True
            elif candidate is None:
                if required_run_id is not None or dispatched_attempts >= 1:
                    # A dispatch accepted by GitHub can take a few polls to
                    # become visible.  A pinned active run that temporarily
                    # disappears must also remain pinned rather than being
                    # replaced by another run.
                    pass
                else:
                    previous_run_ids = {run.run_id for run in runs}
                    if before_dispatch is not None:
                        before_dispatch()
                    self.api.dispatch_workflow(workflow, ref=branch, inputs=inputs)
                    dispatched_attempts += 1
                    awaiting_new_run = True
            elif candidate.status in {"queued", "in_progress", "waiting", "requested", "pending"}:
                # Pin every active run, including one that existed before this
                # coordinator invocation.  This prevents a newer concurrent
                # run from replacing the run we are already waiting on.
                required_run_id = candidate.run_id
                awaiting_new_run = False
            else:
                raise CoordinatorError(f"{workflow} returned unknown run status {candidate.status}")
            self.sleep(self.poll_seconds)

    def _merge_version_pr(
        self, *, pr: Mapping[str, Any], expected_main_sha: str, target_version: str
    ) -> str:
        self._assert_main(expected_main_sha)
        current = self.api.get_pull_request(int(pr["number"]))
        head = current.get("head") if isinstance(current, Mapping) else None
        if not isinstance(head, Mapping):
            raise CoordinatorError("version pull request head metadata is invalid")
        head_sha = _sha(head.get("sha"), "version pull request head SHA")
        if head_sha != pr["head_sha"]:
            raise CoordinatorError("version pull request changed while checks were running")
        merged_sha = self.api.merge_pull_request(
            number=int(pr["number"]),
            head_sha=head_sha,
            title=str(current.get("title") or "Prepare Pump nightly version"),
            commit_message=(
                f"{VERSION_PR_MARKER}\n{VERSION_METADATA_PREFIX}{target_version}\n"
                f"base-sha={expected_main_sha}"
            ),
        )
        self._assert_main(merged_sha)
        self._cleanup_owned_branch(head_sha)
        return merged_sha

    def run(self, *, force: bool = False) -> Optional[str]:
        self.git.ensure_clean()
        self.git.fetch_main()
        initial_main_sha = _sha(self.git.main_sha(), "initial main SHA")
        package_version = self.git.package_version(ref=initial_main_sha)
        plan = plan_nightly(
            package_version=package_version,
            source_sha=initial_main_sha,
            document=self.release_document,
            force=force,
        )
        if (
            plan.action == "bump"
            and plan.latest_version is None
            and plan.target_version != package_version
        ):
            pending_version = self._owned_pending_version(package_version, ref=initial_main_sha)
            if pending_version is not None:
                plan = dataclasses.replace(
                    plan, action="release_pending", target_version=pending_version
                )
        if plan.action == "skip":
            return None

        expected_release_sha = initial_main_sha
        if plan.action == "bump":
            remote_branch_sha = self.api.branch_sha(VERSION_BRANCH)
            if remote_branch_sha is not None:
                # Fetch the object before inspecting the PR's head SHA or
                # validating its exact two-file contents.
                self.git.fetch_version_branch()
            open_pr = self._pull_request_for_branch()
            target = plan.target_version
            if target is None:
                raise CoordinatorError("nightly bump plan has no target version")
            if remote_branch_sha is not None:
                branch_state = self.git.validate_version_branch(
                    base_sha=initial_main_sha,
                    target_version=target,
                    branch_sha=remote_branch_sha,
                )
                if branch_state not in {"exact", "stale-owned"}:
                    raise CoordinatorError("version branch is not an owned nightly bump")
            elif open_pr is not None:
                raise CoordinatorError("owned nightly version pull request has no branch")
            branch_sha = self.git.prepare_version_branch(
                base_sha=initial_main_sha,
                target_version=target,
                existing_branch_sha=remote_branch_sha,
            )
            self._assert_main(initial_main_sha)
            current_branch_sha = self.api.branch_sha(VERSION_BRANCH)
            if current_branch_sha != branch_sha:
                raise CoordinatorError("version branch changed after preparation")
            if open_pr is None:
                body = (
                    f"{VERSION_PR_MARKER}\n\n"
                    f"Prepare Pump nightly package version **{target}** from `{initial_main_sha}`.\n\n"
                    "This is an automated version-only change. Required checks are dispatched "
                    "explicitly before the protected merge."
                )
                created = self.api.create_pull_request(
                    title=f"chore(release): prepare Pump v{target} nightly",
                    body=body,
                    head=VERSION_BRANCH,
                    base=BASE_BRANCH,
                )
                number = _positive_int(created.get("number"), "created pull request number")
                open_pr = {"number": number, "head_sha": branch_sha}
            else:
                refreshed = self.api.get_pull_request(int(open_pr["number"]))
                refreshed_head = refreshed.get("head") if isinstance(refreshed, Mapping) else None
                if not isinstance(refreshed_head, Mapping):
                    raise CoordinatorError("version pull request head metadata is invalid")
                refreshed_sha = _sha(refreshed_head.get("sha"), "version pull request head SHA")
                if refreshed_sha != branch_sha:
                    raise CoordinatorError("owned version pull request head does not match branch")
                open_pr = dict(open_pr)
                open_pr["head_sha"] = refreshed_sha
            self._ensure_workflow(CI_WORKFLOW, branch=VERSION_BRANCH, head_sha=branch_sha)
            self._ensure_workflow(PREFLIGHT_WORKFLOW, branch=VERSION_BRANCH, head_sha=branch_sha)
            expected_release_sha = self._merge_version_pr(
                pr=open_pr,
                expected_main_sha=initial_main_sha,
                target_version=target,
            )
        else:
            # A package version ahead of public history is an already-merged
            # pending bump.  An open version PR in this state is inconsistent
            # and must be resolved before publishing.
            if self._pull_request_for_branch() is not None:
                raise CoordinatorError("main has a pending package bump and an open version pull request")
            remote_branch_sha = self.api.branch_sha(VERSION_BRANCH)
            if remote_branch_sha is not None:
                self.git.fetch_version_branch()
                if not self.git.owned_version_branch(
                    remote_branch_sha, target_version=package_version
                ):
                    raise CoordinatorError("pending version branch is not owned by the coordinator")
                self._cleanup_owned_branch(remote_branch_sha)
            self._assert_main(initial_main_sha)

        self._assert_main(expected_release_sha)
        self._ensure_workflow(
            PREFLIGHT_WORKFLOW,
            branch=BASE_BRANCH,
            head_sha=expected_release_sha,
            before_dispatch=lambda: self._assert_main(expected_release_sha),
        )
        self._assert_main(expected_release_sha)
        release_run = self._ensure_workflow(
            RELEASE_WORKFLOW,
            branch=BASE_BRANCH,
            head_sha=expected_release_sha,
            inputs={"channel": "nightly", "publish": "true", "only_if_changed": "false"},
            before_dispatch=lambda: self._assert_main(expected_release_sha),
        )
        self._require_release_published(release_run, expected_release_sha)
        return expected_release_sha


def main() -> int:
    try:
        force = os.environ.get("FORCE", "false").lower() == "true"
        releases_url = os.environ.get("RELEASES_URL", DEFAULT_RELEASES_URL)
        api = GitHubApi(
            api_url=os.environ.get("GITHUB_API_URL", "https://api.github.com"),
            repository=os.environ.get("GITHUB_REPOSITORY", REPOSITORY),
            token=os.environ.get("GH_TOKEN", ""),
        )
        document = _fetch_release_document(releases_url)
        timeout = float(os.environ.get("COORDINATOR_TIMEOUT_SECONDS", str(DEFAULT_TIMEOUT_SECONDS)))
        poll = float(os.environ.get("COORDINATOR_POLL_SECONDS", str(DEFAULT_POLL_SECONDS)))
        result = NightlyCoordinator(
            git=GitRepository(Path.cwd()),
            api=api,
            release_document=document,
            release_document_loader=lambda: _fetch_release_document(releases_url),
            poll_seconds=poll,
            timeout_seconds=timeout,
        ).run(force=force)
    except (CoordinatorError, ValueError) as error:
        print(f"::error::nightly coordinator failed closed: {error}", file=sys.stderr)
        return 1
    if result is None:
        print("nightly source is already published; no version bump or release was needed")
    else:
        print(f"nightly release workflow completed for main {result}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
