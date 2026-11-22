# Product Requirements Document (PRD) — Esperia

**Document Version:** 1.0.0  
**Status:** Approved / Active Specification  
**Product Name:** Esperia  
**Target Milestone:** v0.1.0 MVP → v1.0.0 Production  

---

## 1. Executive Summary & Product Vision

**Esperia** is a modern, modular, local-first Digital Audio Workstation (DAW) built in Rust. It delivers uncompromising audio reliability, sub-millisecond determinism, and an elegant, hardware-inspired desktop user experience across macOS, Windows, and Linux.

Commercial DAWs have evolved over decades into monolithic, bloated suites encumbered by proprietary licensing dongles, mandatory cloud telemetry, sluggish web-wrapper frontends, and brittle audio thread boundaries. Esperia exists to reset these conventions:
- **Rock-solid stability:** The real-time audio thread never allocates, never blocks, and never crashes due to UI or background tasks.
- **Native performance:** Built on Rust and modern native graphics rendering, achieving a constant 60+ FPS UI alongside low-latency audio capture and playback.
- **Local-first & private:** Projects are self-contained folders with transparent, human-readable metadata, standard lossless audio formats, and zero unsolicited network telemetry.
- **Extensible & modular:** Engine, hardware I/O, DSP pipelines, and UI are completely isolated crates that can be developed, tested, or reused independently.

---

## 2. Product Philosophy & Core Principles

1. **Audio Integrity is Sacred:** An audio glitch, dropout, or unexpected clip is an engineering defect. The audio callback is isolated with zero memory allocations, zero locks, and deterministic scheduling.
2. **Local-First & Sovereign:** The user owns their data. A project created today must open 20 years from now without checking an authentication server or cloud license manager.
3. **Ergonomic Speed:** Every frequent producer action (slicing, looping, soloing, zooming, automation) must be accessible via direct keyboard shortcuts and immediate visual feedback.
4. **Transparent Modularity:** Clean separation between presentation (`esperia-desktop`), core processing (`esperia-audio-engine`), and hardware integration (`esperia-audio-io`).

---

## 3. Target Personas & Use Cases

### Persona 1: The Electronic Producer & Beatmaker (Primary MVP Focus)
- **Profile:** Alex, 26, produces electronic music, hip-hop, and synth-pop. Uses loops, one-shots, multi-track audio layering, and precise grid-based arrangement.
- **Needs:** Snappy timeline navigation, quick drag-and-drop sample import, non-destructive slicing and stretching, low CPU overhead, and responsive channel mixing.
- **Pain Point with Existing DAWs:** Slow project load times, bloat, unexpected crashes mid-session, intrusive DRM popups.

### Persona 2: The Media Composer & Sound Designer (Primary MVP Focus)
- **Profile:** Sarah, 34, scores games and commercial video. Works with hundreds of audio stems, field recordings, and sound layers.
- **Needs:** Rock-solid timeline synchronization, sample-accurate clip alignment, seamless stem exporting, and clean project organization.
- **Pain Point with Existing DAWs:** Poor multi-core scaling, project file corruption, erratic latency compensation.

### Persona 3: The Recording Musician & Audio Engineer (Phase 2 Focus)
- **Profile:** David, 41, records live multi-mic acoustic drums, guitars, and vocal takes in his home studio.
- **Needs:** Ultra-low latency monitoring, multi-channel input routing, reliable zero-dropout recording to disk, and visual level metering with true peak warnings.
- **Pain Point with Existing DAWs:** Audio glitches during takes, complex setup menus, fragile ASIO/CoreAudio device switching.

---

## 4. Core User Workflows

```mermaid
flowchart LR
    A["Create Project / Template"] --> B["Import Audio / Record Takes"]
    B --> C["Arrange & Edit Clips on Timeline"]
    C --> D["Mix: Gain, Pan, Metering & Busses"]
    D --> E["Export Lossless Master / Stems"]
```

### 4.1 Project Inception
- Launch desktop application, create new project or open recent workspace.
- Configurable default sample rate (44.1 kHz, 48.0 kHz, 96.0 kHz) and buffer size (64 to 1024 frames).

### 4.2 Asset Ingestion & Organization
- Drag-and-drop audio files (WAV, FLAC, AIFF) directly onto the timeline or track headers.
- Automatic background generation of waveform visual caches without blocking playback.

### 4.3 Arrangement & Editing
- Non-destructive clip positioning, trimming, splitting, duplication, and volume fading.
- Grid snapping aligned with musical time (beats, bars, divisions) or absolute timecode.

### 4.4 Mixing & Monitoring
- Dedicated mixer console with channel strips (faders, pan pots, mute, solo, record arm).
- Peak and RMS metering with clip indicators.
- Summing into stereo master output bus.

### 4.5 Offline Bounce & Export
- Faster-than-realtime master mix render to 16-bit, 24-bit, or 32-bit float WAV files.
- Individual stem rendering for stems handover.

---

## 5. Scope & Release Roadmap

### Phase 1: MVP (v0.1.0 — Current Scope)
- Rust workspace architecture (`apps/desktop`, `crates/audio-engine`, `crates/audio-io`).
- Native UI shell with dark studio theme (Header, Transport, Timeline, Mixer, Inspector).
- CPAL audio device enumeration and stereo playback/capture stream initialization.
- Lock-free ring buffer communication between UI and real-time audio thread.
- Multitrack audio playback with gain, pan, mute, and solo controls.
- WAV audio import, non-destructive clip trimming, and basic arrangement.
- Project save/load using JSON project manifest.
- Offline WAV master export.

### Phase 2: Creative Tools & DSP (v0.2.0)
- Native built-in DSP modules: Parametric 4-band EQ, Stereo Delay, Compressor, Peak Limiter.
- Automation lanes for volume, pan, and plugin parameters.
- Undo/Redo command history stack with complete persistence.
- Audio recording directly into timeline takes with punch-in/out.

### Phase 3: Plugin Ecosystem & Advanced Routing (v0.3.0 - v1.0.0)
- CLAP and VST3 third-party plugin hosting with sandbox crash protection.
- Flexible bus routing, sidechaining, and auxiliary send/returns.
- MIDI sequencing, piano roll editor, and software synthesizer integration.
- Hardware controller integration (Mackie Control Universal, MIDI CC).

---

## 6. Non-Goals (Explicitly Out of Scope)

To maintain sharp focus and rapid execution, the following capabilities are **explicitly postponed** or rejected:
1. **Cloud Sync & Collaboration Server:** Esperia will remain 100% local-first. Cloud backup can be handled by standard file tools (Git, Syncthing, Dropbox).
2. **Proprietary DRM or Activation Keyrings:** Esperia will never require online phone-home activation.
3. **Video Editing Suite:** While video sync/SMPTE playback will be considered in future for scoring, video compositing will not be developed.
4. **Integrated Social Media Distribution:** No direct-to-streaming uploaders or social media integrations inside the DAW.
5. **Score Notation / Engraving:** Dedicated notation typesetting is deferred to specialized notation tools (MusicXML export will be favored).

---

## 7. Success Criteria & Quality Gates

| Milestone | Deliverable | Acceptance Criteria |
| :--- | :--- | :--- |
| **M1: Audio Scaffold** | `crates/audio-io` & `crates/audio-engine` | Reliable playback through CPAL on macOS (CoreAudio), Linux (ALSA/Pulse), Windows (WASAPI) with zero panics. |
| **M2: UI & Timeline Shell** | `apps/desktop` | 60 FPS viewport rendering, responsive track zooming, playhead tracking, zero latency interference with audio thread. |
| **M3: End-to-End Playback** | Multitrack Engine Integration | Playback of at least 32 simultaneous 44.1kHz/24-bit stereo audio tracks with fader and pan changes under 25% CPU on modern hardware. |
| **M4: Project Persistence** | Serialization & Reload | Saving project creates a clean directory containing `.esperia` project JSON and audio pool; reloads bit-identically. |
| **M5: Offline Bounce** | Master Exporter | Rendered WAV matches real-time playback sum within 32-bit floating point precision limits. |
