# pump

`pump` is a beat-synced gain-shaper plugin for sidechain-style ducking.

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

Draw a freehand spline-like curve that defines gain over one sync cycle.
The curve is sampled in real time and applied to stereo gain for controlled pumping.

## Main controls

- `Mix`: dry/wet blend of ducking intensity.
- `Depth`: how deep the gain reduction follows the curve.
- `Phase Offset`: shifts where the curve starts in the sync cycle.
- `Output Gain`: level trim after ducking.
- `Division`: beat-synced cycle length from `1/16` to `2 Bars`.

## Notes

- v1 is envelope-only (no external sidechain input).
- The modulation phase is host-beat-locked when transport data is available.
- Curve state is persisted in plugin state payloads.
