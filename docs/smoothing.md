# Smooth

`Smooth` is a host-automatable continuous control from `0%` to `100%`,
defaulting to `0%`. It is a dimensionless amount: `0%` is an exact identity
path. The compatibility boundary is `75%`: from `0%` through `75%`, the
one-pole time constant is exactly the legacy mapping `tau = amount * 100 ms`,
including sub-sample values. From `75%` to `100%`, a continuous smoothstep tail
extends the time constant from `75 ms` to `250 ms`; therefore `100%` selects a
250 ms one-pole time constant.

Pump samples the editable curve and maps it through `Depth` and `Floor` first.
Smooth then filters that evaluated wet gain, before `Mix` blends it with the
dry signal and before `Output` applies its post-gain trim. The editable curve
and its stored preset representation are never modified.

The filter is zero-latency and uses the current sample rate for its coefficient.
Attack and release use the same time constant, so the response is continuous
and deterministic across sample rates and tempo changes. The state is reset to
unity when a processing session is reset; stopping transport does not edit the
curve or jump the filter state, and processing resumes from the held state. A
host seek or other timeline discontinuity likewise retains the filter state and
ramps toward the newly evaluated target.
Rapid automation changes the target and/or time constant at the exact sample
offset supplied by the host. Non-finite inputs fall back to safe finite values,
the output is clamped to `[0, 1]`, and vanishing tails are flushed to prevent
denormals. No realtime allocations, locks, or blocking operations are used.
