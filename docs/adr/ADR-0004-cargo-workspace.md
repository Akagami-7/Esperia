# ADR-0004: Cargo Workspace Architecture

## Status
Accepted

## Context
Esperia consists of multiple decoupled subsystems: a desktop GUI application, a real-time DSP audio engine, a hardware I/O driver layer, and future modules such as project serialization, audio decoding, and command history.

## Problem
A monolithic single-crate architecture encourages tight coupling between the user interface, real-time audio callback internals, and hardware drivers. A mistake in the GUI could easily introduce blocking calls or memory allocations into the audio processing loop.

## Alternatives
1. **Single Monolithic Crate:** All code in one crate with modules. Easy to start, but no compiler-enforced boundary protection.
2. **Multi-Repository Setup:** Separate Git repos for audio engine, I/O, and desktop. Too heavy for rapid day-to-day synchronized iteration.
3. **Cargo Workspace:** A single Git repository with a root `Cargo.toml` orchestrating independent member crates with shared dependency caching and unified testing.

## Chosen Decision
We adopt a virtual Cargo workspace with resolver 3, Rust 2024, and shared minimum Rust version, license, and lint policy. The workspace contains `apps/desktop` and seven domain crates: `audio-io`, `audio-engine`, `project-model`, `media`, `recording`, `mixer`, and `midi`.

## Consequences
- **Positive:** Compiler-enforced dependency separation, fast incremental builds, shared dependency resolution, and single-command workspace verification (`cargo test --workspace`).
- **Negative:** Minor configuration boilerplate in crate `Cargo.toml` files.

## Future Reconsideration Conditions
Add a crate only when it has a stable, independent responsibility and a real dependency boundary.

