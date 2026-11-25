# ADR-0011 — Mixer Architecture and Routing Engine

**Date:** 2026-11-25  
**Status:** Accepted  
**Milestone:** M5 — Professional Mixing & Routing Engine  
**Authors:** Esperia Engineering

---

## Context

Milestone 5 introduces a full multitrack mixing architecture to Esperia.
Before M5, audio from scheduled clips was summed directly in the audio-engine
callback using a simple per-clip gain multiplier.  That approach cannot support:

- Per-track gain and panning
- Track mute and solo
- Group/auxiliary buses
- Metering (peak, RMS, clip detection)
- Deterministic offline rendering for tests
- Future plugin insert chains

We need a dedicated mixer crate that can be built, tested, and reasoned about
independently of the real-time audio callback, and that integrates with the
audio engine without introducing a dependency on CPAL.

---

## Decision

### 1. Dedicated `esperia-mixer` crate

All mixing logic lives in `crates/mixer/`.  The crate has zero external
dependencies (only `std`).  `esperia-audio-engine` depends on it; nothing
above it does.

```
crates/
├── audio-engine/   ← depends on mixer
├── audio-io/       ← no dependency on mixer
├── mixer/          ← new; zero external deps
│   └── src/
│       ├── gain.rs          dB↔linear conversion, Gain wrapper
│       ├── panning.rs       StereoPan (constant-power, √2 centre comp)
│       ├── meter.rs         AtomicMeter, MeterReading (lock-free)
│       ├── channel.rs       ChannelConfig descriptor
│       ├── channel_strip.rs ChannelStrip (preallocated buffer, process_in_place)
│       ├── bus.rs           Bus, BusConfig (summing, preallocated buffer)
│       ├── routing.rs       RouteTarget, SendRoute, cycle detection (Kahn's)
│       ├── master.rs        MasterChannel (headroom ceiling, process_to_output)
│       └── lib.rs           MixerGraph (full processing graph)
```

### 2. Zero real-time allocations

Every buffer required for processing (channel strips, buses, master) is
pre-allocated at graph construction time.  `MixerGraph::process()` performs
no heap allocation.  The `max_frames` parameter at construction sets the
largest buffer the graph will ever be asked to process.

The audio engine passes `max_frames = buffer_frames_hint × 4` so device
buffer size changes within that range need no reallocation.

### 3. Lock-free graph swap via `AtomicPtr`

The UI thread and the audio thread must never share a mutable reference to the
same `MixerGraph`.  We use two `Arc<AtomicPtr<MixerGraph>>` pointers — one for
publishing a new graph (`pending_mixer_ptr`), one for returning the retired
graph (`retired_mixer_ptr`):

```
UI thread                      Audio thread
─────────────────────────────  ──────────────────────────────────
Box::into_raw(new_graph)   →   swap(pending_mixer_ptr, null)
                               Box::from_raw(incoming) → owns graph
old_graph Box::into_raw    ←   swap(retired_mixer_ptr, old_raw)
Box::from_raw(retired) drop
```

**Invariants:**
- Only the audio thread calls `Box::from_raw(pending)` — once per non-null swap.
- Only the UI thread calls `Box::from_raw(retired)` — once per non-null swap.
- The `EngineHandle::Drop` impl drains both pointers to avoid leaks when the
  engine shuts down.

This design avoids any mutex in the audio callback and eliminates allocation
on both the hot path and during graph transitions.

### 4. Constant-power panning with √2 centre compensation

The pan law uses cosine/sine gains derived from the pan position `p ∈ [-1, 1]`:

```
θ = (p + 1) × π / 4          (θ ∈ [0, π/2])
gain_L = cos(θ)
gain_R = sin(θ)
```

A centre-panned mono signal through this law has each channel at `1/√2 ≈ 0.707`
linear.  To compensate so that centre-panned mono sums to 0 dB, both gains are
multiplied by `√2`:

```
final_L = cos(θ) × √2
final_R = sin(θ) × √2
```

This ensures that a mono source panned centre produces the same loudness as a
true stereo source at unity gain.

### 5. Topological sort with Kahn's algorithm for cycle detection

Buses can route into other buses, forming a directed acyclic graph (DAG).
Cycles in this graph would cause infinite audio feedback at process time.

We use Kahn's BFS-based topological sort at graph construction:

1. Compute in-degree for every bus node.
2. Enqueue buses with in-degree 0.
3. Process each queued bus, decrementing the in-degree of its successor.
4. If any bus remains with non-zero in-degree after the BFS, a cycle exists.

Construction returns `Err(RoutingError::CycleDetected)` for cyclic graphs.
Self-routes (a bus targeting itself) are caught before the BFS.

### 6. `ScheduledClip.track_id` → channel routing

Each `ScheduledClip` now carries a `track_id: u64` matching the `ChannelConfig.id`
in the mixer graph.  The audio engine uses this field to route decoded clip data
into the correct channel strip's preallocated input buffer.

If a clip's `track_id` has no corresponding channel strip (e.g., after a track
was deleted), the clip's data is accumulated directly into the master bus,
preserving audio output with a minor quality loss until the next graph rebuild.

### 7. `AtomicMeter` for lock-free metering

Meter state (peak, RMS, clip) is stored in `AtomicMeter` as a pack of
`AtomicU32` values using bit-casting (`f32::from_bits` / `f32::to_bits`).
Both the audio thread (write) and UI thread (read) access these atomics with
`Relaxed` ordering — the values are display-only and do not gate any
safety-critical decision.

---

## Consequences

**Positive:**
- Mixer logic is fully testable without audio hardware (`render_offline`).
- No real-time allocations; predictable latency in the audio callback.
- Lock-free graph hot-swap: device selection or project changes do not stall
  the audio thread.
- Cycle detection prevents feedback loops at construction time.
- Metering is always live with zero blocking.

**Negative / Trade-offs:**
- Raw pointer usage (`Box::into_raw` / `Box::from_raw`) introduces unsafety
  that must be carefully audited.  The safety invariant (single-reader per
  pointer per swap) is enforced by the design and documented in code comments.
- `MixerGraph` is rebuilt from scratch on every track/project change.  For
  large projects (100+ tracks) this may add a brief UI stutter; a future
  optimisation could patch individual channels in-place.
- Pan is currently fixed at centre (0.0) because `Track` in `project-model`
  does not yet carry a pan field.  This will be addressed in a future story.

---

## Alternatives Considered

| Alternative | Reason Rejected |
|---|---|
| `Arc<Mutex<MixerGraph>>` shared between threads | Mutex in audio callback violates real-time safety |
| Lock-free queue of mixer commands | More complex; rebuild is fast enough for current track counts |
| Integrate mixer into audio-engine | Breaks single-responsibility principle; makes mixer untestable in isolation |
| External mixer library (e.g., dasp) | Adds dependency; bespoke design gives full control over buffer layout |
