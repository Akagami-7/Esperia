# Phase 0 Baseline

Recorded 2026-11-28 before Phase 0 edits.

## Repository state

- Branch: `feature/midi-composition`
- HEAD: `99ed76a feat(ESPERIA-006): add urlencoding and wire create_count_metronome_button in Tauri setup`
- The working tree was already dirty. It contained an uncommitted switch from the checked-in eframe application to a Tauri/native-wgpu experiment, plus generated files. That experiment was treated as prior work and audited before the Phase 0 cleanup.

## Toolchain

- Host: `aarch64-apple-darwin`
- Installed compiler: Rust 1.100.0, Cargo 1.100.0
- Installed target: `aarch64-apple-darwin`
- Rust 1.95.0 was not installed at baseline.

## Baseline verification

Commands were run offline on macOS before modifications:

- `cargo fmt --all -- --check`: passed
- `cargo check --workspace --all-targets --offline`: passed
- `cargo test --workspace --all-targets --offline`: passed
- `cargo clippy --workspace --all-targets --offline`: exited successfully, with an `items_after_test_module` warning in the experimental renderer crate.

These results describe the local macOS build only. They do not establish Windows/Linux compilation, hardware audio behavior, or callback timing.
