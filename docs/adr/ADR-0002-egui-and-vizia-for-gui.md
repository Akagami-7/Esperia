# ADR-0002: GUI Framework Selection — egui/eframe with Vizia Support

## Status
Accepted

## Context
A desktop DAW interface requires low-latency 60+ FPS rendering, responsive timeline zooming and scrolling, hardware-accelerated drawing, and smooth integration with native OS windowing on macOS, Windows, and Linux.

## Problem
Web-based desktop shells (Electron, Tauri/WebView) incur significant memory overhead, DOM layout lag, and non-deterministic event loop latency. Native platform wrappers (AppKit, WinUI, GTK) require writing and maintaining three completely disparate GUI codebases.

## Alternatives
1. **egui / eframe:** Immediate-mode, pure Rust GUI library rendered via `wgpu`. Extremely fast to iterate, low memory overhead, straightforward state binding.
2. **Vizia:** Modern retained-mode GUI library in Rust tailored specifically for audio plugins and musical applications with reactive state management, CSS-like styling, and accessibility support.
3. **Iced:** Retained-mode Elm-inspired architecture, good layout system, but slower turnaround for custom audio canvas rendering (waveforms, timeline clips).

## Chosen Decision
We adopt **`eframe` / `egui`** for the core application shell and immediate developer preview and debug tooling, while actively integrating and supporting **`vizia`** for complex reactive widgets, channel strips, and musical styling components. Both operate on top of native windowing (`winit`) and GPU rendering (`wgpu`).

## Consequences
- **Positive:** Fast development velocity, instant UI state synchronization with audio telemetry, native GPU performance, and modularity to leverage both immediate-mode flexibility and retained-mode audio widgets.
- **Negative:** Careful layout caching is required for immediate-mode drawing of thousands of timeline clips to avoid unnecessary per-frame polygon generation.

## Future Reconsideration Conditions
If immediate-mode rendering reaches CPU bounds on extremely large arrangements (e.g. 500+ tracks), retained-mode rendering via Vizia will take over the primary timeline canvas.
