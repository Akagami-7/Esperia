//! `esperia-media` — audio decoding, resampling, and asset management.
//!
//! # Architecture
//! * Decoding runs on a background worker thread — never inside the audio callback.
//! * The decoded audio (`Arc<DecodedAudio>`) is immutable after creation; it can be
//!   shared safely across threads without locking.
//! * `AssetManager` owns a background thread that processes load requests sequentially.
//!   Results are stored in an `Arc<Mutex<HashMap>>` and readable from any thread.
//!
//! # Supported formats
//! * WAV (pcm, f32, i16, i24, i32) via Symphonia.

pub mod asset;
pub mod decode;
pub mod error;
pub mod resample;

pub use asset::{AssetManager, LoadStatus};
pub use decode::DecodedAudio;
pub use error::MediaError;
pub use esperia_project_model::audio_source::SourceId;
