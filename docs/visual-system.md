# Pump visual system

Pump uses a fixed dark-coral visual system over the reusable Radiant `ThemeTokens`
surface. `src/gui/visual_system.rs` is the Pump-local contract; it intentionally
does not add Pump-specific fields to Radiant.

## Palette

All values are RGBA and are the canonical Radiant dark palette values used by
`pump_theme()` at every supported viewport tier.

| Semantic token | Radiant field | Value |
| --- | --- | --- |
| canvas | `clear_color` / `bg_primary` | `#1B1E1EFF` |
| header | `surface_raised` | `#1B1E1EFF` |
| panel | `surface_base` / `surface_raised` | `#1B1E1EFF` |
| editor surface | `bg_secondary` | `#1B1E1EFF` |
| overlay | `surface_overlay` | `#2A2D2DFF` |
| border | `border` | `#3A3D3DFF` |
| emphasized border | `border_emphasis` | `#404342FF` |
| primary text | `text_primary` | `#D8D7D3FF` |
| muted text | `text_muted` | `#999B9AFF` |
| strong grid | `grid_strong` | `#363939FF` |
| soft grid | `grid_soft` | `#282B2BFF` |
| coral primary | `accent_mint` / `highlight_orange` | `#E95843FF` |
| coral secondary | `accent_copper` | `#F16C56FF` |
| warning | `accent_warning` | `#D9975FFF` |
| error | `accent_danger` | `#EF4C3DFF` |
| disabled fill | `control_disabled_fill` | `#242829FF` |

Meter aliases are `track = grid_soft`, `nominal = coral secondary`,
`hot = error`, `border = border`, and `text = muted text`.

## Typography and icons

Roles are brand 22/28, body 14/18, value 12/16, control label 10/16, and meta
9/14 (font size / line height in logical pixels). Text uses the license-safe
Ioskeley Mono face first, Sometype Mono for glyph-aware fallback, then the
native/system fallback. The current offscreen capture path cannot prove native
font selection; host/runtime glyph diagnostics remain the authoritative evidence
for that final fallback.

Shared controls use the retained Lucide v0.468 ISC catalog in Radiant: `History`,
`CompareAb`, `Settings`, `Favorite`, `ChevronLeft`, `ChevronRight`,
`ChevronUp`, `ChevronDown`, `Trigger`, `Pattern`, and `Power`. Icon identity is
retained SVG identity, never a single-character text approximation.

Pump header actions map to that catalog with explicit semantic labels: `Undo`
→ `ChevronLeft` ("Undo"), `Redo` → `ChevronRight` ("Redo"), and the A/B group
uses `Sound A` and `Sound B` buttons around a directional `ChevronLeft` or
`ChevronRight` `Switch sound` control. The switch chevron selects the other
side; Option-click copies the active sound to the inactive side. Cmd-clicking
the active `Sound A` or `Sound B` button stores that side's working state.
Sound-side value text is `Stored` or `Modified`.

## Metrics and states

The compact editor uses a uniform 0.85 visual scale. The spacing scale is
3.4/6.8/10.2/13.6; surface padding is 10.2, control gap 6.8, radius 6.8,
border/divider 1, control/dropdown height 27.2, dropdown minimum width 81.6,
icon hit target 28 (24 px floor), icon 13.6, knob 47.6, knob column 74.8,
label line 13.6, meter panel width 40.8, meter track width 27.2, meter segment
3.4 with a 1.7 px gap, and the Pump parameter deck is 81.6 px high. The eight
curve slots remain equal-width and fluid across the inner width; the compact
slot height is 51 px with a 3.4 px gap.

Typography scales with the composition: brand 18.7/23.8, body 11.9/15.3,
value 10.2/13.6, control label 8.5/13.6, and metadata 8/11.9 (font size / line
height in logical pixels). Metadata keeps an 8 px legibility floor.

Resolver precedence is disabled, pressed, selected, hover, focus, then default;
automation is an additive state. Focus and automation remain visible through
border/ring and patterned or animated cues, selection also uses a knob tick or
outline, pressed uses a stronger fill, and disabled uses reduced contrast and
suppressed automation motion. These cues do not depend on color alone.

Reusable control primitives belong in Toybox/Radiant when they serve more than
Pump. Pump owns this palette aliasing, the editor composition, and Pump-specific
meter/curve treatment. Header/deck composition and new controls remain outside
this bounded visual-system slice.
