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
check, archive topology, source SHA, Toybox/Radiant/VST3 SDK revisions, runner,
Rust, and CPython provenance. The macOS assembly validates that sidecar before
creating the schema-3 nightly manifest.

Stable and RC releases remain the existing macOS-only schema-2 path. Manual
Windows workflow runs are inspection-only and do not alter or publish a stable
or RC release.

The required dependency revisions are Toybox
`a69df15593a5cb9320993dde8d9908bfe857a9f6` and Radiant
`c1343993c973bdece3e8cd469415b0d08c7f6cf1`. The final publisher checkout is
temporarily pinned to PortalSurfer commit
`165776d6707ab6d9e8bb76b2a8866654140ca6bc` until the shared generic publisher
change lands.
