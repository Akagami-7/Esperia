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
3. **Bounded lock-free queues and atomic snapshots:** Preallocated MPSC/SPSC queues are used where messages are required; latest-value metrics use atomics.

## Chosen Decision
The audio callback must not allocate or deallocate, acquire blocking locks, perform file/network/console I/O, or wait on another thread. Processing and command draining have explicit bounds. Queue saturation has a defined result and may not overwrite unread data.

Clip and MIDI schedules are immutable snapshots read through a callback guard. The publisher retains prior versions until readers are quiescent, then reclaims them outside the callback; a fixed retention cap rejects excess updates. Mixer graphs use one pending pointer slot and one retired pointer slot. If the retired slot is occupied, the callback defers replacement and keeps the active graph. Raw pointers require these explicit ownership/lifetime protocols; an `Arc` refcount increment after an unprotected pointer load is unsafe.

## Consequences
- **Positive:** Makes callback work and ownership transitions auditable and prevents UI stalls from blocking audio.
- **Negative:** Bounded queues can reject updates; callers must report or retry them outside the callback. This policy is not a claim that all current engine paths already meet the rule.

## Future Reconsideration Conditions
Revisit only if measured requirements show that a different, equally safe ownership model is needed.

