pub use cpal;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::fmt;

#[derive(Debug)]
pub enum AudioIoError {
    HostUnavailable(String),
    DeviceUnavailable(String),
    DefaultDeviceNotFound,
    ConfigurationError(String),
    StreamBuildError(String),
    StreamPlayError(String),
}

impl fmt::Display for AudioIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostUnavailable(msg) => write!(f, "Audio host unavailable: {msg}"),
            Self::DeviceUnavailable(msg) => write!(f, "Audio device unavailable: {msg}"),
            Self::DefaultDeviceNotFound => write!(f, "Default audio device was not found"),
            Self::ConfigurationError(msg) => write!(f, "Invalid audio configuration: {msg}"),
            Self::StreamBuildError(msg) => write!(f, "Failed to build audio stream: {msg}"),
            Self::StreamPlayError(msg) => write!(f, "Failed to start audio stream: {msg}"),
        }
    }
}

impl std::error::Error for AudioIoError {}

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

pub struct AudioDeviceManager {
    host: cpal::Host,
}

impl AudioDeviceManager {
    pub fn new() -> Result<Self, AudioIoError> {
        let host = cpal::default_host();
        Ok(Self { host })
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

    fn device_display_name(dev: &cpal::Device) -> String {
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

    pub fn enumerate_output_devices(&self) -> Result<Vec<AudioDeviceInfo>, AudioIoError> {
        let default_name = self
            .host
            .default_output_device()
            .map(|d| Self::device_display_name(&d));

        let devices = self
            .host
            .output_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?;

        let mut result = Vec::new();
        for dev in devices {
            let name = Self::device_display_name(&dev);
            let is_default = default_name.as_deref() == Some(&name);
            let (min_ch, max_ch, min_sr, max_sr) = match dev.supported_output_configs() {
                Ok(configs) => {
                    let mut min_ch = u16::MAX;
                    let mut max_ch = 0;
                    let mut min_sr = u32::MAX;
                    let mut max_sr = 0;
                    let mut has_any = false;

                    for c in configs {
                        has_any = true;
                        min_ch = min_ch.min(c.channels());
                        max_ch = max_ch.max(c.channels());
                        min_sr = min_sr.min(c.min_sample_rate());
                        max_sr = max_sr.max(c.max_sample_rate());
                    }

                    if has_any {
                        (min_ch, max_ch, min_sr, max_sr)
                    } else {
                        (0, 0, 0, 0)
                    }
                }
                Err(_) => (0, 0, 0, 0),
            };

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

        let devices = self
            .host
            .input_devices()
            .map_err(|e| AudioIoError::DeviceUnavailable(e.to_string()))?;

        let mut result = Vec::new();
        for dev in devices {
            let name = Self::device_display_name(&dev);
            let is_default = default_name.as_deref() == Some(&name);
            result.push(AudioDeviceInfo {
                name,
                is_default_input: is_default,
                is_default_output: false,
                min_channels: 0,
                max_channels: 0,
                min_sample_rate: 0,
                max_sample_rate: 0,
            });
        }

        Ok(result)
    }

    pub fn build_output_stream<F>(
        &self,
        device: &cpal::Device,
        sample_rate: u32,
        channels: u16,
        mut render_callback: F,
    ) -> Result<cpal::Stream, AudioIoError>
    where
        F: FnMut(&mut [f32]) + Send + 'static,
    {
        let config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let err_callback = |err| {
            tracing::error!("Audio output stream error: {err}");
        };

        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    render_callback(data);
                },
                err_callback,
                None,
            )
            .map_err(|e| AudioIoError::StreamBuildError(e.to_string()))?;

        stream
            .play()
            .map_err(|e| AudioIoError::StreamPlayError(e.to_string()))?;

        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_device_manager_initialization() {
        let manager = AudioDeviceManager::new();
        assert!(manager.is_ok());
        let hosts = AudioDeviceManager::available_hosts();
        assert!(!hosts.is_empty());
    }

    #[test]
    fn test_host_name_not_empty() {
        let manager = AudioDeviceManager::new().expect("device manager init");
        assert!(!manager.host_name().is_empty());
    }

    #[test]
    fn test_output_device_enumeration() {
        let manager = AudioDeviceManager::new().expect("device manager init");
        let devices = manager.enumerate_output_devices();
        assert!(devices.is_ok());
    }
}
