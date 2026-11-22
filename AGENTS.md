# Project Esperia — Engineering Guidelines

## Language
- Primary language: Rust.
- Rust edition: 2024.
- Use stable Rust unless explicitly approved otherwise.

## Architecture
- Keep GUI and audio processing independent.
- Audio-engine must not depend on audio-io.
- Audio callbacks must not perform disk I/O.
- No blocking operations in the real-time callback.
- Keep application state separate from runtime audio state.

## Code Quality
- Run `cargo fmt`.
- Run `cargo clippy`.
- Add tests for new functionality.
- Avoid unnecessary dependencies.
- Never suppress failing tests merely to pass CI.
- Document non-obvious engineering decisions via ADRs in `docs/adr/`.
- No unhandled `unwrap()` calls in production library code; use structured domain errors.

## Development
- Use feature branches (`feature/*`, `fix/*`, `refactor/*`).
- Make small, reviewable changes.
- Read relevant architecture documentation before modifying modules.
- Document important technical decisions.
- Never modify unrelated components.
- Run relevant tests (`cargo test --workspace`) before proposing a merge.

## Audio Safety
- Avoid heap allocation inside real-time callbacks.
- Avoid blocking mutexes.
- Avoid unbounded operations.
- Use preallocated buffers and bounded communication.
- Test audio processing under different buffer sizes.
