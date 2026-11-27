# ADR-0002: Desktop UI and Rendering

## Status
Accepted

## Context
A desktop DAW interface requires low-latency 60+ FPS rendering, responsive timeline zooming and scrolling, hardware-accelerated drawing, and smooth integration with native OS windowing on macOS, Windows, and Linux.

## Problem
The DAW needs a shared cross-platform interface with custom controls and GPU-backed timeline drawing. Maintaining a separate windowing and renderer stack alongside an application framework would duplicate event, surface, and rendering responsibilities.

## Alternatives
1. **eframe / egui:** Rust desktop framework and UI library with native window/input integration and a wgpu renderer.
2. **Raw winit + wgpu:** Maximum renderer ownership, with additional lifecycle, input, accessibility, and platform integration work owned by Esperia.
3. **Tauri/WebView:** HTML/CSS UI with native webview dependencies and an IPC boundary.

## Chosen Decision
Use **eframe + egui** for the desktop shell and custom DAW controls. Use its winit and wgpu integration rather than integrating independent windowing or rendering stacks. Do not add Vizia or another UI framework unless a demonstrated requirement justifies the additional dependency.

## Consequences
- **Positive:** Shared cross-platform window, input, and GPU infrastructure; custom-painted controls remain possible through egui.
- **Negative:** Large timelines need virtualization and careful clipping; immediate-mode layout does not remove the need for performance measurement.

## Future Reconsideration Conditions
Revisit only if profiling on representative projects identifies a concrete limitation that cannot be addressed within eframe/egui.
