# Project Esperia

> **Esperia** is a modern, modular, local-first Digital Audio Workstation (DAW) built in Rust for macOS, Windows, and Linux.

---

## Architecture Overview

Esperia enforces strict thread isolation and lock-free concurrency between the user interface and the real-time audio pipeline:

```
Esperia/
├── apps/
│   └── desktop/            # Desktop application and eframe/egui UI
├── crates/
│   ├── audio-io/           # Audio hardware and CPAL streams
│   ├── audio-engine/       # Audio processing and transport
│   ├── project-model/      # Project state and persistence
│   ├── media/              # Audio decoding and asset management
│   ├── recording/          # Audio capture and recording workflow
│   ├── mixer/              # Mixing and routing
│   └── midi/               # MIDI events, devices, and musical time
└── docs/                   # Full engineering and product specifications
    ├── product/            # PRD
    ├── requirements/       # SRS
    ├── architecture/       # System Architecture
    ├── ui/                 # UI/UX Specification
    ├── audio-engine/       # Audio Engine Specification
    ├── testing/            # Testing Strategy
    └── adr/                # Architecture Decision Records
```

## Quick Start

### Prerequisites
- macOS, Linux, or Windows
- Rust 1.95.0+ / Rust 2024 edition (`rustup toolchain install 1.95.0`)
- Clang / C compiler build utilities

### Platform baseline
- macOS 14.2 or later (CPAL CoreAudio backend baseline)
- Windows 10 22H2 or later
- Linux x86_64 on a glibc 2.35+ distribution; ALSA development headers are required to build

The CI matrix currently uses the latest GitHub-hosted macOS, Windows, and Ubuntu
runners. It checks cross-platform compilation, but does not certify device
latency or every distribution/runtime combination.

### Building and Testing

```bash
# Check all workspace crates
cargo check --workspace

# Build the desktop binary
cargo build --workspace

# Run full test suite
cargo test --workspace --all-targets

# Check formatting and linter
cargo fmt --all --check
cargo clippy --workspace --all-targets

# Run the desktop application
cargo run -p esperia-desktop
```

The desktop UI uses eframe/egui. Windowing and GPU rendering come through
eframe's winit/wgpu integration; don't add a second window or rendering stack
without a specific requirement and an ADR.

## License

Licensed under either of:

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
