# Project Resonance — Engineering Guidelines

## General
- Primary language: Rust.
- Follow Rust 2024 edition conventions.
- Use the Cargo workspace structure.
- Keep modules focused and responsibilities separated.
- Do not introduce dependencies without justification.

## Architecture
- The GUI must not directly manipulate audio-thread internals.
- Audio processing must remain independent of GUI rendering.
- Project state must remain independent of audio-device state.
- Never perform disk I/O on the real-time audio callback.
- Never introduce unbounded operations in the audio callback.
- Maintain strict real-time safety in the audio path: zero memory allocations, zero locks/mutexes, zero syscalls.

## Code Quality
- Use `cargo fmt` for formatting.
- Pass `cargo clippy` without warnings.
- Add tests for new behavior.
- Do not suppress failing tests to make builds pass.
- Document non-obvious engineering decisions via ADRs in `docs/adr/`.
- No unhandled `unwrap()` calls in production library code; use structured domain errors.

## Development Workflow
- Work on feature branches (`feature/*`, `fix/*`, `refactor/*`).
- Keep changes small and reviewable.
- Do not modify unrelated modules.
- Explain architectural implications before major changes.
- Run relevant tests (`cargo test --workspace`) before proposing a merge.
