//! `esperia-recording` — real-time-safe audio recording engine.
//!
//! # Architecture
//! * `InputCapture` runs in the CPAL input callback: zero heap allocations,
//!   zero disk I/O, zero blocking mutexes.
//! * `CaptureRingBuffer` transfers captured samples lock-free to the background
//!   recording worker, detecting and reporting buffer overruns.
//! * `WavFileWriter` encodes and flushes audio to disk on the background thread.
//! * `Recorder` manages track arming, channel selection, punch-in/out, takes,
//!   and device-aware latency compensation.

pub mod buffer_manager;
pub mod file_writer;
pub mod input_capture;
pub mod recorder;
pub mod recording_errors;
pub mod recording_state;

pub use buffer_manager::CaptureRingBuffer;
pub use input_capture::InputCapture;
pub use recorder::{AlignedPlacement, LatencyConfig, Recorder, compute_aligned_placement};
pub use recording_errors::RecordingError;
pub use recording_state::{
    InputChannelSelection, PunchRegion, RecordingStatus, RecordingTelemetry, TakeInfo,
};
