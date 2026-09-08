# Pump embedded GPUI migration

## Scope

Replace the Radiant editor with Toybox's embedded GPUI backend while preserving
Pump's appearance, controls and audio behavior. The host retains its event loop;
Toybox owns the native child window, renderer, input and CLAP/VST3 lifecycle.
Pump owns its editor model, composition, curve gestures and parameter edits.

The source baseline is `aeea75d361b58fe6929db833570b40d7cd98dbe4`. Its 472 tests
and seven screenshot tests pass. Twenty-one reference captures are preserved in
`/Users/portalsurfer/dev/audiodev/dist/pump-gpui-reference-aeea75d`.

## Invariants

- Preserve the 640×400 default/minimum and 1280×800 maximum with an 8:5 aspect
  ratio, including fractional DPI. Keep the dark/coral palette, Ioskeley Mono
  typography, real icon shapes, eight slots, curve/waveform layers and meters.
- Preserve parameter IDs, ranges, saved-state formats, A/B sounds, undo/redo,
  global curve slots and host automation gesture admission/order.
- Reuse the pure curve geometry, paint reconstruction, DSP, transport and
  waveform algorithms. Replace the GUI framework boundary without changing
  audio-thread ownership or adding UI work to audio processing.
- Retain curve seam ownership, modifier precedence, node/segment/offset hit
  zones, snapping, drag cancellation and all existing semantic regressions.
- Native numeric entry supports selection, clipboard, Enter/Escape, arrows,
  modified arrows and composition. Focused buttons support Space/Enter while
  unfocused transport keys remain available to the host.
- Hiding Pump must preserve audio parameters. End or cancel active UI gestures
  safely on teardown; do not import GainSnap-specific Match behavior.
- Use exact remote Toybox/GPUI revisions. The final active graph contains no
  Radiant and no local development patches.

## Acceptance

Compare live GPUI captures with the baseline at supported sizes and key states.
Run strict checks and semantic regressions, native editor input/lifecycle tests
on macOS and Windows, and both independently linked plugin load orders on macOS.
Audit fresh CLAP/VST3 review artifacts and the existing Windows release sidecar
contract. Manual DAW keyboard and audible acceptance remains user-owned.

## Status

Baseline captured; migration in progress. No release is published by this work.
