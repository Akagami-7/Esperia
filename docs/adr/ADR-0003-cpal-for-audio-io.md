# ADR-0003: CPAL for Audio Device I/O

## Status
Accepted

## Context
Esperia requires reliable, low-latency audio device enumeration, stream initialization, and audio buffer callbacks across macOS, Windows, and Linux.

## Problem
Each operating system provides radically different native audio APIs: CoreAudio on macOS, WASAPI / ASIO on Windows, and ALSA / PulseAudio / PipeWire / JACK on Linux. Writing and maintaining bespoke driver wrappers for all five interfaces directly inside the engine would introduce massive maintenance burden and platform fragmentation.

## Alternatives
1. **Direct native bindings per OS:** Write CoreAudio (macOS), WASAPI (Windows), and ALSA (Linux) manually using `bindgen`. Highly labor-intensive, error-prone.
2. **PortAudio C library wrapper:** Proven multi-platform C library, but requires compiling external C code, complicating cross-compilation and Cargo builds.
3. **CPAL (Cross-Platform Audio Library):** Pure Rust multi-platform audio library actively maintained by the Rust audio community, providing unified abstractions over CoreAudio, WASAPI, and ALSA.

## Chosen Decision
We choose **CPAL 0.18.2**, encapsulated entirely within `crates/audio-io`. Product support targets macOS 14.2+, Windows 10 22H2+, and Linux x86_64/glibc 2.35+ with ALSA development/runtime support. These are Esperia release baselines, not a claim that every audio backend has been tested on every OS image.

## Consequences
- **Positive:** Pure Rust dependency chain, zero external C library toolchain dependencies, and native support for CoreAudio on macOS, WASAPI on Windows, and ALSA on Linux.
- **Negative:** CPAL does not bundle native proprietary ASIO support by default on Windows due to Steinberg licensing constraints (requires user build flag `asio` if needed). Linux audio runtime behavior varies by distribution and device server.

## Future Reconsideration Conditions
If hardware performance or exclusive low-latency routing requires specialized features not exposed by CPAL (such as direct JACK transport sync or ASIO direct channel routing), `audio-io` will implement an internal pluggable driver backend architecture.

