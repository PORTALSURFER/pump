# Swing timing

Pump's host-automatable **Swing** parameter is a percentage from **0% to
100%**, defaulting to **0%**. It applies to the alternating midpoint of every
selected cycle, including straight and triplet divisions, bar-length cycles,
free-running fallback timing, patterns, and sidechain-triggered cycles.

At 0%, Pump uses the legacy phase mapping exactly. Increasing Swing delays the
cycle midpoint without changing the cycle length. At 100%, the midpoint lands
at two-thirds of the cycle (a 2:1 triplet feel); the second half is compressed
to end at the same cycle boundary. Phase offset is applied after this warp.

Swing is persisted in project state and presets. State written by versions
before Swing was introduced restores 0%.
