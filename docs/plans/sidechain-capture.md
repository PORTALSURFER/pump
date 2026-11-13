# Sidechain compression and capture

## Outcome

A user can route an internal or external detector, adjust compressor and
sidechain controls, capture one complete sync cycle, and edit the resulting
curve with live compression turned off. Pump's compressor is an independent
implementation, not an identical model of Ableton's Glue Compressor.

## Status and acceptance

CLAP and VST3 routing, controls, state, DSP, capture and the native editor are
implemented. Captured curves use a high-resolution playback path while legacy
curves retain their original path. Automated coverage includes signal and
capture behavior, phase and swing, an eight-bar cycle, state compatibility,
host automation, undo, native editor input and callback allocation checks.

Capture requires Sync, a host beat position, playback, zero Delay and detector
monitoring off. Boosted captures with band processing cannot be represented
faithfully by the current gain envelope and require reduced makeup. Soft
clipping is nonlinear and cannot be frozen into a gain envelope. The 2× path is
a lightweight implementation and has not been compared with Glue.

User acceptance in a DAW remains open: route an external sidechain, adjust the
controls, capture a cycle, compare playback and edit/undo the result, then save
and reopen the project. Windows runtime acceptance is also open.
