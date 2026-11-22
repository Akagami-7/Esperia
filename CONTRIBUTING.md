# Contributing to Esperia

Thank you for contributing to Project Esperia! Please review these guidelines before submitting a pull request.

---

## 1. Core Engineering Principles
1. **Real-Time Audio Safety:** Any code intended to run within an audio callback MUST NOT allocate memory, acquire standard OS mutexes, perform I/O, or call unbounded operations.
2. **Crate Decoupling:** Respect crate boundaries. `audio-io` must never know about `audio-engine` or `desktop`. `audio-engine` must never depend on GUI state.
3. **No Unhandled Unwraps:** Use Rust's `Result` and domain errors in library crates.

---

## 2. Development Workflow

1. Fork or branch from `main`:
   ```bash
   git checkout -b feature/my-feature-name
   ```
2. Verify local checks pass:
   ```bash
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
3. Commit using Conventional Commits:
   ```
   feat(audio-io): add device buffer size query
   fix(audio-engine): prevent zipper noise during pan change
   ```
4. Submit a Pull Request targeting `main`.

