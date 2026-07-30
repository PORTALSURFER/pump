# pump

`pump` is a node-based beat-synced gain-shaper plugin.

## Build

Pump builds as a CLAP-first plugin by default. VST3 support is available behind
the `vst3` cargo feature and requires a local VST3 SDK checkout.

CLAP-only (default):

```bash
cargo build
cargo test
```

VST3 (requires SDK):

```bash
VST3_SDK_DIR=/mnt/e/lib/vst3sdk cargo build --features vst3
VST3_SDK_DIR=/mnt/e/lib/vst3sdk cargo test --features vst3
```

## Production releases

The same producer is used locally and by the manual `Pump release` Actions
workflow. On a clean macOS arm64 checkout, with `VST3_SDK_DIR` set to the pinned
Steinberg SDK and the Apple Developer ID/notarization credentials configured,
run:

```bash
bash scripts/release.sh --package-only --channel stable
```

This creates `dist/releases/pump-v<version>-<12-char HEAD>/` containing only the
two host-installable ZIP bundles, `pump-default-640x400.png`, `CHANGELOG.md`, and
`release-manifest.json` schema 2. Add `--publish` and set
`PORTALSURFER_RELEASE_TOKEN` in the environment to capability-check and publish
the immutable bundle. The token is never accepted as a command-line argument.

`--package-only` is still a production release: it signs, notarizes, staples, and
verifies notarization on both bundles. The Actions workflow cannot run until the production
environment has all Apple certificate/notary secrets, `RADIANT_REPO_TOKEN`, and
the PortalSurfer release token for publish runs.

Publishing is fail-closed to the exact `https://portalsurfer.org` origin. Immediately
before a publish, the producer re-audits the final ZIP bytes, bundle signatures and
team, stapling, notarization, arm64 architecture, exports, and manifest hashes; the
publish transport is injectable only in zero-network tests.

Production artifacts are macOS arm64, hardened-runtime Developer ID signed,
notarized, stapled, and checked with `codesign -vvvv -R=notarized --check-notarization`.
Universal2 and ad-hoc production
artifacts are intentionally unsupported until a separate host-compatibility
decision; the manifest records this production provenance explicitly.

## Core idea

Edit a node-based spline curve that defines gain over one sync cycle.
The curve is sampled in real time and applied to stereo gain for controlled pumping.

## Main controls

- `Mix`: dry/wet blend of ducking intensity.
- `Phase Offset`: shifts where the curve starts in the sync cycle.
- `Output Gain`: level trim after ducking.
- The curve always follows the host beat/transport timeline.

Depth and Floor control the curve's wet gain mapping. Depth ranges from `0` to
`120 dB` and defaults to `120 dB`; `0 dB` is no effect. Floor supports `−∞`
plus values above `−60` through `0 dB` finite values. Processing is curve → Depth → Floor → Mix →
Output Gain. See [docs/depth-floor.md](docs/depth-floor.md) for the exact
mapping and compatibility behavior.
- `Division`: beat-synced cycle length from `1/16` to `2 Bars`.

## Quick Shape Strip

- The row between the curve editor and knobs provides 8 globally persisted curve
  slots with micro previews of stored curves.
- The slot bank is shared by every Pump instance and survives host/plugin
  relaunches independently of project state, presets, and individual instances.
- Clicking a slot loads only that slot's curve into the editor and keeps the
  current sync division unchanged.
- Empty slots show a muted dash; clicking an empty slot is a non-destructive
  no-op.
- `Cmd`-clicking a slot stores the current editable curve into that slot without
  loading first.
- A loaded slot turns red when the editor curve diverges from the stored slot
  curve, and returns to normal when the curve matches again or a matching slot
  is loaded/stored.

## Timing behavior

Pump has two timing sources:

- **Sync** follows the host beat timeline and the selected Sync division (for
  example, 1/4 or 1/8). Host tempo and song position therefore determine the
  modulation phase when that timeline is available.
- **Free** runs continuously at the selected Free Rate in hertz. It is
  independent of host tempo and song position, so it remains continuous even
  when the host transport is stopped or does not provide beat-position data.

## Notes

- Sync modulation uses host beat position when the host exposes a beat timeline.
- Free modulation always uses its continuous rate; it does not fall back to or
  depend on host tempo or song position.
- Curve state is persisted in plugin state payloads.

## Transport Indicator

- The header indicator blinks by beat phase when host beat timeline data is available.
- If the host is playing but does not expose beat timeline data, the indicator stays lit as a fallback "transport active" signal.

## Troubleshooting

- If curve edits are visible but audio does not change:
  - Verify `Mix` is above `0%`.
  - Verify the plugin is not bypassed.
  - For VST3 hosts, ensure you are on a build that includes shared processor/controller state sync.
- If the transport indicator never lights:
  - Start host playback.
  - Confirm the plugin build includes transport telemetry propagation from the processor to GUI status.

Timing controls are documented in [docs/swing.md](docs/swing.md), including the
host-automatable Swing mapping.
