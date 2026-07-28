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
`CompareAb`, `Copy`, `Settings`, `Favorite`, `ChevronLeft`, `ChevronRight`,
`ChevronUp`, `ChevronDown`, `Trigger`, `Pattern`, and `Power`. Icon identity is
retained SVG identity, never a single-character text approximation.

Pump action buttons map to that catalog with explicit automation labels:
`PresetPrevious` → `ChevronLeft` ("Previous preset"), `PresetNext` →
`ChevronRight` ("Next preset"), `PresetFavorite` → `Favorite` ("Favorite
preset"), `PresetAdd` → `Copy` ("Add preset"), `PresetSave` → `Pattern` ("Save
preset"), `Undo` → `History` ("Undo"), and `Redo` → `ChevronRight` ("Redo").
`Copy` is intentional because adding a preset clones the current state, while
`Pattern` intentionally represents saving the current state into the preset
bank.

## Metrics and states

The base unit is 4 px. The spacing scale is 4/8/12/16; surface padding is 12,
control gap 8, radius 8, border/divider 1, control/dropdown height 32, dropdown
minimum width 96, icon hit target 28, icon 16, knob 56, knob column 88, label line
16, meter panel width 48, meter track width 32, meter segment 4 with a 2 px gap,
and the existing Pump parameter deck remains 96 px high.

Resolver precedence is disabled, pressed, selected, hover, focus, then default;
automation is an additive state. Focus and automation remain visible through
border/ring and patterned or animated cues, selection also uses a knob tick or
outline, pressed uses a stronger fill, and disabled uses reduced contrast and
suppressed automation motion. These cues do not depend on color alone.

Reusable control primitives belong in Toybox/Radiant when they serve more than
Pump. Pump owns this palette aliasing, the editor composition, and Pump-specific
meter/curve treatment. Header/deck composition and new controls remain outside
this bounded visual-system slice.
