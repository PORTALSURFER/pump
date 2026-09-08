#!/usr/bin/env python3
"""Fail-closed verification of the protected publisher preflight run."""

from __future__ import annotations

import datetime as dt
import json
import os
import re
import sys
from typing import Any, Sequence
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import Request, urlopen


WORKFLOW_FILE = ".github/workflows/release-preflight.yml"
WORKFLOW_NAME = "Pump release preflight"
BRANCH = "main"
EVENT = "push"
JOB_NAME = "publisher_integration"
ENVIRONMENT_NAME = "publisher-integration"
SHA_PATTERN = re.compile(r"[0-9a-f]{40}\Z")
REPOSITORY_PATTERN = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+\Z")


class GateError(RuntimeError):
    """An evidence or API failure that must block production publication."""


def _object(value: Any, label: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise GateError(f"{label} must be an object")
    return value


def _string(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise GateError(f"{label} must be a non-empty string")
    return value


def _positive_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
        raise GateError(f"{label} must be a positive integer")
    return value


def _nonnegative_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise GateError(f"{label} must be a non-negative integer")
    return value


def _sha(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA_PATTERN.fullmatch(value) is None:
        raise GateError(f"{label} must be a lowercase 40-character SHA")
    return value


def _timestamp(value: Any, label: str) -> dt.datetime:
    value = _string(value, label)
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise GateError(f"{label} must be RFC3339") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise GateError(f"{label} must include a timezone")
    return parsed


def validate_workflow(document: Any) -> int:
    """Validate the workflow identity and return its numeric GitHub id."""
    document = _object(document, "workflow response")
    workflow_id = _positive_int(document.get("id"), "workflow id")
    if (
        document.get("name") != WORKFLOW_NAME
        or document.get("path") != WORKFLOW_FILE
        or document.get("state") != "active"
    ):
        raise GateError("release-preflight workflow identity is invalid")
    return workflow_id


def _validate_run(document: Any, expected_sha: str, workflow_id: int) -> dict[str, Any]:
    run = _object(document, "workflow run")
    _positive_int(run.get("id"), "workflow run id")
    _positive_int(run.get("workflow_id"), "workflow run workflow id")
    _positive_int(run.get("run_number"), "workflow run number")
    _positive_int(run.get("run_attempt"), "workflow run attempt")
    _timestamp(run.get("created_at"), "workflow run created_at")
    _timestamp(run.get("updated_at"), "workflow run updated_at")
    if (
        run.get("workflow_id") != workflow_id
        or run.get("name") != WORKFLOW_NAME
        or run.get("path") != WORKFLOW_FILE
        or run.get("head_sha") != expected_sha
        or run.get("head_branch") != BRANCH
        or run.get("event") != EVENT
        or run.get("status") != "completed"
        or not isinstance(run.get("conclusion"), str)
        or not run["conclusion"]
    ):
        raise GateError("workflow run identity or completion evidence is invalid")
    return run


def select_latest_run(document: Any, expected_sha: str, workflow_id: int) -> dict[str, Any]:
    """Select the newest completed attempt, then require its overall success."""
    _sha(expected_sha, "expected source SHA")
    document = _object(document, "workflow runs response")
    total_count = _nonnegative_int(document.get("total_count"), "workflow runs total_count")
    runs = document.get("workflow_runs")
    if not isinstance(runs, list) or total_count != len(runs) or not runs:
        raise GateError("workflow runs response is missing a complete run list")
    validated = [_validate_run(run, expected_sha, workflow_id) for run in runs]
    latest = max(
        validated,
        key=lambda run: (
            run["run_number"],
            run["run_attempt"],
            _timestamp(run["updated_at"], "workflow run updated_at"),
            run["id"],
        ),
    )
    if latest["conclusion"] != "success":
        raise GateError("latest protected release-preflight run did not succeed")
    return latest


def validate_attempt(document: Any, run: dict[str, Any], expected_sha: str) -> dict[str, Any]:
    """Require the fetched attempt to be exactly the selected workflow attempt."""
    attempt = _validate_run(document, expected_sha, run["workflow_id"])
    if (
        attempt.get("id") != run["id"]
        or attempt.get("run_number") != run["run_number"]
        or attempt.get("run_attempt") != run["run_attempt"]
    ):
        raise GateError("workflow run attempt identity is invalid")
    if attempt["conclusion"] != "success":
        raise GateError("latest release-preflight attempt did not succeed")
    return attempt


def validate_jobs(document: Any, run: dict[str, Any], expected_sha: str) -> dict[str, Any]:
    """Require exactly one successful publisher job from the selected attempt."""
    document = _object(document, "workflow jobs response")
    total_count = _nonnegative_int(document.get("total_count"), "workflow jobs total_count")
    jobs = document.get("jobs")
    if not isinstance(jobs, list) or total_count != len(jobs):
        raise GateError("workflow jobs response is missing a complete job list")
    matches = []
    for value in jobs:
        job = _object(value, "workflow job")
        _positive_int(job.get("id"), "workflow job id")
        _positive_int(job.get("run_id"), "workflow job run id")
        _positive_int(job.get("run_attempt"), "workflow job attempt")
        _sha(job.get("head_sha"), "workflow job head SHA")
        _string(job.get("head_branch"), "workflow job head branch")
        _string(job.get("workflow_name"), "workflow job workflow name")
        _string(job.get("name"), "workflow job name")
        _string(job.get("status"), "workflow job status")
        _string(job.get("conclusion"), "workflow job conclusion")
        if (
            job.get("run_id") != run["id"]
            or job.get("run_attempt") != run["run_attempt"]
            or job.get("head_sha") != expected_sha
            or job.get("head_branch") != BRANCH
            or job.get("workflow_name") != WORKFLOW_NAME
            or job.get("status") != "completed"
        ):
            raise GateError("publisher_integration job identity or completion evidence is invalid")
        if job["name"] == JOB_NAME:
            matches.append(job)
    if len(matches) != 1:
        raise GateError("workflow jobs response must contain exactly one publisher_integration job")
    job = matches[0]
    if job["conclusion"] != "success":
        raise GateError("publisher_integration job did not succeed")
    return job


def validate_approvals(document: Any) -> None:
    """Require an approved review for the publisher-integration environment."""
    if not isinstance(document, list) or not document:
        raise GateError("workflow run review history is missing")
    approved = False
    for value in document:
        approval = _object(value, "workflow approval")
        state = approval.get("state")
        if state not in {"approved", "rejected"}:
            raise GateError("workflow approval state is invalid")
        user = _object(approval.get("user"), "workflow approval user")
        _string(user.get("login"), "workflow approval user login")
        environments = approval.get("environments")
        if not isinstance(environments, list) or not environments:
            raise GateError("workflow approval environments are invalid")
        for value in environments:
            environment = _object(value, "workflow approval environment")
            _positive_int(environment.get("id"), "workflow approval environment id")
            name = _string(environment.get("name"), "workflow approval environment name")
            if state == "approved" and name == ENVIRONMENT_NAME:
                approved = True
    if not approved:
        raise GateError("publisher-integration environment approval is missing")


class GitHubApi:
    """Small read-only GitHub REST client used by the release gate."""

    def __init__(self, api_url: str, repository: str, token: str) -> None:
        if REPOSITORY_PATTERN.fullmatch(repository or "") is None:
            raise GateError("GITHUB_REPOSITORY is invalid")
        parsed = urlsplit(api_url or "")
        if parsed.scheme not in {"http", "https"} or not parsed.netloc or parsed.query or parsed.fragment:
            raise GateError("GITHUB_API_URL is invalid")
        if not token:
            raise GateError("workflow token is missing")
        owner, repo = repository.split("/", 1)
        self.base_url = api_url.rstrip("/")
        self.repository_path = f"/repos/{quote(owner, safe='')}/{quote(repo, safe='')}"
        self.token = token

    def get(self, path: str, query: Sequence[tuple[str, str]] = ()) -> Any:
        if not path.startswith("/"):
            raise GateError("GitHub API path is invalid")
        query_string = urlencode(list(query))
        url = f"{self.base_url}{self.repository_path}{path}"
        if query_string:
            url = f"{url}?{query_string}"
        request = Request(
            url,
            method="GET",
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {self.token}",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        try:
            with urlopen(request, timeout=30) as response:
                status = response.getcode()
                body = response.read()
        except HTTPError as error:
            raise GateError(f"GitHub API request failed with HTTP {error.code}") from error
        except (OSError, URLError, TimeoutError, ValueError) as error:
            raise GateError("GitHub API request failed") from error
        if status != 200:
            raise GateError(f"GitHub API request returned HTTP {status}")
        try:
            return json.loads(body)
        except (TypeError, json.JSONDecodeError) as error:
            raise GateError("GitHub API returned malformed JSON") from error


def _paged(
    api: GitHubApi,
    path: str,
    list_key: str,
    label: str,
    query: Sequence[tuple[str, str]],
) -> dict[str, Any]:
    first = _object(api.get(path, [*query, ("page", "1")]), f"{label} response")
    total_count = _nonnegative_int(first.get("total_count"), f"{label} total_count")
    values = first.get(list_key)
    if not isinstance(values, list) or len(values) > total_count:
        raise GateError(f"{label} response is malformed")
    values = list(values)
    if total_count > 1000:
        raise GateError(f"{label} response exceeds the GitHub API result limit")
    page = 1
    while len(values) < total_count:
        page += 1
        next_page = _object(api.get(path, [*query, ("page", str(page))]), f"{label} response")
        if next_page.get("total_count") != total_count:
            raise GateError(f"{label} total_count changed during pagination")
        next_values = next_page.get(list_key)
        if not isinstance(next_values, list) or not next_values:
            raise GateError(f"{label} pagination is incomplete")
        values.extend(next_values)
    if len(values) != total_count:
        raise GateError(f"{label} pagination is incomplete")
    return {"total_count": total_count, list_key: values}


def verify_publisher_preflight(api: GitHubApi, expected_sha: str) -> dict[str, int]:
    """Verify the exact successful, approved publisher preflight evidence."""
    expected_sha = _sha(expected_sha, "expected source SHA")
    workflow = api.get(f"/actions/workflows/{quote('release-preflight.yml', safe='')}")
    workflow_id = validate_workflow(workflow)
    runs = _paged(
        api,
        f"/actions/workflows/{workflow_id}/runs",
        "workflow_runs",
        "workflow runs",
        [
            ("branch", BRANCH),
            ("event", EVENT),
            ("head_sha", expected_sha),
            ("status", "completed"),
            ("per_page", "100"),
        ],
    )
    run = select_latest_run(runs, expected_sha, workflow_id)
    attempt = api.get(f"/actions/runs/{run['id']}/attempts/{run['run_attempt']}")
    validate_attempt(attempt, run, expected_sha)
    jobs = _paged(
        api,
        f"/actions/runs/{run['id']}/attempts/{run['run_attempt']}/jobs",
        "jobs",
        "workflow jobs",
        [("per_page", "100")],
    )
    job = validate_jobs(jobs, run, expected_sha)
    approvals = api.get(f"/actions/runs/{run['id']}/approvals")
    validate_approvals(approvals)
    return {"run_id": run["id"], "run_attempt": run["run_attempt"], "job_id": job["id"]}


def main() -> int:
    try:
        expected_sha = _sha(os.environ.get("EXPECTED_SOURCE_SHA", ""), "expected source SHA")
        api = GitHubApi(
            os.environ.get("GITHUB_API_URL", "https://api.github.com"),
            os.environ.get("GITHUB_REPOSITORY", ""),
            os.environ.get("GITHUB_TOKEN", ""),
        )
        evidence = verify_publisher_preflight(api, expected_sha)
    except GateError as error:
        print(f"::error::publisher preflight gate blocked: {error}", file=sys.stderr)
        return 1
    except Exception:
        print("::error::publisher preflight gate failed closed on an unexpected error", file=sys.stderr)
        return 1
    print(
        f"verified protected publisher preflight run {evidence['run_id']} "
        f"attempt {evidence['run_attempt']} job {evidence['job_id']} for {expected_sha}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
