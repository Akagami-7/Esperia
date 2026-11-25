//! `esperia-audio-io` — CPAL audio device enumeration and stream management.
//!
//! # Architecture rules (enforced)
//! * This crate depends on `cpal` only.
//! * It has **zero** dependency on `esperia-audio-engine` or `esperia-desktop`.
//! * The hot-path render callback is provided by the caller (desktop crate).
//! * Stream errors are reported through a `StreamErrorChannel` — a lock-based
//!   channel that is **only written from the CPAL error callback** and **only
//!   read from the UI thread**.  It is never touched inside the render callback.

pub use cpal;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fmt;
use std::sync::{Arc, Mutex};

// ─────────────────────────────────────────────────────────────────────────────
// Error types
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum AudioIoError {
    HostUnavailable(String),
    DeviceUnavailable(String),
    DefaultDeviceNotFound,
    DeviceNotFound(String),
    ConfigurationError(String),
    StreamBuildError(String),
    StreamPlayError(String),
}

impl fmt::Display for AudioIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostUnavailable(m) => write!(f, "Audio host unavailable: {m}"),
            Self::DeviceUnavailable(m) => write!(f, "Audio device unavailable: {m}"),
            Self::DefaultDeviceNotFound => write!(f, "Default audio device not found"),
            Self::DeviceNotFound(n) => write!(f, "Audio device not found: {n}"),
            Self::ConfigurationError(m) => write!(f, "Invalid audio configuration: {m}"),
            Self::StreamBuildError(m) => write!(f, "Failed to build audio stream: {m}"),
            Self::StreamPlayError(m) => write!(f, "Failed to start audio stream: {m}"),
        }
    }
}

impl std::error::Error for AudioIoError {}

// ─────────────────────────────────────────────────────────────────────────────
// StreamErrorChannel — non-blocking error reporting from CPAL error callback
// ─────────────────────────────────────────────────────────────────────────────

/// A lightweight channel that carries stream error messages from the CPAL error
/// callback to the UI thread.
///
/// # Real-time safety note
/// The CPAL error callback is *not* on the hot-path render callback — it fires
/// asynchronously when CPAL detects a fatal stream fault.  Using a `Mutex` here
/// is acceptable because error callbacks are exceptional, infrequent events.
/// **Never use this inside the render callback.**
#[derive(Clone, Default)]
pub struct StreamErrorChannel {
    inner: Arc<Mutex<Option<String>>>,
}

impl StreamErrorChannel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called by the CPAL error callback. Stores the latest error message.
    /// Silently drops the error if the mutex is poisoned.
    pub fn report(&self, msg: String) {
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some(msg);
        }
    }

    /// Take the pending error (if any). Called from the UI thread.
    pub fn take(&self) -> Option<String> {
        self.inner.lock().ok()?.take()
    }

    /// True if an error is waiting to be collected.
    pub fn has_error(&self) -> bool {
        self.inner.lock().ok().and_then(|g| g.clone()).is_some()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AudioStreamConfig — the resolved configuration for an open stream
// ─────────────────────────────────────────────────────────────────────────────

/// The resolved configuration actually used to open the CPAL stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamConfig {
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

// ─────────────────────────────────────────────────────────────────────────────
// AudioDeviceInfo
// ─────────────────────────────────────────────────────────────────────────────

/// Summary of a discovered audio device, independent of `cpal::Device`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub is_default_input: bool,
    pub is_default_output: bool,
    pub min_channels: u16,
    pub max_channels: u16,
    pub min_sample_rate: u32,
    pub max_sample_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioHostInfo {
    pub id: String,
    pub name: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// AudioDeviceManager
// ─────────────────────────────────────────────────────────────────────────────

pub struct AudioDeviceManager {
    host: cpal::Host,
}

impl AudioDeviceManager {
    pub fn new() -> Result<Self, AudioIoError> {
        Ok(Self {
            host: cpal::default_host(),
        })
    }

    pub fn with_host(host_id: cpal::HostId) -> Result<Self, AudioIoError> {
        let host = cpal::host_from_id(host_id)
            .map_err(|e| AudioIoError::HostUnavailable(e.to_string()))?;
        Ok(Self { host })
    }

    pub fn available_hosts() -> Vec<AudioHostInfo> {
        cpal::available_hosts()
            .into_iter()
            .map(|id| AudioHostInfo {
                id: format!("{id:?}"),
                name: format!("{id:?}"),
            })
            .collect()
    }

    pub fn host_name(&self) -> String {
        format!("{:?}", self.host.id())
    }

    // ── Device access ─────────────────────────────────────────────────────

    pub fn default_output_device(&self) -> Result<cpal::Device, AudioIoError> {
        self.host
            .default_output_device()
            .ok_or(AudioIoError::DefaultDeviceNotFound)
    }

    pub fn default_input_device(&self) -> Result<cpal::Device, AudioIoError> {
        self.host
            .default_input_device()
            .ok_or(AudioIoError::DefaultDeviceNotFound)
    }

    /// Human-readable name for a `cpal::Device`.
    pub fn device_display_name(dev: &cpal::Device) -> String {
        if let Ok(desc) = dev.description() {
            let name = desc.name();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        if let Ok(id) = dev.id() {
            return format!("{id:?}");
        }
        "Audio Device".to_string()
    }

    /// Find an output device by its display name.
    pub fn find_output_device_by_name(&self, name: &str) -> Result<cpal::Device, AudioIoError> {
        let devices = self
            .host
            .output_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?;
        for dev in devices {
            if Self::device_display_name(&dev) == name {
                return Ok(dev);
            }
        }
        Err(AudioIoError::DeviceNotFound(name.to_string()))
    }

    /// Find an input device by its display name.
    pub fn find_input_device_by_name(&self, name: &str) -> Result<cpal::Device, AudioIoError> {
        let devices = self
            .host
            .input_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?;
        for dev in devices {
            if Self::device_display_name(&dev) == name {
                return Ok(dev);
            }
        }
        Err(AudioIoError::DeviceNotFound(name.to_string()))
    }

    // ── Configuration helpers ─────────────────────────────────────────────

    /// Query the native default output configuration for `dev`.
    /// Returns `(sample_rate, channels)`.  Falls back to `(48000, 2)` on failure.
    pub fn default_output_config_params(dev: &cpal::Device) -> (u32, u16) {
        if let Ok(config) = dev.default_output_config() {
            (config.sample_rate(), config.channels())
        } else {
            (48_000, 2)
        }
    }

    /// Query the native default input configuration for `dev`.
    /// Returns `(sample_rate, channels)`.  Falls back to `(48000, 1)` on failure.
    pub fn default_input_config_params(dev: &cpal::Device) -> (u32, u16) {
        if let Ok(config) = dev.default_input_config() {
            (config.sample_rate(), config.channels())
        } else {
            (48_000, 1)
        }
    }

    // ── Enumeration ───────────────────────────────────────────────────────

    pub fn enumerate_output_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioIoError> {
        let default_name = self
            .host
            .default_output_device()
            .map(|d| Self::device_display_name(&d));

        let mut result = Vec::new();
        for dev in self
            .host
            .output_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?
        {
            let name = Self::device_display_name(&dev);
            let is_default = default_name.as_deref() == Some(&name);
            let (min_ch, max_ch, min_sr, max_sr) = Self::supported_ranges(&dev);
            result.push(AudioDeviceInfo {
                name,
                is_default_input: false,
                is_default_output: is_default,
                min_channels: min_ch,
                max_channels: max_ch,
                min_sample_rate: min_sr,
                max_sample_rate: max_sr,
            });
        }
        Ok(result)
    }

    pub fn enumerate_input_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioIoError> {
        let default_name = self
            .host
            .default_input_device()
            .map(|d| Self::device_display_name(&d));

        let mut result = Vec::new();
        for dev in self
            .host
            .input_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?
        {
            let name = Self::device_display_name(&dev);
            let is_default = default_name.as_deref() == Some(&name);
            let (min_ch, max_ch, min_sr, max_sr) = Self::supported_input_ranges(&dev);
            result.push(AudioDeviceInfo {
                name,
                is_default_input: is_default,
                is_default_output: false,
                min_channels: min_ch,
                max_channels: max_ch,
                min_sample_rate: min_sr,
                max_sample_rate: max_sr,
            });
        }
        Ok(result)
    }

    fn aggregate_ranges<I>(configs: I) -> (u16, u16, u32, u32)
    where
        I: Iterator<Item = cpal::SupportedStreamConfigRange>,
    {
        let mut min_ch = u16::MAX;
        let mut max_ch = 0u16;
        let mut min_sr = u32::MAX;
        let mut max_sr = 0u32;
        let mut any = false;
        for c in configs {
            any = true;
            min_ch = min_ch.min(c.channels());
            max_ch = max_ch.max(c.channels());
            min_sr = min_sr.min(c.min_sample_rate());
            max_sr = max_sr.max(c.max_sample_rate());
        }
        if any {
            (min_ch, max_ch, min_sr, max_sr)
        } else {
            (0, 0, 0, 0)
        }
    }

    fn supported_ranges(dev: &cpal::Device) -> (u16, u16, u32, u32) {
        let Ok(configs) = dev.supported_output_configs() else {
            return (0, 0, 0, 0);
        };
        Self::aggregate_ranges(configs)
    }

    fn supported_input_ranges(dev: &cpal::Device) -> (u16, u16, u32, u32) {
        let Ok(configs) = dev.supported_input_configs() else {
            return (0, 0, 0, 0);
        };
        Self::aggregate_ranges(configs)
    }

    // ── Stream construction ───────────────────────────────────────────────

    /// Build and start an `f32` output stream on `device`.
    ///
    /// Stream errors are delivered to `error_channel`; they are **not** passed
    /// to the render callback.  The render callback (`render_fn`) must be
    /// real-time safe.
    pub fn build_output_stream<F>(
        &self,
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
        error_channel: StreamErrorChannel,
        mut render_fn: F,
    ) -> Result<cpal::Stream, AudioIoError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        self.build_output_stream_with_timing(
            device,
            sample_rate,
            channels,
            error_channel,
            move |data, _| render_fn(data),
        )
    }

    /// Build and start an `f32` output stream on `device`, providing the
    /// hardware-predicted output latency in nanoseconds (when reported by host).
    pub fn build_output_stream_with_timing<F>(
        &self,
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
        error_channel: StreamErrorChannel,
        mut render_fn: F,
    ) -> Result<cpal::Stream, AudioIoError>
    where
        F: FnMut(&mut [f32], Option<u64>) + Send + 'static,
    {
        let config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    let ts = info.timestamp();
                    let latency_ns = ts
                        .playback
                        .checked_duration_since(ts.callback)
                        .map(|d| d.as_nanos() as u64);
                    render_fn(data, latency_ns);
                },
                move |err| {
                    error_channel.report(err.to_string());
                },
                None,
            )
            .map_err(|e| AudioIoError::StreamBuildError(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioIoError::StreamPlayError(e.to_string()))?;

        Ok(stream)
    }

    /// Build and start an `f32` input stream on `device`.
    ///
    /// `capture_fn` receives interleaved `f32` samples and the hardware-reported
    /// input latency in nanoseconds (`callback - capture`), if available.
    /// `capture_fn` **must** be real-time safe (no allocation, no disk I/O, no mutexes).
    pub fn build_input_stream<F>(
        &self,
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
        error_channel: StreamErrorChannel,
        mut capture_fn: F,
    ) -> Result<cpal::Stream, AudioIoError>
    where
        F: FnMut(&[f32], Option<u64>) + Send + 'static,
    {
        let config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let stream = device
            .build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], info: &cpal::InputCallbackInfo| {
                    let ts = info.timestamp();
                    let latency_ns = ts
                        .callback
                        .checked_duration_since(ts.capture)
                        .map(|d| d.as_nanos() as u64);
                    capture_fn(data, latency_ns);
                },
                move |err| {
                    error_channel.report(err.to_string());
                },
                None,
            )
            .map_err(|e| AudioIoError::StreamBuildError(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioIoError::StreamPlayError(e.to_string()))?;

        Ok(stream)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_error_channel_roundtrip() {
        let ch = StreamErrorChannel::new();
        assert!(!ch.has_error());
        assert!(ch.take().is_none());

        ch.report("device disconnected".into());
        assert!(ch.has_error());
        let msg = ch.take();
        assert_eq!(msg.as_deref(), Some("device disconnected"));
        assert!(ch.take().is_none()); // consumed
    }

    #[test]
    fn test_stream_error_channel_overwrites() {
        let ch = StreamErrorChannel::new();
        ch.report("first".into());
        ch.report("second".into());
        // Only the last write is kept (Option stores one value).
        let msg = ch.take().unwrap();
        assert_eq!(msg, "second");
    }

    #[test]
    fn test_stream_error_channel_clone_shares_state() {
        let ch1 = StreamErrorChannel::new();
        let ch2 = ch1.clone();
        ch1.report("from clone".into());
        assert_eq!(ch2.take().as_deref(), Some("from clone"));
    }

    #[test]
    fn test_device_manager_initializes() {
        if let Ok(mgr) = AudioDeviceManager::new() {
            assert!(!mgr.host_name().is_empty());
        }
        // CI/headless: skip — AudioDeviceManager::new() may succeed but
        // enumerate nothing; that's acceptable.
    }

    #[test]
    fn test_available_hosts_non_empty() {
        // At least one host (CoreAudio / WASAPI / ALSA) must be present.
        if AudioDeviceManager::new().is_ok() {
            let hosts = AudioDeviceManager::available_hosts();
            assert!(!hosts.is_empty());
        }
    }

    #[test]
    fn test_output_device_enumeration_graceful() {
        if let Ok(mgr) = AudioDeviceManager::new() {
            let result = mgr.enumerate_output_devices();
            // Either succeeds or returns a structured error — never panics.
            assert!(
                result.is_ok() || matches!(result, Err(AudioIoError::DeviceUnavailable(_))),
                "unexpected error: {result:?}",
            );
        }
    }

    #[test]
    fn test_find_nonexistent_device_returns_not_found() {
        if let Ok(mgr) = AudioDeviceManager::new() {
            let result = mgr.find_output_device_by_name("__no_such_device__");
            assert!(
                matches!(result, Err(AudioIoError::DeviceNotFound(_))),
                "expected DeviceNotFound"
            );
        }
    }

    #[test]
    fn test_default_output_config_params_fallback() {
        // Construct a mock by using real device if available, or verify fallback.
        if let Ok(mgr) = AudioDeviceManager::new()
            && let Ok(dev) = mgr.default_output_device()
        {
            let (sr, ch) = AudioDeviceManager::default_output_config_params(&dev);
            assert!((8_000..=384_000).contains(&sr), "sample_rate={sr}");
            assert!(ch >= 1, "channels={ch}");
        }
    }
}
