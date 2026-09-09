# Nightly releases

Use the `Pump nightly scheduler` workflow (`nightly.yml`) to prepare and publish a new nightly:

```bash
gh workflow run nightly.yml --repo PORTALSURFER/pump --ref main
```

A scheduled run skips a source commit already published as a nightly. Use `-f force=true` to request a new nightly even without source changes.

Each new nightly advances the package patch version in both `Cargo.toml` and `Cargo.lock`. For example, a prepared release is `0.2.7-nightly.<run-number>`. The workflow sequence distinguishes attempts; it does not replace the package patch increment. If a version was prepared but publication failed, retrying reuses that unpublished package version.

## Protected preparation

The coordinator creates or reuses a version-only PR, explicitly dispatches CI and release preflight for its exact head, and merges only after the checks pass. It then explicitly dispatches the protected preflight on merged `main` and waits for success before requesting the production release. If `main` moves during validation, the run stops instead of publishing a different commit.

Explicit dispatch is necessary because changes made with `GITHUB_TOKEN` do not trigger the usual push workflows. The repository allows Actions to create PRs, while its default workflow token remains read-only. Only the nightly coordinator receives repository contents, pull-request, and Actions write permissions. Branch protection is retained.

Existing protected environment approvals still apply. Approve the publisher-integration and production environments when requested by GitHub. Neither the coordinator nor its token approves those environments automatically.

## Retries and direct releases

Run the scheduler again after an interrupted attempt. An already prepared, unpublished patch is reused, and completed validation can be reused only for the exact source commit. A failed or incomplete check blocks publication.

Direct production nightly dispatch through `release.yml` requires an already prepared package version newer than public release history. A new run cannot publish another nightly suffix using an already published patch. An exact retry of a published build is a no-op. Stable and RC release behavior is unchanged.

All platform bundles use the same package version, source SHA, publication version, and build identity. macOS signing/notarization, Windows artifact validation, and the approved publisher-preflight gate remain part of production publication.
