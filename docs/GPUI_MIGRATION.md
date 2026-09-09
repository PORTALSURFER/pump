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

## Verification coverage

The GPUI implementation retains 107 editor-model regressions. Local default
checks cover 367 library tests; the VST3 configuration covers 426. Native macOS
fixtures exercise typing, selection, clipboard, arrows, wheel/drag edits, explicit
RATE units, focused-button repeat handling, host updates, and close/reopen.
Screenshots cover supported sizes and control states, with pixel assertions for
expanded layout and idle numeric-label updates. Windows CI exercises the real
HWND through VST3 input and hide/reopen operations.

Toybox owns resize notification ordering and native repeat metadata. Pump also
tracks focused button presses through key-up to support host callbacks that do
not carry repeat information. Focus loss cancels numeric drafts; hiding ends
active UI gestures while retaining applied audio settings.

CI and release preflight results are attached to the migration pull request.
Fresh ad-hoc review bundles are audited separately; DAW and audible acceptance
remain manual. This migration does not publish a release.

## Curve and numeric input parity

Curve hover uses the rendered segment geometry, with highlighted nodes, an
insertion preview, blue move-range segments and a widened amber curve while
sliding. Paint previews and marquee selection remain visible during editing.
Right-button drags paint; plain left drags in empty space do not. Option-click
removes interior points while protecting endpoints and distinguishing a drag.

Admitted knob and curve gestures own native movement until matching release,
including outside-window movement. Capture loss cancels paint previews and
ends accepted knob edits. The timing menu occludes the curve beneath it.
Delay drafts contain digits only, allow temporary empty text, support Backspace,
and consume arrows without changing Sync. An empty submission restores the
prior value and exits editing.

Native tests cover these mouse and keyboard paths, and screenshot assertions
check the blue segment overlay rather than merely writing image files. Synthetic
hover events target the installed tracking-area owner, matching AppKit's tracking
route without changing the host window's mouse-move or first-responder settings.

The full delay control (progress strip and padding included) focuses the numeric
field and closes the timing menu. Arrow keys then step only delay. A plain drag
from empty plot space creates one node; a click alone does not. Hover insertion
uses a circular preview sampled on the curve, matching the click position.
Command–Shift and offset-strip drags follow the pointer visually; their stored
phase delta is inverted to retain existing DSP and preset semantics.

Paired diamond handles mark the viewport seam at both clipping boundaries.
They are sampled projections during offset changes. Dragging either copy
materializes or reuses one authored seam point and locks it vertically; ordinary
nodes reaching the boundary use the existing seam takeover/merge behavior.
