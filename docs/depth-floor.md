# Depth and Floor

Pump treats each normalized curve sample `c` as an existing linear gain shape:
`c = 1` is unity and `c = 0` is silence. Depth is the amount of that curve's
attenuation that is applied:

```text
wet_gain = c ^ (Depth / 120 dB)
```

Depth is `0–120 dB`, defaults to `120 dB`, and `0 dB` is no effect. The default
therefore preserves the pre-Depth behavior exactly. A zero curve value remains
silence for any positive Depth unless Floor is finite.

Floor is the minimum wet curve gain. It supports `−∞` (the host plain-value
sentinel is `−60 dB`) and finite values above `−60 dB` through `0 dB`:

```text
wet_gain = max(wet_gain, 10 ^ (Floor / 20))
```

The processing order is:

```text
curve value → Depth mapping → Floor clamp → Mix blend → Output Gain
```

Mix blends the wet gain with unity (`(1 − Mix) + Mix × wet_gain`). Output Gain
then applies a post-mix trim. The gain-reduction meter reports the mix result
before Output Gain, and the curve editor's dB guides use the same Depth/Floor
mapping. Depth and Floor are smoothed on the audio thread, so automation remains
click-safe without allocation or locking.

State payloads and presets written before version 7 migrate to `Depth = 120 dB`
and `Floor = −∞`, preserving their previous full-depth sound. Both controls are
included in host automation, text conversion, preset/A-B snapshots, and state
persistence.
