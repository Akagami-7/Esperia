# ADR-0005: Strict Real-Time Audio Thread Isolation

## Status
Accepted

## Context
Audio rendering operates under strict hardware-driven real-time callback deadlines (e.g., $1.33\text{ ms}$ at 64 samples @ 48 kHz). Missing a deadline causes an under-run (XRun), resulting in audible glitches, pops, or dropped audio.

## Problem
Standard application programming patterns rely on dynamic heap allocation (`malloc`/`free`), mutual exclusion locks (`std::sync::Mutex`), logging to console or disk, and kernel system calls. In an audio callback, any lock contention or heap allocator lock can block the audio thread indefinitely, causing immediate audio failure.

## Alternatives
1. **Standard `Mutex` / `RwLock` shared state:** Simple to write, but fatal for real-time audio due to priority inversion and unpredictable blocking duration.
2. **Actor Model / Channel Message Passing:** Using standard channels (`crossbeam`, `std::sync::mpsc`) which may allocate internally under high message loads.
3. **Lock-Free Bounded Ringbuffers (SPSC):** Pre-allocated, wait-free Single-Producer Single-Consumer queues with atomic head and tail pointers.

## Chosen Decision
We enforce **strict real-time audio thread isolation**:
- The audio callback thread NEVER performs heap allocations, acquires OS mutexes, performs disk/network I/O, or makes blocking calls.
- Inter-thread communication between UI and the audio engine operates strictly via bounded, pre-allocated lock-free ringbuffers (SPSC) and atomic variables.

## Consequences
- **Positive:** Mathematically deterministic audio callback timing, complete immunity to UI stalls, and zero dropouts caused by garbage collection or lock contention.
- **Negative:** Requires disciplined software design: all scratch memory must be pre-allocated, and state synchronization must be modeled through non-blocking command queues.

## Future Reconsideration Conditions
None. This is a non-negotiable core invariant of Resonance.

