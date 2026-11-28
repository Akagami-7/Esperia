# Esperia UI design system

## Direction

Graphite studio surfaces, restrained tactile depth, and selective violet accents. Matte surfaces are used for the workspace; raised surfaces are for controls; floating surfaces are reserved for overlays. Audio meaning colors remain semantic: green for positive/playback, red for record/danger, amber for warnings, and cyan for secondary editing.

## Module Architecture

- **`crates/ui-kit` (`esperia-ui`)** — single source of truth for all reusable UI code:
  - `theme/` — semantic palette, spacing/radii/shadow/animation metrics, and typography tokens
  - `animation/` — spring-damper physics (`SpringNode`, `SpringParams`) and easing/color transitions
  - `primitives/` — matte/raised/floating surfaces, ambient shadows, borders, focus rings, dividers, and geometric icons
  - `components/` — reusable interactive widgets organized by category:
    - `buttons/` (`PrimaryButton`, `SecondaryButton`, `IconButton`, `PillButton`, `ToggleButton`, `TransportButton`, `SegmentedButton`, `ButtonState`)
    - `toggles/` (`Toggle`)
    - `knobs/` (`RotaryKnob`, `KnobMode`)
    - `faders/` (`VolumeFader`, `FaderOrientation`, dB taper helpers)
    - `gliders/` (`VolumeGlider` with elastic spring-chain physics)
    - `meters/` (`PeakMeter`, `RmsMeter`, `MeterState`)
- **`apps/desktop/src/`** — desktop application only (no duplicate component library):
  - `main.rs` / `app.rs` — DAW coordinator (`EsperiaApp`)
  - `views/` — application-specific DAW views (`piano_roll.rs`)
  - `platform/` — OS integrations (`macos_menu.rs`)
  - `lab/` — dev-only Component Lab (`--features component-lab`)

## Source of truth

`crates/ui-kit` owns the palette, typography sizes, spacing, corner radii, shadow tokens, animation timings, and shared paint primitives. Desktop startup applies `Theme` once to egui. Components should receive the same `Theme`; they should not invent brand colors or radii.

Palette tokens beyond the base graphite steps:

- `control_pressed` — recessed fill for a pushed control. It is darker than `control`, so press reads as depth rather than as a second brand color.
- `glass` — the floating-surface fill. It is the raised panel color with a little transparency. It is not a backdrop blur; blur stays a later wgpu pass and is not required to draw an overlay.

Shadow tokens `panel_alpha`, `control_alpha`, `floating_alpha`, and `blur_radius` drive `Shadow::panel`, `Shadow::control`, and `Shadow::floating`. Matte workspace surfaces do not take a shadow. Raised controls use the control shadow. Overlays use the floating shadow.

The baseline palette and geometry are tuned in the Component Lab. Dimensions are egui logical points and scale with the display density.

## Component Lab

Run with `cargo run -p esperia-desktop --features component-lab`. The compile-time-only screen constructs no DAW/audio application state and previews all `esperia-ui` buttons, toggles, knobs, faders, spring gliders, meters, and surface levels.
