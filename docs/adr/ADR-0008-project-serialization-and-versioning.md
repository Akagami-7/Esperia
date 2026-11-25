# ADR-0008: Project Serialization Format, Versioning & Atomic Persistence

## Status
Accepted

## Context
Step 2 introduces `crates/project-model`, which manages projects, tracks, clips, audio source references, and command history. Projects are stored as self-contained directory bundles (`MyComposition.esperia/`), per ADR-0006.

Before freezing the persistence layer, we must decide:
1. The initial manifest serialization format (`project.json`).
2. How schema versions and forward migrations are handled.
3. How saves and autosaves protect against partial writes and corruption.

## Alternatives Considered (Manifest Format)

### 1. JSON (`serde_json`) — Chosen
- **Pros:** Human-readable, diffable in Git, trivially inspectable during early DAW development, zero binary alignment quirks, mature Rust ecosystem (`serde` + `serde_json`).
- **Cons:** Larger on disk and slower to parse than binary formats for massive projects with millions of automation points.

### 2. MessagePack / CBOR
- **Pros:** Compact binary representation, fast serialization.
- **Cons:** Opaque to inspect with standard text tools during debugging and early schema evolution.

### 3. SQLite
- **Pros:** Transactional queries and partial updates.
- **Cons:** Heavy C dependency, poor Git diffability, overkill for initial project sizes.

## Chosen Decision

1. **Manifest Format:** Use **versioned JSON (`project.json`)** via `serde` and `serde_json` for the MVP and Phase 1–2 milestones.
2. **Schema Versioning & Migration:**
   - Every manifest carries `schema_version: u32` (currently `2`).
   - On load, `persistence::load_project` probes `schema_version`:
     - Version `0` is rejected as corrupt.
     - Older supported versions (e.g., `1`) are migrated in-memory to `CURRENT_SCHEMA_VERSION`.
     - Future versions (`> CURRENT_SCHEMA_VERSION`) fail fast with `ProjectError::UnsupportedSchemaVersion`.
3. **Atomic Save & Recovery:**
   - `save_project` serializes to `project.json.tmp`, calls `sync_all()`, copies any existing `project.json` to `project.json.bak`, and atomically renames `project.json.tmp` to `project.json`.
   - On load, stale `project.json.tmp` files are cleaned up, and if `project.json` is missing or corrupt, `load_project` recovers from `project.json.bak`.
4. **Autosave Isolation:**
   - `autosave_project` writes to `Autosave/autosave_<id>.json` via its own `.tmp` + rename cycle and never modifies `project.json` or `project.json.bak`.
5. **Asset & Peak Cache Separation:**
   - Clips reference `AudioSource` entries by `SourceId` rather than duplicating files.
   - Missing audio files are reported in `AssetValidationReport::missing_sources` without aborting project load.
   - Missing waveform peak caches in `Peaks/` are regenerated automatically.

## Consequences
- **Positive:** Safe, crash-resilient persistence; trivial debugging; clean forward migration path; zero coupling to GUI or audio hardware.
- **Negative:** JSON parsing overhead if automation curves become very dense in later milestones.

## Future Reconsideration Conditions
If project manifests grow large enough that JSON load/save exceeds ~50 ms (e.g., dense per-sample automation in Phase 2/3), move dense arrays to binary sidecar files or adopt MessagePack/CBOR while keeping `schema_version` gating intact.
