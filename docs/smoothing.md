# Smooth

`Smooth` is a host-automatable continuous control from `0%` to `100%`,
defaulting to `0%`. It is a dimensionless amount: `0%` is an exact identity
path, while `100%` selects a 100 ms one-pole time constant. Intermediate values
scale that time constant linearly in seconds.

Pump samples the editable curve and maps it through `Depth` and `Floor` first.
Smooth then filters that evaluated wet gain, before `Mix` blends it with the
dry signal and before `Output` applies its post-gain trim. The editable curve
and its stored preset representation are never modified.

The filter is zero-latency and uses the current sample rate for its coefficient.
Attack and release use the same time constant, so the response is continuous
and deterministic across sample rates and tempo changes. The state is reset to
unity when a processing session is reset; stopping transport does not edit the
curve or jump the filter state, and processing resumes from the held state.
Rapid automation changes the target and/or time constant at the exact sample
offset supplied by the host. Non-finite inputs fall back to safe finite values,
the output is clamped to `[0, 1]`, and vanishing tails are flushed to prevent
denormals. No realtime allocations, locks, or blocking operations are used.
