# Dual-band processing

Pump now provides a single minimum-phase Linkwitz–Riley crossover with 12 or 24 dB/oct slopes. The nominal crossover range is 20 Hz–20 kHz, bounded internally to 45% of the audio sample rate. LR2 uses an inverted high-band polarity; both slopes recombine with flat steady-state magnitude. This is zero-latency processing with the phase rotation inherent in minimum-phase filters. Slope changes crossfade continuously running filters, and cutoff and intensity controls smooth over 5 ms.

Low and high bands share Pump's authored envelope, timing, depth, floor and global mix. The LP and HP mix sliders form a compact stacked group with no thumb handles; each band also has its own solo. The filled tracks remain directly draggable. Sliders support dragging, wheel adjustment, arrow keys (Shift for larger steps), Home/End, and double-click reset to 100%. A drag is one automation gesture and one undo step; cancel and editor teardown close the host gesture. Both solos together monitor the full split signal. Independent band envelopes are not implemented. Dual mode replaces the legacy selective bandpass path when enabled. Existing HP/LP/Q/slope automation IDs remain intact for older projects; no ID has been repurposed.

When Dual is enabled, the main curve view includes a frequency-response overlay spanning the full graph: mint low-pass and blue high-pass curves with translucent fills, a crossover marker, and a logarithmic 20 Hz–20 kHz axis. It follows the crossover and 12/24 dB slope controls; the response preview uses a nominal 48 kHz sample rate. The envelope and its edit points paint above the overlay and retain their time axis and mouse interactions.


The compact native strip contains crossover slope and Smooth/Swing sliders. LP/HP mix bars and Solo buttons sit beside the Mix and Output knobs. Dual enable and crossover frequency remain host parameters. The Frame canvas is a visual review surface.

## Host and persistence contract

CLAP and VST3 expose one main stereo input and one main stereo output. Processing and parameter automation remain allocation-free.

| ID | Parameter | Plain range / default |
| --- | --- | --- |
| 24 | Dual Band | Off / On; Off |
| 25 | Crossover | 20–20000 Hz; 200 Hz |
| 26 | Crossover Slope | 0 = 12, 1 = 24 dB/oct; 24 |
| 27 | Low Mix | 0–1; 1 |
| 28 | High Mix | 0–1; 1 |
| 29 | Low Solo | Off / On; Off |
| 30 | High Solo | Off / On; Off |

Spectral ducking, its auxiliary input, and its host controls have been removed. IDs 31–35 are retired and will not be reused. Automation targeting those IDs is ignored. The remaining parameter IDs and their meanings are unchanged.

State version 22 and preset-store version 16 save seven dual-band values. State version 21 and preset-store version 15 still load: all active, working/stored A/B and preset dual-band values are retained, and the five removed spectral values are discarded. Invalid and truncated legacy records are rejected before mutating active state. Older formats continue to seed dual-band defaults.

## Validation

Tests cover LR recombination, per-band ducking and solos, extreme parameter automation, allocation-free processing, stable host IDs, sample offsets, undo, current/legacy state and preset migration, and malformed records. Native rendering and DAW verification still require macOS or Windows and a real host.
