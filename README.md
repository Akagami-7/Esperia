# Project Resonance

> **Resonance** is a modern, modular, local-first Digital Audio Workstation (DAW) built in Rust for macOS, Windows, and Linux.

---

## Architecture Overview

Resonance enforces strict thread isolation and lock-free concurrency between the user interface and the real-time audio pipeline:

```
Esperia / resonance-daw/
├── apps/
│   └── desktop/            # Native desktop GUI (eframe/egui & vizia)
├── crates/
│   ├── audio-engine/       # Real-time DSP graph, mixing, and tone synthesis
│   └── audio-io/           # CPAL-powered audio device enumeration & stream I/O
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
- Rust 1.85+ / 2024 edition (`rustup default stable`)
- Clang / C compiler build utilities

### Building and Testing

```bash
# Check all workspace crates
cargo check --workspace

# Build the desktop binary
cargo build --workspace

# Run full test suite
cargo test --workspace

# Check formatting and linter
cargo fmt --all --check
cargo clippy --workspace --all-targets

# Run the desktop application
cargo run -p resonance-desktop
```

## Licensing

Dual-licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

