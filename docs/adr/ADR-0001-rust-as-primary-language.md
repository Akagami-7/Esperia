# ADR-0001: Rust as Primary Language

## Status
Accepted

## Context
A Digital Audio Workstation (DAW) requires microsecond-level determinism in its real-time audio threads, high graphical throughput for visual timeline rendering, and strong memory safety to prevent project corruption or crashes during live performance and studio recording.

## Problem
Traditional DAWs written in C++ suffer from persistent memory corruption bugs, data races, brittle concurrency boundaries, and complex platform-dependent build systems. Languages with Garbage Collection (Java, C#, Go) introduce unpredictable runtime pauses that violate real-time audio callback deadlines, causing audible dropouts (XRuns).

## Alternatives
1. **C++ (C++20):** Industry standard for audio plugins and legacy DAWs. High performance, but lacks compiler-enforced memory safety and thread safety guarantees.
2. **C# / .NET:** Rapid UI development, but garbage collection latency spikes make reliable sub-millisecond audio callbacks unpredictable.
3. **Zig:** Modern manual memory management and C interoperability, but smaller ecosystem and lacks mature audio/GUI production libraries.

## Chosen Decision
We choose **Rust (2024 edition)** as the primary programming language for Resonance across all modules (engine, I/O, UI, and tooling).

## Consequences
- **Positive:** Fearless concurrency (`Send`/`Sync`), compile-time prevention of data races, zero-cost abstractions, robust package management with Cargo, and memory safety without GC pauses.
- **Negative:** Steeper initial learning curve around borrow checking in complex circular graph structures (such as audio routing matrices and DSP DAGs).

## Future Reconsideration Conditions
None for core architecture. Native C ABI bindings (`extern "C"`) will be provided in later phases for third-party plugin hosting (CLAP / VST3).
