# Dual-band and spectral ducking

Pump now provides a single minimum-phase Linkwitz–Riley crossover with 12 or 24 dB/oct slopes. The nominal crossover range is 20 Hz–20 kHz, bounded internally to 45% of the audio sample rate. LR2 uses an inverted high-band polarity; both slopes recombine with flat steady-state magnitude. This is zero-latency processing with the phase rotation inherent in minimum-phase filters. Slope changes crossfade continuously running filters, and cutoff and intensity controls smooth over 5 ms.

Low and high bands share Pump's authored envelope, timing, depth, floor and global mix. The LP and HP mix sliders form a compact stacked group with no thumb handles; each band also has its own solo. The filled tracks remain directly draggable. Sliders support dragging, wheel adjustment, arrow keys (Shift for larger steps), Home/End, and double-click reset to 100%. A drag is one automation gesture and one undo step; cancel and editor teardown close the host gesture. Both solos together monitor the full split signal. Independent band envelopes are not implemented. Dual mode replaces the legacy selective bandpass path when enabled. Existing HP/LP/Q/slope automation IDs remain intact for older projects; no ID has been repurposed.

When Dual is enabled, the main curve view includes a frequency-response overlay spanning the full graph: coral low-pass and blue high-pass curves with translucent fills, a crossover marker, and a logarithmic 20 Hz–20 kHz axis. It follows the crossover and 12/24 dB slope controls; the response preview uses a nominal 48 kHz sample rate. The envelope and its edit points paint above the overlay and retain their time axis and mouse interactions.

Spectral ducking detects the optional external stereo sidechain with 24 log-spaced bandpass filters and applies inverse, attenuation-only peaking EQ to the main signal. Detectors are stereo linked by the stronger channel; audio filters maintain independent stereo histories. Spectral attack and release control detector response. Relative spectral weighting suppresses detector skirts so a narrow sidechain tone does not cause broad ducking. Absolute sidechain strength controls attenuation and tends to zero at silence. An unconnected sidechain is treated as silence; after release settles, the spectral stage is unity.

The spectral stage precedes envelope processing. In **Volume** mode, spectral ducking follows the sidechain and the envelope ducks the resulting signal. In **Depth** mode, the envelope scales spectral depth and volume ducking fades out. With both Dual and Depth enabled, the spectral depth uses the shared envelope while per-band volume ducking fades out. Output trim and host bypass remain last; fully settled host bypass returns original input exactly. Mode and enable transitions smooth over 5 ms.

The native editor exposes compact effect value controls. Scroll vertically over a value to adjust a continuous control; click stepped controls to toggle. These edits use the host automation sink and undo history. Dual-band controls occupy one short row: crossover frequency, slope, stacked LP/HP mixes and solos, and a compact Spectral switch. There is no separate section header or tab row. The 640 × 400 editor preserves more room for the graph; expanded spectral controls remain available through the switch. The original dark gray/red-orange envelope theme is retained. The live 24-band meter shows sidechain activity, with coral indicating active spectral attenuation. The Frame study is a separate visual review surface, and its spectrum is illustrative.

## Host and persistence contract

CLAP and VST3 advertise one main stereo input, one optional stereo sidechain input and one main stereo output. Audio buffers and parameter schedules are allocated at activation, not in processing. CLAP copies the sidechain before borrowing main outputs; VST3 reads sidechain samples through validated optional host pointers before writing each main sample. One-input host blocks remain supported.

New stable parameter IDs:

| ID | Parameter | Plain range / default |
| --- | --- | --- |
| 24 | Dual Band | Off / On; Off |
| 25 | Crossover | 20–20000 Hz; 200 Hz |
| 26 | Crossover Slope | 0 = 12, 1 = 24 dB/oct; 24 |
| 27 | Low Mix | 0–1; 1 |
| 28 | High Mix | 0–1; 1 |
| 29 | Low Solo | Off / On; Off |
| 30 | High Solo | Off / On; Off |
| 31 | Spectral Duck | Off / On; Off |
| 32 | Spectral Depth | 0–36 dB; 12 dB |
| 33 | Spectral Attack | 0.1–500 ms; 10 ms |
| 34 | Spectral Release | 5–2000 ms; 150 ms |
| 35 | Spectral Envelope | 0 = Volume, 1 = Depth; Volume |

Project state version 21 and preset-store version 15 append these settings to active state, working A/B sounds, stored A/B references, and presets. Older formats decode with both new effects disabled. Invalid or truncated current payloads fail before changing active state.

## Validation

DSP tests cover LR recombination at multiple rates, per-band ducking and solos, silent-sidechain unity, stereo independence, targeted spectral attenuation, release, extreme parameter automation, and allocation-free processing. Parameter tests cover host text and normalized round trips, sample offsets, A/B and stored references, preset disk persistence, older state migration, malformed payloads, and undo.

Native window rendering and DAW routing still require macOS or Windows and a real host. A Linux type check of the GPUI composition checks its API use but does not replace those platform checks. The spectral algorithm is this project's own 24-band dynamic EQ; it does not claim to reproduce Sidekick's proprietary DSP.
