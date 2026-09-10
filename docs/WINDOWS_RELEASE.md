# Pump Windows release

Pump's Windows VST3 is a nightly-only release artifact. The release prepare job
captures one immutable `main` source SHA, package version, publication version,
build ID, and timestamp. The reusable Windows job receives those values and
produces an unsigned x86_64 VST3 sidecar; it is never given PortalSurfer
publishing credentials or an OIDC permission.

The Windows build emits the standard bundle at:

```text
dist/Pump-v{package_version}.vst3/Contents/x86_64-win/Pump-v{package_version}.vst3
```

The nightly archive is exactly:

```text
pump-v{publication_version}-windows-x86_64-unsigned.vst3.zip
```

The archive contains only the one VST3 binary at
`Pump-v{package_version}.vst3/Contents/x86_64-win/Pump-v{package_version}.vst3`.
`windows-artifact-manifest.json` records the PE32+ amd64/no-Authenticode
check, archive topology, source SHA, Toybox/GPUI core/GPUI renderer/VST3 SDK revisions, runner,
Rust, and CPython provenance. The macOS assembly validates that sidecar before
creating the schema-3 nightly manifest.

Stable and RC releases remain the existing macOS-only schema-2 path. Manual
Windows workflow runs are inspection-only and do not alter or publish a stable
or RC release.

The required dependency revisions are Toybox
`f6a4a9cc05750d2831752afe05f23a175c85e28f` and GPUI core/renderer
`77a1325a13bd0d3631f737b98445b70c8c936cb7`. Both GPUI crates must use
the same repository and revision. The final publisher checkout is
pinned to PortalSurfer commit
`12d2c089d3d135c6839013a097dbf3baebf5fdb3`, the merged generic publisher.
