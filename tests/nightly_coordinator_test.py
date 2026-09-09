#!/usr/bin/env python3
"""Mocked state-machine tests for the protected Pump nightly coordinator."""

from __future__ import annotations

import copy
import shutil
import sys
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any, Optional

sys.path.insert(0, str(Path(__file__).parents[1] / "scripts"))
import nightly_coordinator as coordinator


MAIN_SHA = "a" * 40
SOURCE_SHA = "b" * 40
BRANCH_SHA = "c" * 40
MERGED_SHA = "d" * 40
DRIFT_SHA = "e" * 40
REPOSITORY = "PORTALSURFER/pump"


def release_history(
    *, source_sha: str = MAIN_SHA, version: str = "0.2.6-nightly.73"
) -> dict[str, Any]:
    return {
        "releases": [
            {
                "channel": "nightly",
                "version": version,
                "build_id": f"pump-v{version}-{source_sha[:12]}",
                "released_at": "2026-09-09T08:55:04Z",
                "source": {"repository": REPOSITORY, "git_sha": source_sha},
            }
        ]
    }


def workflow_run(
    run_id: int,
    run_number: int,
    head_sha: str,
    branch: str,
    *,
    status: str = "completed",
    conclusion: Optional[str] = "success",
) -> dict[str, Any]:
    return {
        "id": run_id,
        "run_number": run_number,
        "run_attempt": 1,
        "head_sha": head_sha,
        "head_branch": branch,
        "event": "workflow_dispatch",
        "status": status,
        "conclusion": conclusion,
        "created_at": f"2026-09-09T09:00:{run_id:02d}Z",
    }


class FakeGit:
    def __init__(
        self,
        *,
        package_version: str = "0.2.6",
        main_sha: str = MAIN_SHA,
        commit_message: str = "",
        branch_state: str = "exact",
        owned_branch: bool = True,
        drift_on_fetch: Optional[int] = None,
    ) -> None:
        self.package = package_version
        self.main = main_sha
        self.message = commit_message
        self.branch_state = branch_state
        self.owned_branch = owned_branch
        self.drift_on_fetch = drift_on_fetch
        self.fetch_count = 0
        self.prepare_calls: list[dict[str, Any]] = []
        self.delete_calls: list[tuple[str, str]] = []
        self.api: Optional[FakeApi] = None
        self.pending_package: Optional[str] = None

    def ensure_clean(self) -> None:
        pass

    def fetch_main(self) -> None:
        self.fetch_count += 1
        if self.drift_on_fetch == self.fetch_count:
            self.main = DRIFT_SHA

    def fetch_version_branch(self) -> None:
        pass

    def main_sha(self) -> str:
        return self.main

    def package_version(self, ref: Optional[str] = None) -> str:
        return self.package

    def commit_message(self, ref: Optional[str] = None) -> str:
        return self.message

    def validate_version_branch(self, *, base_sha: str, target_version: str, branch_sha: str) -> str:
        if not self.owned_branch:
            raise coordinator.CoordinatorError("version branch is not an owned nightly bump")
        return self.branch_state

    def owned_version_branch(self, branch_sha: str, *, target_version: str) -> bool:
        return self.owned_branch

    def prepare_version_branch(
        self, *, base_sha: str, target_version: str, existing_branch_sha: Optional[str]
    ) -> str:
        self.prepare_calls.append(
            {
                "base_sha": base_sha,
                "target_version": target_version,
                "existing_branch_sha": existing_branch_sha,
            }
        )
        if self.api is not None:
            self.api.branch = BRANCH_SHA
        self.pending_package = target_version
        return BRANCH_SHA

    def delete_remote_branch(self, *, branch: str, expected_sha: str) -> None:
        self.delete_calls.append((branch, expected_sha))
        if self.api is not None:
            self.api.branch = None


class FakeApi:
    def __init__(
        self,
        git: FakeGit,
        *,
        branch: Optional[str] = None,
        open_prs: Optional[list[dict[str, Any]]] = None,
        workflow_runs: Optional[dict[str, list[dict[str, Any]]]] = None,
        on_dispatch: Any = None,
    ) -> None:
        self.git = git
        git.api = self
        self.branch = branch
        self.open_prs = open_prs or []
        self.pull_requests: dict[int, dict[str, Any]] = {}
        for pr in self.open_prs:
            self.pull_requests[int(pr["number"])] = copy.deepcopy(pr)
        self.runs = copy.deepcopy(workflow_runs or {})
        self.on_dispatch = on_dispatch
        self.dispatches: list[tuple[str, str, dict[str, str]]] = []
        self.workflow_queries: list[tuple[str, str, str]] = []
        self.created: list[dict[str, Any]] = []
        self.merges: list[dict[str, Any]] = []
        self.next_pr_number = 101
        self.next_run_id = 10
        self.next_run_number = 80
        self.fail_list = False

    def list_open_pull_requests(self) -> list[dict[str, Any]]:
        if self.fail_list:
            raise coordinator.CoordinatorError("mock API failure")
        return copy.deepcopy(self.open_prs)

    def branch_sha(self, branch: str) -> Optional[str]:
        return self.branch

    def create_pull_request(self, *, title: str, body: str, head: str, base: str) -> dict[str, Any]:
        pr = {
            "number": self.next_pr_number,
            "title": title,
            "body": body,
            "head": {"ref": head, "sha": self.branch, "repo": {"full_name": REPOSITORY}},
            "base": {"ref": base},
        }
        self.next_pr_number += 1
        self.created.append(copy.deepcopy(pr))
        self.open_prs.append(copy.deepcopy(pr))
        self.pull_requests[int(pr["number"])] = copy.deepcopy(pr)
        return pr

    def get_pull_request(self, number: int) -> dict[str, Any]:
        return copy.deepcopy(self.pull_requests[number])

    def merge_pull_request(
        self, *, number: int, head_sha: str, title: str, commit_message: str = ""
    ) -> str:
        pr = self.pull_requests[number]
        if pr["head"]["sha"] != head_sha:
            raise coordinator.CoordinatorError("mock merge SHA mismatch")
        self.merges.append(
            {
                "number": number,
                "head_sha": head_sha,
                "title": title,
                "commit_message": commit_message,
            }
        )
        self.git.main = MERGED_SHA
        self.git.package = self.git.pending_package or self.git.package
        return MERGED_SHA

    def dispatch_workflow(
        self, workflow: str, *, ref: str, inputs: Optional[dict[str, str]] = None
    ) -> None:
        normalized = dict(inputs or {})
        self.dispatches.append((workflow, ref, normalized))
        run = workflow_run(
            self.next_run_id,
            self.next_run_number,
            MERGED_SHA if ref == coordinator.BASE_BRANCH and self.git.main == MERGED_SHA else (
                BRANCH_SHA if ref == coordinator.VERSION_BRANCH else self.git.main
            ),
            ref,
        )
        self.next_run_id += 1
        self.next_run_number += 1
        self.runs.setdefault(workflow, []).append(run)
        if self.on_dispatch is not None:
            self.on_dispatch(workflow, ref, normalized, run)

    def workflow_runs(self, workflow: str, *, branch: str, head_sha: str) -> list[dict[str, Any]]:
        self.workflow_queries.append((workflow, branch, head_sha))
        return copy.deepcopy(
            [
                run
                for run in self.runs.get(workflow, [])
                if run.get("head_branch") == branch and run.get("head_sha") == head_sha
            ]
        )


class Clock:
    def __init__(self) -> None:
        self.value = 0.0

    def monotonic(self) -> float:
        return self.value

    def sleep(self, seconds: float) -> None:
        self.value += seconds


def run_coordinator(
    git: FakeGit, api: FakeApi, document: dict[str, Any], *, force: bool = False
) -> Optional[str]:
    clock = Clock()
    return coordinator.NightlyCoordinator(
        git=git,
        api=api,
        release_document=document,
        release_document_loader=lambda: document,
        sleep=clock.sleep,
        monotonic=clock.monotonic,
        poll_seconds=1,
        timeout_seconds=100,
    ).run(force=force)


class CoordinatorTests(unittest.TestCase):
    def test_unchanged_source_skips_without_mutation(self) -> None:
        git = FakeGit()
        api = FakeApi(git)
        self.assertIsNone(run_coordinator(git, api, release_history()))
        self.assertEqual(api.dispatches, [])
        self.assertEqual(git.prepare_calls, [])

    def test_bump_pr_checks_merge_and_release_use_exact_identity(self) -> None:
        git = FakeGit()
        document = release_history(source_sha=SOURCE_SHA)

        def publish_on_dispatch(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW and inputs.get("publish") == "true":
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MERGED_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MERGED_SHA},
                    }
                )

        api = FakeApi(git, on_dispatch=publish_on_dispatch)
        document = release_history(source_sha=SOURCE_SHA)
        self.assertEqual(run_coordinator(git, api, document), MERGED_SHA)
        self.assertEqual(
            [(workflow, ref) for workflow, ref, _ in api.dispatches],
            [
                (coordinator.CI_WORKFLOW, coordinator.VERSION_BRANCH),
                (coordinator.PREFLIGHT_WORKFLOW, coordinator.VERSION_BRANCH),
                (coordinator.PREFLIGHT_WORKFLOW, coordinator.BASE_BRANCH),
                (coordinator.RELEASE_WORKFLOW, coordinator.BASE_BRANCH),
            ],
        )
        self.assertEqual(
            api.dispatches[-1][2],
            {"channel": "nightly", "publish": "true", "only_if_changed": "false"},
        )
        self.assertEqual(api.merges[0]["head_sha"], BRANCH_SHA)
        self.assertIn(coordinator.VERSION_METADATA_PREFIX + "0.2.7", api.merges[0]["commit_message"])
        self.assertEqual(git.delete_calls, [(coordinator.VERSION_BRANCH, BRANCH_SHA)])

    def test_empty_history_reuses_owned_merged_pending_version(self) -> None:
        git = FakeGit(
            package_version="0.2.7",
            commit_message=(
                f"{coordinator.VERSION_PR_MARKER}\n"
                f"{coordinator.VERSION_METADATA_PREFIX}0.2.7\n"
            ),
        )
        document: dict[str, Any] = {"releases": []}

        def publish_on_dispatch(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MAIN_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MAIN_SHA},
                    }
                )

        api = FakeApi(git, on_dispatch=publish_on_dispatch)
        self.assertEqual(run_coordinator(git, api, document), MAIN_SHA)
        self.assertEqual(git.prepare_calls, [])
        self.assertEqual(api.created, [])
        self.assertEqual(
            [dispatch[0] for dispatch in api.dispatches],
            [coordinator.PREFLIGHT_WORKFLOW, coordinator.RELEASE_WORKFLOW],
        )

    def test_successful_package_only_release_is_retried_with_publish_true(self) -> None:
        git = FakeGit()
        document = release_history(source_sha=SOURCE_SHA)
        old_run = workflow_run(1, 74, MAIN_SHA, coordinator.BASE_BRANCH)

        def publish_on_dispatch(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MAIN_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MAIN_SHA},
                    }
                )

        api = FakeApi(git, workflow_runs={coordinator.RELEASE_WORKFLOW: [old_run]}, on_dispatch=publish_on_dispatch)
        # This direct pending state avoids a version PR and tests the release
        # resume path independently of the bump workflow.
        git.package = "0.2.7"
        git.message = f"{coordinator.VERSION_PR_MARKER}\n{coordinator.VERSION_METADATA_PREFIX}0.2.7\n"
        self.assertEqual(run_coordinator(git, api, document), MAIN_SHA)
        release_dispatches = [d for d in api.dispatches if d[0] == coordinator.RELEASE_WORKFLOW]
        self.assertEqual(len(release_dispatches), 1)
        self.assertEqual(
            release_dispatches[0][2],
            {"channel": "nightly", "publish": "true", "only_if_changed": "false"},
        )

    def test_public_history_error_fails_closed_without_production_retry(self) -> None:
        git = FakeGit(package_version="0.2.7")
        api = FakeApi(git)
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document={"releases": []},
            release_document_loader=lambda: {"releases": [{"channel": "nightly"}]},
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        successful = coordinator.WorkflowRun(
            run_id=1,
            run_number=80,
            run_attempt=1,
            head_sha=MAIN_SHA,
            head_branch=coordinator.BASE_BRANCH,
            event="workflow_dispatch",
            status="completed",
            conclusion="success",
            created_at="2026-09-09T09:00:00Z",
        )
        with self.assertRaisesRegex(coordinator.CoordinatorError, "history is invalid"):
            instance._require_release_published(successful, MAIN_SHA)
        self.assertEqual(api.dispatches, [])

    def test_concurrent_publisher_is_waited_and_accepted_by_core_and_source(self) -> None:
        git = FakeGit(package_version="0.2.7")
        document: dict[str, Any] = {"releases": []}
        old = workflow_run(1, 80, MAIN_SHA, coordinator.BASE_BRANCH)
        concurrent = workflow_run(
            2,
            81,
            MAIN_SHA,
            coordinator.BASE_BRANCH,
            status="in_progress",
            conclusion=None,
        )
        api = FakeApi(
            git,
            workflow_runs={coordinator.RELEASE_WORKFLOW: [old, concurrent]},
        )
        query_count = 0

        def workflow_runs(workflow: str, *, branch: str, head_sha: str) -> list[dict[str, Any]]:
            nonlocal query_count
            query_count += 1
            if query_count == 2:
                concurrent["status"] = "completed"
                concurrent["conclusion"] = "success"
                publication = "0.2.7-nightly.999"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MAIN_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MAIN_SHA},
                    }
                )
                api.runs[coordinator.RELEASE_WORKFLOW] = [old, concurrent]
            return [
                copy.deepcopy(run)
                for run in api.runs.get(workflow, [])
                if run.get("head_branch") == branch and run.get("head_sha") == head_sha
            ]

        api.workflow_runs = workflow_runs  # type: ignore[method-assign]
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document=document,
            release_document_loader=lambda: document,
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        successful = coordinator.WorkflowRun(
            run_id=old["id"],
            run_number=old["run_number"],
            run_attempt=1,
            head_sha=MAIN_SHA,
            head_branch=coordinator.BASE_BRANCH,
            event="workflow_dispatch",
            status="completed",
            conclusion="success",
            created_at=old["created_at"],
        )
        instance._require_release_published(successful, MAIN_SHA)
        self.assertEqual(api.dispatches, [])
        self.assertGreaterEqual(query_count, 2)

    def test_existing_active_run_stays_pinned_when_a_newer_run_appears(self) -> None:
        git = FakeGit()
        api = FakeApi(git)
        active = workflow_run(
            1,
            80,
            MAIN_SHA,
            coordinator.BASE_BRANCH,
            status="in_progress",
            conclusion=None,
        )
        api.runs[coordinator.RELEASE_WORKFLOW] = [active]
        query_count = 0

        def workflow_runs(workflow: str, *, branch: str, head_sha: str) -> list[dict[str, Any]]:
            nonlocal query_count
            query_count += 1
            if query_count == 2:
                api.runs[coordinator.RELEASE_WORKFLOW].append(
                    workflow_run(2, 81, MAIN_SHA, coordinator.BASE_BRANCH)
                )
            if query_count == 3:
                api.runs[coordinator.RELEASE_WORKFLOW][0]["status"] = "completed"
                api.runs[coordinator.RELEASE_WORKFLOW][0]["conclusion"] = "success"
            return [
                copy.deepcopy(run)
                for run in api.runs.get(workflow, [])
                if run.get("head_branch") == branch and run.get("head_sha") == head_sha
            ]

        api.workflow_runs = workflow_runs  # type: ignore[method-assign]
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document={"releases": []},
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        result = instance._ensure_workflow(
            coordinator.RELEASE_WORKFLOW,
            branch=coordinator.BASE_BRANCH,
            head_sha=MAIN_SHA,
        )
        self.assertEqual(result.run_id, 1)
        self.assertEqual(api.dispatches, [])

    def test_failed_pinned_run_clears_pin_before_retry(self) -> None:
        git = FakeGit()
        api = FakeApi(git)
        active = workflow_run(
            1,
            80,
            MAIN_SHA,
            coordinator.BASE_BRANCH,
            status="in_progress",
            conclusion=None,
        )
        api.runs[coordinator.RELEASE_WORKFLOW] = [active]
        query_count = 0

        def workflow_runs(workflow: str, *, branch: str, head_sha: str) -> list[dict[str, Any]]:
            nonlocal query_count
            query_count += 1
            if query_count == 2:
                api.runs[coordinator.RELEASE_WORKFLOW][0]["status"] = "completed"
                api.runs[coordinator.RELEASE_WORKFLOW][0]["conclusion"] = "failure"
            return [
                copy.deepcopy(run)
                for run in api.runs.get(workflow, [])
                if run.get("head_branch") == branch and run.get("head_sha") == head_sha
            ]

        api.workflow_runs = workflow_runs  # type: ignore[method-assign]
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document={"releases": []},
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        result = instance._ensure_workflow(
            coordinator.RELEASE_WORKFLOW,
            branch=coordinator.BASE_BRANCH,
            head_sha=MAIN_SHA,
        )
        self.assertEqual(len(api.dispatches), 1)
        self.assertEqual(result.run_id, 10)

    def test_failed_release_with_public_artifact_is_not_retried(self) -> None:
        git = FakeGit(package_version="0.2.7")
        document = release_history(source_sha=MAIN_SHA, version="0.2.7-nightly.999")
        api = FakeApi(git)
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document=document,
            release_document_loader=lambda: document,
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        failed = coordinator._parse_run(
            workflow_run(
                1,
                80,
                MAIN_SHA,
                coordinator.BASE_BRANCH,
                conclusion="failure",
            ),
            expected_sha=MAIN_SHA,
            expected_branch=coordinator.BASE_BRANCH,
        )
        instance._require_release_published(failed, MAIN_SHA)
        self.assertEqual(api.dispatches, [])

    def test_open_owned_pr_is_refreshed_after_branch_preparation(self) -> None:
        body = f"{coordinator.VERSION_PR_MARKER}\nPrepare 0.2.7"
        pr = {
            "number": 42,
            "title": "prepare",
            "body": body,
            "head": {"ref": coordinator.VERSION_BRANCH, "sha": BRANCH_SHA, "repo": {"full_name": REPOSITORY}},
            "base": {"ref": coordinator.BASE_BRANCH},
        }
        git = FakeGit(branch_state="exact")
        api = FakeApi(git, branch=BRANCH_SHA, open_prs=[pr])
        document = release_history(source_sha=SOURCE_SHA)
        # No release publication is needed for this test's API assertions.
        def publish(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MERGED_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MERGED_SHA},
                    }
                )
        api.on_dispatch = publish
        self.assertEqual(run_coordinator(git, api, document), MERGED_SHA)
        self.assertEqual(api.created, [])

    def test_foreign_pull_request_fails_closed(self) -> None:
        pr = {
            "number": 42,
            "body": coordinator.VERSION_PR_MARKER,
            "head": {
                "ref": coordinator.VERSION_BRANCH,
                "sha": BRANCH_SHA,
                "repo": {"full_name": "someone/fork"},
            },
            "base": {"ref": coordinator.BASE_BRANCH},
        }
        git = FakeGit()
        api = FakeApi(git, open_prs=[pr])
        with self.assertRaisesRegex(coordinator.CoordinatorError, "foreign"):
            run_coordinator(git, api, release_history(source_sha=SOURCE_SHA))

    def test_interrupted_branch_push_recovers_only_owned_branch(self) -> None:
        git = FakeGit(branch_state="stale-owned")
        api = FakeApi(git, branch=BRANCH_SHA)
        document = release_history(source_sha=SOURCE_SHA)
        def publish(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MERGED_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MERGED_SHA},
                    }
                )
        api.on_dispatch = publish
        self.assertEqual(run_coordinator(git, api, document), MERGED_SHA)
        self.assertEqual(git.prepare_calls[0]["existing_branch_sha"], BRANCH_SHA)
        self.assertEqual(len(api.created), 1)

    def test_unowned_orphan_branch_fails_closed(self) -> None:
        git = FakeGit(owned_branch=False)
        api = FakeApi(git, branch=BRANCH_SHA)
        with self.assertRaisesRegex(coordinator.CoordinatorError, "not an owned"):
            run_coordinator(git, api, release_history(source_sha=SOURCE_SHA))

    def test_main_drift_before_merge_stops_before_publication(self) -> None:
        git = FakeGit(drift_on_fetch=3)
        api = FakeApi(git, branch=None)
        with self.assertRaisesRegex(coordinator.CoordinatorError, "main moved"):
            run_coordinator(git, api, release_history(source_sha=SOURCE_SHA))
        self.assertEqual(api.merges, [])
        self.assertNotIn(coordinator.RELEASE_WORKFLOW, [dispatch[0] for dispatch in api.dispatches])

    def test_failed_workflow_gets_one_exact_retry(self) -> None:
        git = FakeGit(package_version="0.2.7", commit_message=f"{coordinator.VERSION_PR_MARKER}\n{coordinator.VERSION_METADATA_PREFIX}0.2.7\n")
        document: dict[str, Any] = {"releases": []}
        failed = workflow_run(1, 79, MAIN_SHA, coordinator.BASE_BRANCH, conclusion="failure")

        def publish(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.7-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MAIN_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MAIN_SHA},
                    }
                )

        api = FakeApi(git, workflow_runs={coordinator.RELEASE_WORKFLOW: [failed]}, on_dispatch=publish)
        self.assertEqual(run_coordinator(git, api, document), MAIN_SHA)
        self.assertEqual(len([d for d in api.dispatches if d[0] == coordinator.RELEASE_WORKFLOW]), 1)

    def test_force_bump_ignores_old_merge_marker_when_history_exists(self) -> None:
        git = FakeGit(
            package_version="0.2.7",
            commit_message=(
                f"{coordinator.VERSION_PR_MARKER}\n"
                f"{coordinator.VERSION_METADATA_PREFIX}0.2.7\n"
            ),
        )
        document = release_history(source_sha=MAIN_SHA, version="0.2.7-nightly.80")

        def publish(workflow: str, ref: str, inputs: dict[str, str], run: dict[str, Any]) -> None:
            if workflow == coordinator.RELEASE_WORKFLOW:
                publication = f"0.2.8-nightly.{run['run_number']}"
                document["releases"].append(
                    {
                        "channel": "nightly",
                        "version": publication,
                        "build_id": f"pump-v{publication}-{MERGED_SHA[:12]}",
                        "released_at": "2026-09-09T09:10:00Z",
                        "source": {"repository": REPOSITORY, "git_sha": MERGED_SHA},
                    }
                )

        api = FakeApi(git, on_dispatch=publish)
        self.assertEqual(run_coordinator(git, api, document, force=True), MERGED_SHA)
        self.assertEqual(git.prepare_calls[0]["target_version"], "0.2.8")
        self.assertEqual(api.merges[0]["commit_message"].count("0.2.8"), 1)

    def test_pending_workflow_status_is_waited(self) -> None:
        git = FakeGit()
        api = FakeApi(git)
        api.runs[coordinator.CI_WORKFLOW] = [
            workflow_run(1, 1, BRANCH_SHA, coordinator.VERSION_BRANCH, status="pending", conclusion=None)
        ]
        clock = Clock()
        instance = coordinator.NightlyCoordinator(
            git=git,
            api=api,
            release_document=release_history(),
            sleep=clock.sleep,
            monotonic=clock.monotonic,
            poll_seconds=1,
            timeout_seconds=100,
        )
        # The fake run is already pending forever; the bounded clock proves
        # that it is treated as active and eventually times out.
        with self.assertRaisesRegex(coordinator.CoordinatorError, "timed out"):
            instance._ensure_workflow(coordinator.CI_WORKFLOW, branch=coordinator.VERSION_BRANCH, head_sha=BRANCH_SHA)

    def test_github_adapter_requires_product_repository_and_release_inputs(self) -> None:
        with self.assertRaisesRegex(coordinator.CoordinatorError, "repository"):
            coordinator.GitHubApi(api_url="https://api.github.com", repository="fork/pump", token="x")
        api = coordinator.GitHubApi(
            api_url="https://api.github.com", repository=REPOSITORY, token="x"
        )
        calls: list[dict[str, Any]] = []

        def request(method: str, path: str, *, payload: Any = None, expected: Any = None, query: Any = ()) -> None:
            calls.append({"method": method, "path": path, "payload": payload})
            return None

        api.request = request  # type: ignore[method-assign]
        api.dispatch_workflow(
            coordinator.RELEASE_WORKFLOW,
            ref=coordinator.BASE_BRANCH,
            inputs={"channel": "nightly", "publish": "true", "only_if_changed": "false"},
        )
        self.assertEqual(
            calls[-1]["payload"],
            {"ref": coordinator.BASE_BRANCH, "inputs": {"channel": "nightly", "publish": "true", "only_if_changed": "false"}},
        )


class LocalVersionBranchTests(unittest.TestCase):
    """Exercise the real two-file branch and leased cleanup against a bare remote."""

    def git(self, root: Path, *args: str, check: bool = True) -> str:
        result = subprocess.run(
            ["git", *args], cwd=root, text=True, capture_output=True, check=False
        )
        if check and result.returncode != 0:
            self.fail(f"git {' '.join(args)} failed: {result.stderr}")
        return result.stdout.strip()

    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="pump-nightly-git-")
        root = Path(self.temporary.name)
        self.bare = root / "origin.git"
        self.seed = root / "seed"
        self.clone = root / "scheduler"
        self.git(root, "init", "--bare", str(self.bare))
        self.seed.mkdir()
        self.git(self.seed, "init", "-b", "main")
        self.git(self.seed, "config", "user.name", "test")
        self.git(self.seed, "config", "user.email", "test@example.com")
        source = Path(__file__).parents[1]
        (self.seed / "scripts").mkdir()
        shutil.copy2(source / "Cargo.toml", self.seed / "Cargo.toml")
        shutil.copy2(source / "Cargo.lock", self.seed / "Cargo.lock")
        shutil.copy2(source / "scripts" / "bump_version.py", self.seed / "scripts" / "bump_version.py")
        self.git(self.seed, "add", "Cargo.toml", "Cargo.lock", "scripts/bump_version.py")
        self.git(self.seed, "commit", "-m", "base")
        self.git(self.seed, "remote", "add", "origin", str(self.bare))
        self.git(self.seed, "push", "origin", "main")
        self.git(root, "clone", str(self.bare), str(self.clone))
        self.git(self.clone, "config", "user.name", "test")
        self.git(self.clone, "config", "user.email", "test@example.com")
        self.repo = coordinator.GitRepository(self.clone)
        self.repo.fetch_main()
        self.base = self.repo.main_sha()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def test_create_reuse_tamper_rejection_and_atomic_cleanup(self) -> None:
        branch_sha = self.repo.prepare_version_branch(
            base_sha=self.base,
            target_version="0.2.7",
            existing_branch_sha=None,
        )
        self.assertEqual(
            self.repo.validate_version_branch(
                base_sha=self.base,
                target_version="0.2.7",
                branch_sha=branch_sha,
            ),
            "exact",
        )
        self.assertEqual(
            self.repo.prepare_version_branch(
                base_sha=self.base,
                target_version="0.2.7",
                existing_branch_sha=branch_sha,
            ),
            branch_sha,
        )

        tamper = self.clone.parent / "tamper"
        self.git(self.clone.parent, "clone", str(self.bare), str(tamper))
        self.git(tamper, "config", "user.name", "tamper")
        self.git(tamper, "config", "user.email", "tamper@example.com")
        self.git(tamper, "switch", "--detach", branch_sha)
        lock = tamper / "Cargo.lock"
        lock.write_text(
            lock.read_text(encoding="utf-8").replace('name = "cocoa"', 'name = "cocoa-tampered"', 1),
            encoding="utf-8",
        )
        self.git(
            tamper,
            "add",
            "Cargo.lock",
        )
        self.git(
            tamper,
            "commit",
            "-m",
            "tamper",
            "-m",
            f"{coordinator.VERSION_PR_MARKER}\n{coordinator.VERSION_METADATA_PREFIX}0.2.7",
        )
        tampered_sha = self.git(tamper, "rev-parse", "HEAD")
        self.git(tamper, "push", "origin", f"HEAD:{coordinator.VERSION_BRANCH}")
        self.repo.fetch_version_branch()
        with self.assertRaisesRegex(coordinator.CoordinatorError, "unexpected files|differs"):
            self.repo.validate_version_branch(
                base_sha=self.base,
                target_version="0.2.7",
                branch_sha=tampered_sha,
            )
        with self.assertRaises(coordinator.CoordinatorError):
            self.repo.delete_remote_branch(
                branch=coordinator.VERSION_BRANCH,
                expected_sha=branch_sha,
            )
        self.assertEqual(self.repo.remote_branch_sha(coordinator.VERSION_BRANCH), tampered_sha)
        self.repo.delete_remote_branch(
            branch=coordinator.VERSION_BRANCH,
            expected_sha=tampered_sha,
        )
        self.assertIsNone(self.repo.remote_branch_sha(coordinator.VERSION_BRANCH))

    def test_tampered_stale_branch_cannot_be_cleaned_after_merge(self) -> None:
        branch_sha = self.repo.prepare_version_branch(
            base_sha=self.base,
            target_version="0.2.7",
            existing_branch_sha=None,
        )
        tamper = self.clone.parent / "tamper-stale"
        self.git(self.clone.parent, "clone", str(self.bare), str(tamper))
        self.git(tamper, "config", "user.name", "tamper")
        self.git(tamper, "config", "user.email", "tamper@example.com")
        self.git(tamper, "switch", "--detach", branch_sha)
        lock = tamper / "Cargo.lock"
        lock.write_text(
            lock.read_text(encoding="utf-8").replace('name = "cocoa"', 'name = "cocoa-tampered"', 1),
            encoding="utf-8",
        )
        self.git(tamper, "add", "Cargo.lock")
        self.git(
            tamper,
            "commit",
            "-m",
            "tamper stale branch",
            "-m",
            f"{coordinator.VERSION_PR_MARKER}\n{coordinator.VERSION_METADATA_PREFIX}0.2.7",
        )
        tampered_sha = self.git(tamper, "rev-parse", "HEAD")
        self.git(tamper, "push", "origin", f"HEAD:{coordinator.VERSION_BRANCH}")

        # Merge the package bump on main separately so the coordinator enters
        # its release-pending cleanup path with this stale, tampered branch.
        self.git(self.seed, "switch", "main")
        bump = self.seed / "scripts" / "bump_version.py"
        result = subprocess.run(
            [sys.executable, str(bump), "0.2.7"],
            cwd=self.seed,
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.git(self.seed, "add", "Cargo.toml", "Cargo.lock")
        self.git(
            self.seed,
            "commit",
            "-m",
            "chore(release): prepare Pump v0.2.7 nightly",
            "-m",
            f"{coordinator.VERSION_PR_MARKER}\n{coordinator.VERSION_METADATA_PREFIX}0.2.7",
        )
        self.git(self.seed, "push", "origin", "main")
        self.repo.fetch_main()
        merged_main = self.repo.main_sha()

        class ReleasePendingApi:
            def list_open_pull_requests(self) -> list[dict[str, Any]]:
                return []

            def branch_sha(self, branch: str) -> Optional[str]:
                return tampered_sha

        instance = coordinator.NightlyCoordinator(
            git=self.repo,
            api=ReleasePendingApi(),
            release_document=release_history(source_sha=SOURCE_SHA),
            sleep=lambda _: None,
            monotonic=lambda: 0.0,
            poll_seconds=1,
            timeout_seconds=100,
        )
        with self.assertRaisesRegex(coordinator.CoordinatorError, "not owned"):
            instance.run()
        self.assertEqual(self.repo.main_sha(), merged_main)
        self.assertEqual(self.repo.remote_branch_sha(coordinator.VERSION_BRANCH), tampered_sha)


if __name__ == "__main__":
    unittest.main()
