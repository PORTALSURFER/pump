# pump

`pump` is a beat-synced gain-shaper plugin for sidechain-style ducking.

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
