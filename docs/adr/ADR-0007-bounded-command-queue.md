# ADR-0007: Bounded Command Queue and Atomic Telemetry

## Status
Accepted

## Context
The audio callback needs an ordered control path from application/UI and MIDI producers, plus low-cost status snapshots for the UI. The callback cannot block, allocate, or overwrite unread commands.

## Decision
Use a fixed-capacity, sequence-numbered MPSC command ring with one audio-thread consumer. `try_send` reports `Full` or `Contended` within a fixed retry budget. The callback drains at most 16 commands per block. Convenience transport calls expose rejected commands through an atomic telemetry counter.

Single-value telemetry such as peak level, underrun count, callback duration, playhead, and transport state uses atomic latest-value snapshots. It does not need a queue. Audio sample and live MIDI streams use separate bounded SPSC rings with explicit overflow behavior.

## Alternatives
- `Mutex<VecDeque<Command>>`: rejected because the callback could block.
- `std::sync::mpsc`: rejected because queue growth and callback behavior are not bounded.
- Bounded channel libraries: not selected for this foundation because callback-side lock-free guarantees and queue-full semantics need to remain directly auditable.

## Consequences
- Commands are never silently overwritten; senders must report or retry a rejected command outside the callback.
- Queue storage is fixed and command draining is capped per callback.
- The hand-rolled queue requires stress tests and memory-ordering review as the command format changes.
- Schedule and mixer graph lifetime use their own bounded publication/reclamation protocols, documented in ADR-0005.
