#!/usr/bin/env python3
"""Focused fail-closed tests for the protected publisher preflight gate."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
import release_preflight_gate


SOURCE_SHA = "a" * 40
WORKFLOW_ID = 17


def workflow() -> dict:
    return {
        "id": WORKFLOW_ID,
        "name": "Pump release preflight",
        "path": ".github/workflows/release-preflight.yml",
        "state": "active",
    }


def run(run_id: int, run_number: int, *, conclusion: str = "success") -> dict:
    return {
        "id": run_id,
        "workflow_id": WORKFLOW_ID,
        "run_number": run_number,
        "run_attempt": 1,
        "created_at": "2026-09-04T10:00:00Z",
        "updated_at": f"2026-09-04T10:{run_number:02d}:00Z",
        "name": "Pump release preflight",
        "path": ".github/workflows/release-preflight.yml",
        "head_sha": SOURCE_SHA,
        "head_branch": "main",
        "event": "push",
        "status": "completed",
        "conclusion": conclusion,
    }


def jobs(run_id: int, *, conclusion: str = "success") -> dict:
    return {
        "total_count": 1,
        "jobs": [
            {
                "id": 71,
                "run_id": run_id,
                "run_attempt": 1,
                "head_sha": SOURCE_SHA,
                "head_branch": "main",
                "workflow_name": "Pump release preflight",
                "name": "publisher_integration",
                "status": "completed",
                "conclusion": conclusion,
            }
        ],
    }


APPROVALS = [
    {
        "state": "approved",
        "user": {"login": "release-owner"},
        "environments": [{"id": 9, "name": "publisher-integration"}],
    }
]


class FakeApi:
    def __init__(self, responses: dict[tuple[str, tuple[tuple[str, str], ...]], object]) -> None:
        self.responses = responses
        self.calls: list[tuple[str, tuple[tuple[str, str], ...]]] = []

    def get(self, path: str, query=()):
        normalized = tuple(query)
        self.calls.append((path, normalized))
        try:
            return self.responses[(path, normalized)]
        except KeyError as error:
            raise AssertionError(f"unexpected API request: {path} {normalized}") from error


def successful_api(*, newer_conclusion: str = "success") -> FakeApi:
    older = run(100, 10)
    newer = run(101, 11, conclusion=newer_conclusion)
    return FakeApi(
        {
            ("/actions/workflows/release-preflight.yml", ()): workflow(),
            (
                f"/actions/workflows/{WORKFLOW_ID}/runs",
                (
                    ("branch", "main"),
                    ("event", "push"),
                    ("head_sha", SOURCE_SHA),
                    ("status", "completed"),
                    ("per_page", "100"),
                    ("page", "1"),
                ),
            ): {"total_count": 2, "workflow_runs": [older, newer]},
            (f"/actions/runs/{newer['id']}/attempts/1", ()): newer,
            (
                f"/actions/runs/{newer['id']}/attempts/1/jobs",
                (("per_page", "100"), ("page", "1")),
            ): jobs(newer["id"]),
            (f"/actions/runs/{newer['id']}/approvals", ()): APPROVALS,
        }
    )


class ReleasePreflightGateTests(unittest.TestCase):
    def test_verify_uses_completed_exact_sha_query_and_exact_identity(self) -> None:
        api = successful_api()

        evidence = release_preflight_gate.verify_publisher_preflight(api, SOURCE_SHA)

        self.assertEqual(evidence, {"run_id": 101, "run_attempt": 1, "job_id": 71})
        run_query = api.calls[1][1]
        self.assertIn(("status", "completed"), run_query)
        self.assertIn(("head_sha", SOURCE_SHA), run_query)
        self.assertIn(("branch", "main"), run_query)
        self.assertIn(("event", "push"), run_query)

    def test_newer_failed_or_cancelled_run_blocks_even_with_older_success(self) -> None:
        for conclusion in ("failure", "cancelled"):
            with self.subTest(conclusion=conclusion):
                with self.assertRaisesRegex(release_preflight_gate.GateError, "latest"):
                    release_preflight_gate.select_latest_run(
                        {
                            "total_count": 2,
                            "workflow_runs": [run(100, 10), run(101, 11, conclusion=conclusion)],
                        },
                        SOURCE_SHA,
                        WORKFLOW_ID,
                    )

    def test_missing_or_unapproved_environment_evidence_blocks(self) -> None:
        for approvals in ([], [{"state": "approved", "user": {"login": "owner"}, "environments": [{"id": 9, "name": "production"}]}]):
            with self.subTest(approvals=approvals):
                with self.assertRaises(release_preflight_gate.GateError):
                    release_preflight_gate.validate_approvals(approvals)

    def test_skipped_publisher_job_and_wrong_identity_block(self) -> None:
        selected = run(101, 11)
        with self.assertRaises(release_preflight_gate.GateError):
            release_preflight_gate.validate_jobs(jobs(selected["id"], conclusion="skipped"), selected, SOURCE_SHA)
        wrong = jobs(selected["id"])["jobs"][0]
        wrong["head_branch"] = "feature"
        with self.assertRaises(release_preflight_gate.GateError):
            release_preflight_gate.validate_jobs({"total_count": 1, "jobs": [wrong]}, selected, SOURCE_SHA)

    def test_workflow_gates_only_publishing_changed_releases(self) -> None:
        release = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
        gate_start = release.index("  publisher_preflight_gate:\n")
        macos_start = release.index("  macos_release:\n", gate_start)
        gate = release[gate_start:macos_start]
        macos = release[macos_start:]
        self.assertIn("if: ${{ inputs.publish == true && needs.prepare.outputs.should_release == 'true' }}", gate)
        self.assertIn("needs: [prepare, windows, publisher_preflight_gate]", macos)
        self.assertIn("inputs.publish != true || needs.publisher_preflight_gate.result == 'success'", macos)
        self.assertIn("environment: production", macos)


if __name__ == "__main__":
    unittest.main()
