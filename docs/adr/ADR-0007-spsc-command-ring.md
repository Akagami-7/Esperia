# ADR-0007: Bounded SPSC Ring Buffer for Engine↔UI Communication

## Status
Accepted

## Context
Esperia's audio callback runs on a real-time priority thread. The UI thread must be able to send commands (play, stop, frequency change, reconfigure) to the audio engine, and the audio engine must be able to publish telemetry (peak level, playhead position, underrun count, callback duration) back to the UI — all without blocking either thread.

Standard Rust concurrency primitives (`std::sync::Mutex`, `std::sync::mpsc`) are off-limits inside the audio callback because:
- Mutexes can block indefinitely if the OS preempts the holder.
- `mpsc` channels heap-allocate internally under load.
- Either can cause priority inversion and produce audible XRuns.

## Problem
We need a cross-thread command and telemetry pathway that is:
1. Wait-free on the audio thread (no blocking, no allocation).
2. Non-panicking under overload (safe to call even if the ring is full).
3. Simple enough to audit for real-time safety at a glance.

## Alternatives Considered

### 1. `std::sync::Mutex<VecDeque<Command>>`
Simple to write. Rejected: the audio thread can block if the UI thread holds the lock. Direct violation of ADR-0005.

### 2. `crossbeam::channel` (unbounded)
Ergonomic. Rejected: unbounded channels heap-allocate when the queue grows; the audio thread must never call `malloc`.

### 3. `crossbeam::channel` (bounded)
Better, but `send` on a bounded channel blocks when full. The UI thread can use `try_send`, but the audio thread's `recv` path still touches internal allocator locks on some platforms.

### 4. `ringbuf` crate (SPSC lock-free ring)
Well-tested, purpose-built. Considered, but adds an external dependency. We have Cargo hygiene goals (avoid unnecessary dependencies — AGENTS.md).

### 5. Hand-rolled bounded atomic SPSC ring (chosen)
A fixed-capacity ring of `[AtomicU32; 3]` slots with two `AtomicUsize` head/tail pointers:
- Push (UI thread): write three `u32` words to `slots[head & (N-1)]`, advance `head`.
- Pop (audio thread): read the slot if `tail != head`, advance `tail`.
- If the ring fills, the producer overwrites the oldest unread entry. This is acceptable: the audio thread must never block waiting for a UI command. Missing one intermediate frequency-drag position is imperceptible.

## Chosen Decision

A hand-rolled `CommandRing<const N: usize>` with `N = 64` slots (`Box<[[AtomicU32; 3]; N]>`):
- Capacity is sufficient for any realistic burst of UI commands within one audio block.
- Each command is encoded as `[tag: u32, arg0: u32, arg1: u32]`, keeping slots fixed-size.
- Telemetry (peak level, underrun count, callback duration, playing flag) is communicated via individual `AtomicU32` / `AtomicU64` / `AtomicBool` on `EngineTelemetry` — no ring needed for single-value telemetry.

## Consequences

### Positive
- Zero heap allocation in the audio callback.
- No blocking synchronisation primitives anywhere on the audio thread.
- Self-contained: no additional dependency.
- Easy to audit: the entire implementation is ~50 lines of `std::sync::atomic` code.

### Negative
- If the UI sends more than 64 commands before a single audio block processes them, the oldest commands are silently dropped. At typical UI frame rates (60 Hz) and audio block rates (> 300 Hz at 128 frames @ 48 kHz), this is structurally impossible in normal operation.
- The fixed command encoding (`[u32; 3]`) must be extended if future commands require more than two `f32`/`u32` arguments. In that case the ring slots can be widened or a secondary channel can carry large payloads.

## Future Reconsideration Conditions
When the command set grows beyond what fits in `[u32; 3]`, evaluate adopting `rtrb` or `ringbuf` (both zero-alloc, lock-free SPSC crates) rather than widening the hand-rolled ring further.
