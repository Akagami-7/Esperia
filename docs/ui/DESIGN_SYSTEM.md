# Esperia UI design system

## Direction

Graphite studio surfaces, restrained tactile depth, and selective violet accents. Matte surfaces are used for the workspace; raised surfaces are for controls; floating surfaces are reserved for overlays. Audio meaning colors remain semantic: green for positive/playback, red for record/danger, amber for warnings, and cyan for secondary editing.

## Source of truth

`crates/ui-kit` owns the palette, typography sizes, spacing, corner radii, shadow tokens, animation timings, and shared paint primitives. Desktop startup applies `Theme` once to egui. Components should receive the same `Theme`; they should not invent brand colors or radii.

Palette tokens beyond the base graphite steps:

- `control_pressed` — recessed fill for a pushed control. It is darker than `control`, so press reads as depth rather than as a second brand color.
- `glass` — the floating-surface fill. It is the raised panel color with a little transparency. It is not a backdrop blur; blur stays a later wgpu pass and is not required to draw an overlay.

Shadow tokens `panel_alpha`, `control_alpha`, `floating_alpha`, and `blur_radius` drive `Shadow::panel`, `Shadow::control`, and `Shadow::floating`. Matte workspace surfaces do not take a shadow. Raised controls use the control shadow. Overlays use the floating shadow.

The baseline palette and geometry are starting values and will be tuned in the Component Lab before controls are approved. Dimensions are egui logical points and scale with the display density.

The first milestone uses egui's bundled proportional and monospace font families to keep rendering independent of OS font availability. Inter and IBM Plex Mono font files are not present in the repository, so these named typefaces are not claimed as active assets yet.

## Component Lab

Run with `cargo run -p esperia-desktop --offline --features component-lab`. The compile-time-only screen constructs no DAW/audio application state. It previews shared surfaces and static button/toggle states; these are explicitly visual placeholders. The production button and toggle implementations follow only after reviewing this foundation.

## Current scope

Implemented: semantic tokens, egui theme application, matte/raised/floating surface painting, layered ambient shadows, borders, focus rings, dividers, and simple geometric icon primitives. The existing custom SVG folder was inspected and preserved; no SVG assets have been moved or overwritten. Their normalization and renderer integration remain a later deliberate step.

Not implemented in this milestone: the interactive button family, animated control states, knobs, faders, meters, popovers, backdrop blur, or bundled Inter/IBM Plex Mono font files.
