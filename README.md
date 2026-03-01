# pump

`pump` is a node-based beat-synced gain-shaper plugin for sidechain-style ducking.

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

## Core idea

Edit a node-based spline curve that defines gain over one sync cycle.
The curve is sampled in real time and applied to stereo gain for controlled pumping.

## Main controls

- `Mix`: dry/wet blend of ducking intensity.
- `Phase Offset`: shifts where the curve starts in the sync cycle.
- `Output Gain`: level trim after ducking.
- `Division`: beat-synced cycle length from `1/16` to `2 Bars`.

## Notes

- v1 is envelope-only (no external sidechain input).
- The modulation phase follows host beat position when the host exposes a beat timeline.
- When beat timeline data is unavailable, phase falls back to transport-driven free running so curve modulation remains audible.
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
