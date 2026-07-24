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

- Increase attenuation fill visibility (a06699c)

- Render attenuation fill with visible primitives (a0c079f)

- (gui) Align curve interaction width (adb8d27)

- (gui) Clear meter from raw transport state (70cb29f)

- (gui) Preserve inactive meter repaint (7045759)

- (gui) Reserve space for dB reference labels (d80e32c)

- (gui) Preserve Radiant push-through margin (f859009)

- (gui) Derive push-through from widget bounds (3eca60c)

- Fix gate declarative segment drag on Command (3428a3a)

- Fix latch Command on segment move press (46dd92b)

- Fix preserve Option empty canvas no-op (1ab9c98)


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

- (gui) Pin radiant for pump gui surface smoke (709219c)

- Remove local agent state files (9a5e07a)


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

- (changelog) Update changelog [skip ci] (bf82ce0)

- (changelog) Update changelog [skip ci] (ea4bafd)

- (changelog) Update changelog [skip ci] (70c18ee)

- (changelog) Update changelog [skip ci] (abfd1ec)

- (changelog) Update changelog [skip ci] (8be65ee)

- (changelog) Update changelog [skip ci] (91f7d69)

- (changelog) Update changelog [skip ci] (24e5f1f)

- (changelog) Update changelog [skip ci] (5a74360)

- (changelog) Update changelog [skip ci] (e12df21)

- (changelog) Update changelog [skip ci] (667c72a)

- (changelog) Update changelog [skip ci] (ac80d25)

- (changelog) Update changelog [skip ci] (3cb2dae)

- (changelog) Update changelog [skip ci] (1ea9e10)

- (changelog) Update changelog [skip ci] (b225d3b)

- (changelog) Update changelog [skip ci] (9ea4580)

- (changelog) Update changelog [skip ci] (a757bfd)

- (changelog) Update changelog [skip ci] (825d6b5)

- (changelog) Update changelog [skip ci] (1b4211c)

- (changelog) Update changelog [skip ci] (38b9901)

- (changelog) Update changelog [skip ci] (5f499f8)

- (changelog) Update changelog [skip ci] (f2e22ac)

- (changelog) Update changelog [skip ci] (dd4d53e)

- (changelog) Update changelog [skip ci] (cf13e59)

- (changelog) Update changelog [skip ci] (ed61ff2)

- (changelog) Update changelog [skip ci] (151a981)

- (changelog) Update changelog [skip ci] (8e971ab)

- (changelog) Update changelog [skip ci] (defcdce)

- (changelog) Update changelog [skip ci] (46ca808)

- (changelog) Update changelog [skip ci] (5189443)

- (changelog) Update changelog [skip ci] (e0bce35)

- (changelog) Update changelog [skip ci] (2098211)

- (changelog) Update changelog [skip ci] (eb7d4f9)

- (changelog) Update changelog [skip ci] (a874324)

- (changelog) Update changelog [skip ci] (ce4614c)

- (changelog) Update changelog [skip ci] (b115b00)

- Track OPT-1112 review state (472ef61)

- Record OPT-1112 review readiness (c01350b)

- (changelog) Update changelog [skip ci] (9deeaa7)

- Record OPT-1111 review state (219cd77)

- Record OPT-1111 review artifact (19d4615)

- Defer final artifact hash to PR (3b4f6d3)

- (changelog) Update changelog [skip ci] (e0b8ddc)

- Record OPT-1114 review state (87cd029)

- (changelog) Update changelog [skip ci] (06d2d0c)

- Record OPT-1110 review state (babe3a3)

- Record OPT-1110 review follow-up (3df9713)

- (changelog) Update changelog [skip ci] (0e4de15)

- Docs record OPT-1118 review state (601b833)

- Docs mark OPT-1118 ready for review (7b91f5c)

- (changelog) Update changelog [skip ci] (c8cb2d8)

- Record OPT-1115 review handoff (1dc1f96)

- (changelog) Update changelog [skip ci] (cc53981)

- Sync OPT-1117 review state (PR-002) (af48959)

- (changelog) Update changelog [skip ci] (ce991cf)

- (changelog) Update changelog [skip ci] (80e9ab8)

- (changelog) Update changelog [skip ci] (0a9a7fe)

- (changelog) Update changelog [skip ci] (c7063e5)

- (changelog) Update changelog [skip ci] (459ffd8)

- (changelog) Update changelog [skip ci] (116bf0e)

- (changelog) Update changelog [skip ci] (6e50f38)

- (changelog) Update changelog [skip ci] (383fb19)


### Features

- (vst3) Consume keys via edit-mode and registered shortcuts (2ce4027)

- (presets) Make init preset fully writable (253dec6)

- Add attenuation fill beneath curve (8584e02)

- (gui) Add curve dB reference guides (4c6b000)

- Add external sidechain triggering to Pump (#30) (fd6c4ce)


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

- Fix region drag undo coalescing and add regression test (d1199c5)

- Add right-drag marquee node selection in curve editor (a5804a9)

- Fix preset selection persistence on load (b97cfd2)

- Coalesce knob drag undo and clear stale drag mode (a49c08e)

- Skip no-op curve inserts at max node count (69d893a)

- Remove depth control and run pump at full depth (70e36b0)

- Add Pump quick shape strip presets (bbd56cd)

- Add per-preset quick slot previews (9d7b442)

- Add Pump grid override and snap controls (e15f0bd)

- Fix VST3 snap shortcut visibility (4dec26c)

- Fix quick slot hit targets and shift hover (9c0eea4)

- Render snap as checkbox control (c874d72)

- Use Ctrl-Z and Ctrl-Y for Pump undo (c6f924b)

- Fix macOS VST3 release compatibility (7d645c0)

- Fix macOS VST3 editor attach (4a359ec)

- Use Radiant surface for macOS VST3 editor (6649983)

- Share Pump Radiant editor surface (33b129b)

- Render Pump curve in Radiant editor (5db497c)

- Harden Pump Radiant VST3 editor open path (91f17cd)

- Assert hosted VST3 Radiant content (ae6c09f)

- Add Radiant curve point insertion (7e6f186)

- Extend Radiant curve canvas insertion (5dccd37)

- Add Radiant option segment curvature drag (04c5a10)

- Fix CI private git dependency fetch (fdd1999)

- Use Radiant token for CI git dependencies (7e4d677)

- Fix Windows dropdown preset setup (90a822f)

- Stabilize Windows dropdown regression (099f713)

- Merge pull request #1 from PORTALSURFER/codex/update-pump-radiant-gui

Extend Pump Radiant curve insertion (cbb4f09)

- Add Radiant curve node hover deletion (21f1d6f)

- Merge pull request #2 from PORTALSURFER/codex/pump-node-hover-delete

Add Radiant curve node hover deletion (f0f742e)

- Add Radiant playback position marker (72e46f4)

- Fix VST3 playhead redraw timer (3193828)

- Drive VST3 playhead redraws without host timers (77f808c)

- Force VST3 playhead redraw display passes (6bbc143)

- Refresh Radiant surface for realtime playhead (97ee6e2)

- Merge pull request #3 from PORTALSURFER/codex/playback-position-marker

Add Radiant playback position marker (da0a587)

- Bump Toybox for Ioskeley font (46c08bb)

- Point Pump at merged Toybox font commit (b7e7392)

- Merge pull request #4 from PORTALSURFER/codex/pump-ioskeley-toybox-rev

Bump Toybox for Ioskeley font (4725cac)

- Highlight active curve node hover (f782ba7)

- Merge pull request #5 from PORTALSURFER/wsvasek/opt-923-pump-highlight-curve-editor-nodes-on-hover

OPT-923: Highlight Pump curve nodes on hover (12e6fd1)

- Show Pump version build label (a213998)

- Merge pull request #6 from PORTALSURFER/wsvasek/opt-929-pump-show-versionbuild-as-a-small-subtle-ui-label

OPT-929: Show Pump version/build label (f87a57e)

- Guard Pump UI against visible name label (bd7edd6)

- Merge pull request #7 from PORTALSURFER/wsvasek/opt-928-pump-remove-visible-pump-label-from-the-ui

Pump: remove visible pump label from UI (b5ea672)

- Add Cmd-click numeric parameter entry (a0ff243)

- Merge pull request #8 from PORTALSURFER/wsvasek/opt-927-pump-add-cmd-click-numeric-entry-for-parameter-value-labels

OPT-927: Add Cmd-click numeric parameter entry (2fe5c76)

- Add global Pump curve slots (b025f8c)

- Make curve slot swatches uniform (9a36e9c)

- Pin Pump to merged Toybox command modifier (4247875)

- Support sticky curve point drag-through (f86a315)

- Merge pull request #10 from PORTALSURFER/wsvasek/opt-925-pump-support-sticky-drag-through-point-removal-in-curve

OPT-925: Support sticky curve point drag-through (baa6434)

- Merge pull request #11 from PORTALSURFER/codex/pump-attenuation-fill

Add subtle attenuation fill beneath Pump curve (e75ab67)

- Render Pump VST3 GUI through Radiant (#12)

* Use Radiant curve area fill in Pump

* docs(changelog): update changelog [skip ci]

* docs: record Pump curve fill validation

* docs: finalize Pump curve fill handoff

* docs: mark Pump curve fill ready for review

* Route Pump VST3 GUI through Radiant

* Pin Toybox first-paint sizing fix

* Record rebuilt Pump test artifact

* Pin Toybox key forwarding fix

* Record rebuilt Pump key-forwarding artifact

* Pin Toybox hosted-size preservation

* Record rebuilt Pump size-preservation artifact

* Pin Toybox modifier-safe text input

* Record rebuilt Pump modifier-input artifact

* Pin Toybox CI authentication update

* Record rebuilt Pump CI-auth artifact

* Pin embedded surface recovery fix

* Record rebuilt Pump surface-recovery artifact

* Pin shared embedded clip validation

* Record rebuilt clip-validation artifact

* Pin embedded text options support

* Record rebuilt text-options artifact

* Pin embedded animation clock fix

* Record rebuilt animation-clock artifact

* Pin Toybox hosted-size reopen fix

* Pin Toybox VST3 key callback fix

* Pin Toybox secondary-drag fix

* Pin Toybox drag modifier ordering fix

* Pin Toybox AppKit function-key fix

* Pin merged Radiant and Toybox stack (a246e51)

- Distinguish curve playhead from editable nodes (#13) (5f25e11)

- Remove blocking mutex acquisition from VST3 audio processing (#14) (74affe9)

- OPT-1139 Honor sample offsets for CLAP and VST3 automation (#15) (f52b9e9)

- OPT-1141 Bound quick-slot counts during state decode (#16)

* OPT-1141 Bound state decode collection counts

* docs: record OPT-1141 review state (6fac47c)

- OPT-1140 Preallocate CLAP realtime buffers (#17)

* OPT-1140 Preallocate CLAP realtime buffers

* docs: record OPT-1140 review state (b577961)

- OPT-1142 Report preset persistence failures (#18)

* Fix preset persistence failure reporting

* Update OPT-1142 review handoff

* Record OPT-1142 CI success

* Preserve undo across preset write failures

* Update OPT-1142 review artifact

* Make unwritable preset test root-safe

* Update OPT-1142 review artifact hash

* Keep preset warning visible during rename

* Update OPT-1142 review artifact hash (29474c9)

- OPT-1112 Add incoming waveform background (4e6e902)

- OPT-1112 Invalidate waveform on backward seeks (7373ab3)

- OPT-1112 Ignore empty blocks in waveform capture (c160167)

- OPT-1112 Let silent waveform data expire (d073fd6)

- OPT-1112 Reset waveform on forward seeks (7df178a)

- OPT-1112 Preserve CLAP waveform on empty blocks (5c12ab7)

- OPT-1112 Reset waveform on cycle remapping (f991670)

- Merge pull request #19 from PORTALSURFER/wsvasek/opt-1112-pump-optionally-show-the-incoming-waveform-or-kick-transient

OPT-1112 Add incoming waveform behind the Pump curve (8a24ba1)

- OPT-1111 Add sync-aware curve beat grid (e42f4f2)

- Merge pull request #20 from PORTALSURFER/wsvasek/opt-1111-pump-show-sync-aware-vertical-beat-divisions-in-the-curve

OPT-1111 Show sync-aware beat divisions in the Pump curve (11590ad)

- OPT-1114 Add live gain reduction meter (d74e9f9)

- Merge pull request #21 from PORTALSURFER/wsvasek/opt-1114-pump-add-a-compact-live-gain-reduction-meter-beside-the

OPT-1114 Pump: add a compact live gain-reduction meter beside the curve editor (1e20f6b)

- Merge pull request #22 from PORTALSURFER/wsvasek/opt-1110-pump-add-horizontal-db-reference-markings-to-the-curve

OPT-1110 Pump: add horizontal dB reference markings to the curve editor (358b88c)

- OPT-1118 add Command segment dragging (d031074)

- Merge pull request #23 from PORTALSURFER/wsvasek/opt-1118-pump-cmd-drag-curve-segments-as-a-unit-with-distinct-hover

OPT-1118 Pump: Cmd-drag curve segments as a unit (fedfa70)

- OPT-1115 add Command beat-grid snapping (f0c8e54)

- Merge pull request #24 from PORTALSURFER/wsvasek/opt-1115-pump-hold-cmd-to-snap-curve-edits-to-the-beat-grid

OPT-1115 Pump: snap curve edits to the beat grid with Cmd (37dab27)

- Implement vertical curve point constraint (910fa01)

- Merge pull request #25 from PORTALSURFER/wsvasek/opt-1117-pump-hold-shiftoption-to-constrain-curve-point-movement

OPT-1117: constrain curve-point movement vertically (68fa1c5)

- Merge pull request #26 from PORTALSURFER/codex/remove-local-agent-state-files

Remove local agent state files (3f3ae92)

- OPT-1119 add Cmd+Shift cyclic whole-curve offset (#27)

* OPT-1119 add Cmd+Shift cyclic curve offset

* Implement exact cyclic curve offset gesture

* Pin Toybox dependency by full commit SHA

* Correct full Toybox commit pin

* Record full Toybox revision in lockfile

* Preserve version one global curve slots

* Preserve exact phase data when loading saved curves

* Require current command modifier for curve offset

* Update toybox dependency revision

* Update toybox dependency revision (3923d03)

- OPT-1133 add Depth and Floor attenuation parameters (#28)

* feat: add depth and floor attenuation parameters

* fix: honor zero depth at silence points

* fix: preserve floor text and test migration

* test: fix vst3 parameter test imports (19922a6)

- [OPT-1131] Restyle global curve slots as target carousel

Preserve global curve slot persistence and interaction semantics while presenting slots as the target thumbnail carousel. (e2a7a6d)

- [OPT-1127] Harden external sidechain trigger routing

Approved and validated. Merge OPT-1127 sidechain trigger routing. (4ddff72)

- [OPT-1125] Redesign Pump preset navigation and add favorites (#32)

* feat: redesign pump preset navigation and favorites

* fix: preserve preset action hit sizes in warning state (6194573)

- Keep input waveform always on (e88ea64)

- Merge pull request #33 from PORTALSURFER/wsvasek/opt-1291-pump-remove-the-input-waveform-toggle-and-keep-the-waveform

OPT-1291: Keep the input waveform always on (ffa3201)


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

