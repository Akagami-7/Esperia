# Resonance Engineering Roadmap

This roadmap documents our transition from empty repository to a full production DAW.

---

## Stage 1 — Product Discovery & Specifications (Completed)
- [x] Product Requirements Document (`docs/product/PRD.md`)
- [x] Software Requirements Specification (`docs/requirements/SRS.md`)
- [x] System Architecture Document (`docs/architecture/ARCHITECTURE.md`)
- [x] UI/UX Specification (`docs/ui/UI_SPECIFICATION.md`)
- [x] Audio Engine Specification (`docs/audio-engine/AUDIO_ENGINE_SPEC.md`)
- [x] Testing Strategy (`docs/testing/TESTING_STRATEGY.md`)
- [x] Architecture Decision Records (`docs/adr/ADR-0001` to `ADR-0006`)

## Stage 2 — Technical Foundation (Current)
- [x] Virtual Cargo workspace with resolver 3 and Rust 2024 edition.
- [x] Crate separation: `apps/desktop`, `crates/audio-io`, `crates/audio-engine`.
- [x] Workspace dependencies: `eframe`, `vizia`, `cpal`, `tracing`, `tracing-subscriber`.
- [x] Engineering guidelines (`AGENTS.md`) and CI pipeline configuration.
- [x] Clean zero-warning verification across `cargo check`, `cargo fmt`, `cargo clippy`, and `cargo test`.

## Stage 3 — First Working DAW
- [x] Native window management and developer preview shell in `resonance-desktop`.
- [x] Audio device enumeration subsystem in `resonance-audio-io` (hosts, inputs, outputs, default devices, sample rates).
- [x] Sine-wave test tone generator in `resonance-audio-engine` (440 Hz, real-time safe callback, play/stop, frequency control).
- [ ] Multi-threaded audio playback through CPAL without blocking UI.
- [ ] Symphonia WAV file decoding & streaming.
- [ ] Project persistence (manifest save/reopen).

## Stage 4 — Arrangement & Editing
- [ ] Multi-track timeline model with integer sample-frame timebase.
- [ ] Non-destructive clip editing (trim, split, duplicate, delete).
- [ ] Background waveform peak generation and multi-resolution rendering.
- [ ] Command layer for full undo/redo history.

## Stage 5 — Recording & Mixing
- [ ] Multi-channel audio recording via CPAL input stream with ringbuffer disk streaming.
- [ ] Mixer console with volume faders, constant-power panning, mute, and solo.
- [ ] Peak and RMS metering telemetry.
- [ ] Offline faster-than-realtime WAV export.

