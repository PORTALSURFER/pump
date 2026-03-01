# Changelog

All notable changes to this project are documented in this file.

## [unreleased]


### Bug Fixes

- Migrate pump GUI open path to open_parented_with (9eded5e)

- Align pump with latest toybox and restore screenshot helpers (af020b8)

- Fix pump gui metrics scaling for resized window text and curve layout (39101d0)

- Make pump ui sizing uniform-fit with fixed design metrics (706cfa7)

- (pump) Remove unused section split helpers in gui (7f9ae88)

- (vst3) Align bus flag assignment by platform (1ef9337)

- (gui) Tighten pump knob bounds in controls row (85d8c7a)

- (gui) Center-align pump knob labels (dd9ea21)

- (pump) Bump toybox revision for knob alignment (b23979e)

- (pump) Align dependency to updated toybox text centering (559ba40)

- (pump) Bump toybox for vector glyph-centered knob labels (2884197)

- (pump) Bump toybox for bundled Sometype Mono (845db5d)

- (gui) Pack control knobs tightly and left-aligned (a2e5151)

- (gui) Remove spline tip helper text (ce27816)

- (pump) Bump toybox for dropdown overlay visibility (12db5b5)

- (gui) Scale up knobs and reduce knob label text (6670e38)

- (gui) Pack pump knobs tightly and add tiling regression (63a3531)

- (layout) Anchor controls children at panel origin (d9634eb)

- (pump) Default creator metadata to portalsurfer (4b63446)

- (gui) Align dropdown panel to top-right (6ca25ee)

- (build) Export windows vst3 bundle root with -win suffix (fe7be31)

- (build) Emit windows artifacts as direct -win files (641eb40)

- (metadata) Set creator name to PORTALSURFER (099986b)

- (vst3) Normalize process flag types to u32 (f0e4d72)


### CI

- (changelog) Auto-update changelog on push (b5a9434)


### Chores

- Follow toybox main branch (86b07cf)

- Refresh toybox git dependency to latest branch commit (8dc2b0f)

- Bump toybox git-lock to latest patchbay uniform-fit rendering (fc977dd)

- Update toybox lockfile to latest uniform-fit fix (924a5db)

- Refresh toybox lockfile for resize mapping fix (fa5cce1)

- (pump) Update toybox lockfile to new uniform-fit remap fix (cad1454)

- (pump) Update toybox lockfile after branch bump (6d98af2)

- Sync with audiodev harness (651855b)

- Bump toybox revision for screenshot tests (de43d78)

- (deps) Bump toybox for tight knob tiling (d9f646f)

- (deps) Bump toybox for knob centering fixes (50906b4)

- (deps) Bump toybox for knob block-border visibility (25a3e07)

- (lockfile) Refresh cargo lock after toybox bump (6cbe7df)

- (deps) Bump toybox for tighter knob bounds (c529b56)

- (deps) Bump toybox for native dropdown popups (4ec4155)

- (deps) Bump toybox for windows popup build fix (b0cadc4)

- (deps) Unpin toybox git dependency (8ef11b2)

- Bump toybox for NodeId invalidation API (923bbd3)

- Bump toybox for runtime engine adoption (08cf564)

- Bump toybox for targeted invalidation (0c86639)

- Bump toybox for layout-only targeted invalidation (154ecde)

- Bump toybox for hybrid invalidation scopes (df220b2)

- Bump toybox for overflow summary diagnostics (4ab6f5c)

- Bump toybox for layout property sweep tests (c471f48)

- Bump toybox for golden layout rect tests (cc60e60)

- Bump toybox for depth-guard stress milestone (49fc204)

- Bump toybox for 10k scroll stress coverage (3095ccd)

- Bump toybox for container golden layout coverage (b3a7edf)

- Bump toybox for curve editor AA quality fix (db2e074)

- (deps) Bump toybox to latest commit (dc5de52)

- (deps) Bump toybox and add dropdown popup regressions (6e137a3)

- (release) Bump pump to v0.2.0 (76e1045)

- (deps) Bump toybox for editable textbox fix (a8451c7)

- (deps) Bump toybox to latest main (53c68f4)


### Documentation

- Add core framework boundary guidance to AGENTS (01e9211)

- (handoff) Align wake-up portal and plan artifacts (0c9314f)

- (changelog) Update changelog [skip ci] (e65f45a)

- (changelog) Update changelog [skip ci] (4a4e726)

- (changelog) Update changelog [skip ci] (72def75)

- (changelog) Update changelog [skip ci] (435e1d0)

- (changelog) Update changelog [skip ci] (12a14e9)

- (changelog) Update changelog [skip ci] (50a51c3)

- (changelog) Update changelog [skip ci] (71184d4)

- (changelog) Update changelog [skip ci] (5adfbc3)

- (changelog) Update changelog [skip ci] (c4d7110)

- (changelog) Update changelog [skip ci] (af57659)

- (changelog) Update changelog [skip ci] (5b9b7c9)

- (changelog) Update changelog [skip ci] (a23d7ce)

- (changelog) Update changelog [skip ci] (2a7fc03)

- (changelog) Update changelog [skip ci] (4ba6fb8)

- (changelog) Update changelog [skip ci] (f65b685)

- (changelog) Update changelog [skip ci] (6ac65d5)

- (changelog) Update changelog [skip ci] (d0ca5fb)

- (changelog) Update changelog [skip ci] (ab63caf)


### Features

- (vst3) Consume keys via edit-mode and registered shortcuts (2ce4027)

- (presets) Make init preset fully writable (253dec6)


### Other

- Add pump plugin with freehand beat-synced gain shaping (c4e1ec4)

- Migrate VST3 editor view to toybox hosted GUI helper (70defc1)

- Bump toybox dependency to hosted VST3 view revision (5501cc5)

- Use toybox git dependency without local patch override (453bbe6)

- Bump toybox revision for knob arc orientation fix (fdb40c9)

- Replace freehand curve input with node-based spline editor (67ce43f)

- Bump toybox revision for knob indicator alignment fix (557255f)

- Simplify curve editor defaults and smooth node interactions (b418e1e)

- Remove unused sidechain helper to satisfy deny-warnings build (fc76e92)

- Make curve node right-click delete reliable and enhance hover states (21b2750)

- Bump toybox revision for inverted knob drag direction (2f0095b)

- Use region-local pointer events for accurate curve node hit detection (cc43f55)

- Bump toybox to VST3_SDK_DIR-based revision (3bee33f)

- Unify curve hover hit-testing with region local pointers (2bc9078)

- Refactor curve editor to segment drag, alt curve mode, and push-through (7e71913)

- Use vertical drag for Alt curve adjustment (4d3d4d8)

- Split direct-vs-near curve hits for add mode and 2D segment drag (d1b0b9e)

- Use raw drag coordinates and invert curve-adjust direction (861a16f)

- Disable add-node mode while Alt is held (b3ad59e)

- Widen curve hit ranges for move and add modes (6cfba3b)

- Switch node deletion to double-click (69245b1)

- Bump toybox for corrected knob value orientation (d2794bd)

- Bump toybox to vello gui revision (993b0bf)

- Bump toybox to crash-hardened vello gui commit (186e5e6)

- Scale GUI to 60% and bump toybox vello renderer (c990809)

- Bump toybox layout sizing fixes (8836ffd)

- Bump toybox for layout floor and knob-size fixes (cde5c63)

- Open gui using measured layout bounds (2620ede)

- Advertise measured gui size to hosts (55c0527)

- Update toybox for knob label styling refinements (427c3ff)

- Update toybox for container layout debug borders (68fca50)

- Section ui into declarative panels and tighten knob grid (8f2156a)

- Adopt shared toybox main theme for Pump GUI (44fcf40)

- Fix pump section sizing and full-bleed curve layout (b2e8122)

- Enable host-resizable Pump UI with window-sized layout (9427b21)

- Bump toybox to hovered inner-container debug borders (4472bad)

- Enforce uniform GUI resizing for Pump (e18300e)

- Refactor Pump GUI into a strict three-section root layout (cfb94da)

- Measure Pump open size from baseline viewport (eac1f92)

- Remove inter-section outlines in Pump column layout (6962de0)

- Remove panel wrappers between Pump vertical sections (3a05e54)

- Fix declarative Node API misuse for panel header height (db1c65a)

- Bump toybox to include root-wrapper debug border filter (dc0c71a)

- Remove extra controls panel wrappers in Pump layout (bf0a0a2)

- Bump toybox for in-bounds container debug border rendering (12ad677)

- Remove unused PumpTheme panel background field (064605c)

- Make knobs grid row fill controls section height (7a0ce9a)

- Clamp persisted GUI size to minimum in get_size (4883576)

- Bump toybox with render-time container bounds clamping (a14e5ca)

- Scale Pump declarative control tokens with UI scale (395dfca)

- Increase Pump knob size while keeping compact text scale (bf9c84b)

- Increase Pump knob diameter with standard label scale (9159d90)

- Use toybox default knob diameter in Pump (a456b09)

- Bump toybox to fix initial client-size sync in Patchbay (1c426b3)

- Bump toybox for attach-time VST3 GUI min-size enforcement (bdc4a9d)

- Scale Pump sections and controls proportionally with window size (b83cf8a)

- Move Pump GUI scaling ownership to Patchbay root mode (a3a0e54)

- Bump toybox for core root scaling text/widget unification (9fa6381)

- Bump toybox for stable uniform-fit resize rendering (b7ca7b9)

- Stop resize feedback loop during host corner resizing (b448f34)

- Bump toybox for VST3 resize hysteresis (f1a71d9)

- Bump toybox for request-based uniform resize constraints (fbae133)

- Bump toybox for bounded VST3 resize constraints (56b4104)

- Bump toybox for stable VST3 onSize handling (3c5d7cd)

- Bump toybox for uniform-fit renderer and resize growth fix (15b8688)

- Bump toybox for uniform onSize resize constraints (82d4745)

- Bump toybox to include resize/layout stability fixes (12bd7ad)

- Bump toybox for viewport-based root resize fix (be226a3)

- Bump toybox for uniform resize enforcement fix (ba7e8ac)

- Bump toybox for reliable embedded resize sync (344ffc1)

- Bump toybox for onSize child-resize propagation (7a80cdd)

- Bump toybox for resize jump regression fix (e50b0f8)

- Bump toybox for stable resize-axis behavior (437f8d3)

- Bump toybox for resize flicker stabilization (8517214)

- Bump toybox for deterministic VST3 resize constraints (8b3aa01)

- Bump toybox for single-path uniform resize behavior (9d2b73c)

- Bump toybox for uniform-fit resize stability (1a83453)

- Bump toybox for canonical resize layout sizing (07820b1)

- Remove aspect-gated resize adoption dependency (7be8186)

- Remove plugin-local GUI size clamping in Pump (dcf7c6f)

- Bump toybox to root-transform scaling update (2d9505f)

- Bump toybox for full-surface root scaling (e66fa73)

- Apply host set_size requests to patchbay window (b20017a)

- Delegate host resize policy to toybox helpers (1edaf2c)

- Adopt toybox default resize callbacks (adb2bca)

- Delegate clap callback plumbing to toybox (bc8d10b)

- Pull toybox resize fix (27eac9d)

- Pull toybox vst3 resize forwarding (b34cde7)

- Pull toybox deepest debug border selection (6ce1965)

- Reduce pump baseline window size to 60 percent (b3aa1de)

- Keep design canvas full-size, open at 60 percent (6f3d51e)

- Switch pump to 1:1 small ratio-based layout (993a6a0)

- Match pump section blocks to figma layout (abc029e)

- Remove dead spline height constant (191b7e6)

- Keep section layout and drop figma color overrides (fbc43bb)

- Size pump sections from host window bounds (4f98077)

- Migrate pump layout to weighted toybox sections (66bebfa)

- Pull strict toybox section split behavior (2dace3c)

- Align Pump section sizing with Toybox weighted splits (b8eb3c7)

- Bump toybox for section-clamped widget rendering (ab719e2)

- Update pump to toybox canonical layout revision (fba876e)

- Bump pump to panel-origin layout fix in toybox (a3adeb9)

- Refactor pump GUI build pipeline into focused helpers (0b498d9)

- Use latest toybox for pump and disable hosted ratio locking (60cb7e5)

- Refresh patchbay resize logic via updated toybox and adjust screenshot resize cases (676eaa1)

- Fix Pump UI resize flow and include 300% screenshot case (7cb57bb)

- Update toybox dependency to latest VST3 resize fix (4b35a69)

- Add 300% window size coverage to pump resize tests (f826b8d)

- Enforce aspect-ratio-preserving VST3 resizing for Pump (ca01e48)

- Update toybox dependency lock for VST3 resize fix (b02a0e6)

- Scale pump UI layout and interaction hit targets with host size (5ad0e2a)

- Consume shared toybox framework helpers in pump (9086e20)

- Fix pump clippy warnings and small GUI refactors (90b492c)

- Fix VST3 bus flag assignment types for cross-platform build (0bfab3e)

- Add pump UI tiny-window build_ui layout safety regression (c23e7e1)

- Cast VST3 bus flags to u32 in getBusInfo (e47a782)

- (pump) Bump toybox for ScreenToClient fix and layout assertions (a28fdbd)

- (pump) Bump toybox to ScreenToClient BOOL fix commit (92b92cf)

- Derive pump gui layout from runtime slot sizing (37a13b3)

- Rename pump slot layout internals and enforce slot policy checks (8230086)

- Adopt declarative accessors and bump toybox slot guard rev (cd511e3)

- Derive pump layout strictly from host size at runtime (a16c5f7)

- Adopt host-derived container layout API and add tree invariant CI guard (1400646)

- Roll out strict slot-layout policy checks (410c3bd)

- Adopt strict overflow policy API and diagnostics surface (3efc9a7)

- Adopt switch-layout responsive header breakpoint (5d66d17)

- Handle new single-slot containers in strict tree traversal (d75820f)

- Align strict tree checks with aspect-box container (1b386a9)

- Bump toybox for horizontal scroll offsets (bcd6827)

- Bump toybox for structural gap diagnostics (36c4a0c)

- Bump toybox for keyed node id stability (61abac4)

- Bump toybox for constraint normalization diagnostics (2f63bcd)

- Bump toybox for hard layout-bound failures (679f60f)

- Bump toybox for slot-bound validation (5efab3a)

- Bump toybox after normalization cleanup (5af19b9)

- Bump toybox for compile-fail guard (809067f)

- Bump toybox for builder-only layout boxes (5f266ba)

- Lock pump layout to design-space uniform scaling (980ea23)

- Lock pump aspect ratio and sized-root construction (9b04a58)

- Migrate pump ui to textbox-only text api (c5ab16e)

- Restore pump control captions via textbox overlays (e634212)

- Add transport-synced curve playhead dot (4729f42)

- Simplify pump dropdown stack text labels (fe32258)

- Make segment bend drag direction deterministic (f821107)

- Fix pump control labels and playhead visibility (36a6b82)

- Update GUI transport telemetry every process callback (bb2e9c2)

- Add beat-synced transport indicator to pump (c299c69)

- Extrapolate transport phase for robust curve playback dot (bef3453)

- Fix pump modulation when host transport timeline is absent (e9b1a0a)

- Simplify pump header to 80/20 indicator row (834ea13)

- Fix pump transport sync and shared VST3 runtime state (124ee29)

- Left-pack knob controls and remove dropdown text overlay (0237eec)

- Render pump reduction meter top-down with test coverage (719bd15)

- Prevent dropdown changes from triggering curve reset (175244d)

- Add preset bank header workflow and rename editing (03dc028)

- Add overwrite-by-name preset save and lock Init (9d68291)

- Bump toybox pin to include Win32 UiAction fix (bd0eac4)

- Bump toybox pin for in-window dropdown overlays (37f9e05)

- Bump toybox pin for Win32 PCWSTR hotfix (f08e0ef)

- Fix preset title dirty styling and rename hit region (45ee812)

- Switch preset rename to explicit header button (5f36868)

- Bump toybox for focused keyboard capture fix (09b156b)

- Bump toybox to Windows dialog-key compile fix (7ce1352)

- Route VST3 key events into preset rename textbox (adaea91)

- Pin toybox to PostMessageW windows fix (fe077a7)

- Double pump knob dial baseline size (d6aabae)

- Update toybox pin for dropdown text clipping fix (f797b4b)

- Style preset dropdown hover and open states like division selector (96d2d6b)

- Center knob label and value text horizontally (a824faa)

- Update toybox pin for textbox inset rendering (56e17ad)

- Keep empty preset rename draft while editing (148ba7c)

- Update toybox for textbox selection and cursor navigation (e9b871b)

- Pack preset action buttons and center header glyphs (d68c45f)

- Update textbox rendering scale and clipping behavior (08788cb)

- Center knob textbox labels and cap to eight chars (dfa9219)

- Bump toybox for knob arc style update (505dc0d)

- Bump toybox for tighter knob arc spacing (144f206)

- Center knob label text vertically in control rows (cad11d0)

- Increase knob slot share for larger dials (7e3ec30)

- Enlarge knob dials without changing slot bounds (d097b29)

- Bump toybox for slightly larger knob dials (33729e5)

- Extract pump test modules from src/gui.rs (3b468ec)

- Refine GUI layout and automation gesture handling (7a6eff3)

- Adjust knob column slots to weighted 15/70/15 split (7bba013)

- Adopt toybox widget sizing updates (8364cc4)

- Fix Windows screenshot test to capture child GUI window (4670496)

- Fix Windows API call signatures in screenshot test (017df8c)

- Stabilize screenshot sizing wait and output naming (1d4ba7d)

- Use Toybox renderer frame capture for Windows screenshots (1c53a34)

- Update Pump to synchronous Toybox frame-capture fix (b591f83)

- Pin Pump to SendMessageW frame-capture fix (9228d0d)

- Update toybox pin for unlabeled knob dial expansion (f4f53aa)

- Pin toybox after 25 percent unlabeled dial trim (afe6378)

- Rename reset label and adopt borderless knob rendering (b8a6a97)

- Make header controls fill available header height (3041f9b)

- Center reset label within full button bounds (23882b6)

- Use widget-owned button labels in pump UI (73a07a6)

- Pin toybox with darker unfilled knob arc (5d8de4a)

- Reduce knob dial size slightly (c4fc82c)

- Top-align knob value text (c441d81)

- Bump toybox to 5bd13f1 for global slot debug overlays (77827a5)

- Bump toybox to b5b12f9 for exact debug border bounds (c032bfd)

- Remove header gap between preset dropdown and action buttons (72f7148)

- Remove header preset-action gap path (67a2ea5)

- Remove declarative gap usage from Pump GUI (9b1e4a3)

- Collapse dropdown/reset seam to a single divider line (40a5c30)

- Revert plugin-specific dropdown seam override (09746d7)

- Remove declarative theme color overrides (95f2056)

- Update toybox pin to grayscale theme revision (9ff7ea7)

- Restore dirty preset highlight using theme literal color (3cd2456)

- Stabilize VST3 preset mapping and shared state claims (7124cb8)

- Replace warning row with blinking preset highlight (705a3a8)

- Update toybox dependency for dropdown popup clamp (79b498b)

- Update toybox dependency for row-quantized dropdown viewport (4737830)

- Enable knob default reset values and update toybox (5e94418)

- Adopt toybox automation API cleanup and update toybox pin (fcaf624)

- Update VST3 key mapping for editable textbox input (db9226d)

- Preset dropdown: highlight only selected item when dirty (8a7bd5c)

- Bump toybox for dropdown overlay text scroll fix (c165d51)

- Bump toybox for resize click guard (7b2b638)

- Bump toybox revision for cross-target VST3 keycode fix (4eb9b0e)

- Bump toybox rev for out-of-window hover fix (3f26e88)

- Make push-through node deletion reversible within active drag (1584800)

- Migrate Pump curve UI to toybox curve editor widget (027937d)

- Fix curve point double-click deletion regression (b1e0fd2)

- Increase curve editor vertical safe margin (f758160)

- Add U/u hotkeys for redo and undo (917b2cc)

- Persist presets across instances and coalesce curve drag undo (0105ab7)


### Refactoring

- (pump) Consume toybox shared transport/atomic primitives (cfae73f)

- (gui) Migrate pump to structured sections and surface node (18c67a8)

- (cleanup) Reduce dead gui paths and share timing util (d111837)

- (modules) Split core files into focused submodules (eb882eb)

- (vst3) Share transport helpers and param mapping (143ba99)

- (metadata) Centralize plugin id and vendor policy (4b978c1)

- (params) Unify clap and vst3 mapping logic (242f757)

- (transport) Unify gui phase telemetry helpers (c943368)

- Decompose plugin entrypoint hotspots (08bf805)


### Testing

- Add pump host-resize jitter layout regression guard (48fe455)

- Update windows screenshot harness APIs (15f9228)

- Honor constrained size in windows screenshot capture (b44c5fb)

- Make windows screenshot capture runtime-size driven (30d20cb)

- Use headless screenshot harness on windows (5d9213d)

- Capture windows screenshots from live rendered window (c7b3c58)

- Make windows screenshot capture rust-2021 compatible (5c86358)

- (build) Cover config parsing and artifact path logic (dcc82d0)

- (state) Add malformed payload decode coverage (ebed91c)

- (gui) Add non-windows dropdown-over-curve coverage (dd87705)

