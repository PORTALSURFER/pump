# Pump to-do list

Items below are pending implementation and verification.

- [ ] **Drag the crossover in the main curve view.** Hovering the crossover line gives clear feedback and a horizontal-drag cursor. Dragging left/right changes the crossover frequency on the logarithmic axis, with host automation gestures and undo support. Keep envelope editing usable.
- [ ] **Default LP and HP to 0%.** Fresh instances and reset controls start both band mixes at zero, with no band filtering applied. Preserve explicitly saved settings and existing projects.
- [ ] **Fade the band overlays with their mix percentages.** LP and HP response lines and fills fade independently as their mix changes. A band at 0% has no visible response curve. When both bands are at 0%, also hide the crossover marker and its interaction target. Show it again when a band becomes visible.
- [ ] **Expose waveform Sync/Live as a host parameter.** Make the mode automatable and mappable through Ableton Live's Configure panel. Save and restore it in plugin state, presets and the user's default preset. Preserve existing automation IDs.
- [ ] **Highlight hovered sync-menu options.** Give the hovered option a clear, restrained highlight so the item about to be clicked is obvious.
- [ ] **Support keyboard navigation in the sync dropdown.** While open, Up/Down move the highlighted choice without closing the menu or committing a parameter change. Enter commits that choice and closes the menu. Escape cancels and returns focus appropriately. Keep the current selection and keyboard highlight distinguishable.
- [ ] **Make the delay input easier to identify.** Retain the progress indicator above it, draw a compact rectangle around the beat input, and place a small DELAY label below it. Clicking the rectangle starts numeric entry. Review header alignment and spacing at minimum and maximum editor sizes.
- [ ] **Restore the edited-preset indicator.** Clicking a preset loads and selects it. Subsequent curve edits visibly mark that preset as modified with a distinct, restrained color. Saving clears the modified state; returning to the saved curve also clears it. Keep selected, modified and empty preset states distinguishable.

Verify host-facing changes in the native macOS VST3 editor and Ableton Live, including automation, state/preset persistence, keyboard focus and undo.
