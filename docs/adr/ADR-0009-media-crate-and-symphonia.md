# ADR-0009: Media Crate, Symphonia Decoding, and Lock-Free Clip Scheduling

- **Status:** Accepted
- **Date:** 2026-11-25
- **Scope:** `crates/media`, `crates/audio-engine`, `apps/desktop`

## Context

Step 3 transitions Esperia from a single-tone generator to a multitrack DAW capable of importing WAV audio, arranging clips on a timeline, and playing them back in real time. Per `AGENTS.md`:
- `audio-engine` must remain independent of `audio-io` and `cpal`.
- Audio callbacks must not perform disk I/O, heap allocation, or blocking mutexes.

## Decision

1. **Symphonia for Audio Decoding (`crates/media`)**:
   - Chosen over `hound` (WAV-only) and `ffmpeg` bindings (heavy native build dependencies).
   - Configured with `wav` and `pcm` features initially; FLAC/MP3/AAC can be enabled later without changing the `DecodedAudio` representation.
2. **Background Worker (`AssetManager`)**:
   - `AssetManager` spawns a dedicated worker thread fed by a bounded `crossbeam-channel` queue.
   - Decoding and optional linear resampling happen entirely off the UI and audio threads.
3. **Linear Resampling**:
   - `resample_linear` converts mismatched source sample rates to the active engine sample rate during background import so the real-time callback never performs per-sample rate conversion.
   - A windowed-sinc resampler (e.g., `rubato`) can replace `resample_linear` later without API changes.
4. **Lock-Free Clip Schedule Transfer**:
   - The UI thread builds an immutable `Arc<ClipSchedule>` containing `ScheduledClip` entries (referencing `Arc<Vec<f32>>` decoded buffers) and publishes it to `AudioEngine` via `AtomicPtr::swap`.
   - The audio callback reads the pointer with `Acquire` ordering and performs a single atomic refcount increment (`Arc::clone`), avoiding allocations and mutexes on the real-time path.

## Consequences

- `esperia-audio-engine` remains dependency-free and deterministically testable without audio hardware.
- Decoding large files does not block the egui render loop or the CoreAudio callback.
- Future streaming-from-disk for very long recordings can replace `Arc<Vec<f32>>` inside `ScheduledClip` with a preallocated ring buffer per voice.
