# ADR-0010: Audio Recording Subsystem and Device-Aware Latency Compensation

- **Status:** Accepted
- **Date:** 2026-11-25
- **Scope:** `crates/audio-io`, `crates/recording`, `crates/audio-engine`, `crates/project-model`, `apps/desktop`

## Context

Milestone 4 introduces real-time audio recording (vocals, guitars, microphones, and external interfaces) with overdubbing, input monitoring, punch-in, take management, and project persistence. Per `AGENTS.md`:
- The CPAL input callback must never perform disk I/O, heap allocation, or blocking synchronisation.
- `esperia-audio-engine` must not depend on `cpal` or `esperia-audio-io`.
- Recorded takes must align accurately with the playback timeline rather than using a hardcoded millisecond offset.

## Decision

1. **Dedicated `esperia-recording` Crate**:
   - Separated into `input_capture`, `buffer_manager`, `file_writer`, `recorder`, `recording_state`, and `recording_errors`.
   - `InputCapture` uses a preallocated scratch buffer to extract the selected mono/stereo/all channel(s) and pushes into a preallocated lock-free SPSC `CaptureRingBuffer`.
   - Overruns are tracked atomically in `CaptureRingBuffer` and `RecordingTelemetry` and surfaced as `RecordingError::RingBufferOverrun` instead of silently dropping audio.
   - A background worker thread drains `CaptureRingBuffer` into a streaming 16-bit PCM `WavFileWriter`.
2. **Input Monitoring via `MonitorRing` in `esperia-audio-engine`**:
   - `AudioEngine` owns a preallocated SPSC `MonitorRing` and `EngineCommand::EnableInputMonitoring(bool)`.
   - `apps/desktop` forwards captured samples from the CPAL input callback to `MonitorRing` lock-free, keeping `audio-engine` and `recording` decoupled.
3. **Device-Aware Latency Compensation (`LatencyConfig` + `compute_aligned_placement`)**:
   - `esperia-audio-io` computes hardware input latency (`callback - capture`) from `cpal::InputCallbackInfo` and predicted output latency (`playback - callback`) from `cpal::OutputCallbackInfo`.
   - When hardware timestamps are unavailable on a host, `LatencyConfig` falls back to `(input_buffer_frames / input_sr) + (output_buffer_frames / output_sr)` plus any manual calibration offset.
   - If compensation places a take before timeline frame 0, leading samples are trimmed cleanly instead of producing negative timeline positions.
   - If `input_sample_rate != project_sample_rate`, `Recorder::stop_recording` resamples the recorded WAV to `project_sample_rate` off-thread before creating the timeline clip.

## Consequences

- Real-time input callbacks remain allocation-free and lock-free.
- Recorded takes align with overdubbed playback tracks across varying devices, sample rates, and buffer sizes.
- Saving a project bundle copies all recorded WAV files into `<bundle>/Audio/`, ensuring reopening restores and plays back every take.
