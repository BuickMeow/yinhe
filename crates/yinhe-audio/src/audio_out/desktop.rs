//! cpal 后端（桌面 + Android）。

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use super::{BufferSize, OutputFormat, StreamError, StreamErrorKind};

pub(crate) struct Device(cpal::Device);
pub(crate) struct Stream(cpal::Stream);

fn to_cpal_buffer(buffer: BufferSize) -> cpal::BufferSize {
    match buffer {
        BufferSize::Default => cpal::BufferSize::Default,
        BufferSize::Fixed(n) => cpal::BufferSize::Fixed(n),
    }
}

fn f32_configs(device: &cpal::Device) -> Option<Vec<cpal::SupportedStreamConfigRange>> {
    device.supported_output_configs().ok().map(|configs| {
        configs
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
            .collect()
    })
}

impl Device {
    pub(crate) fn default_format(&self) -> Result<OutputFormat, String> {
        let cfg = self.0.default_output_config().map_err(|e| e.to_string())?;
        Ok(OutputFormat {
            sample_rate: cfg.sample_rate(),
            channels: cfg.channels(),
        })
    }

    /// 设备支持的 f32 采样率范围；`None` = 枚举失败（调用方不做判断）。
    pub(crate) fn f32_rate_ranges(&self) -> Option<Vec<(u32, u32)>> {
        f32_configs(&self.0).map(|configs| {
            configs
                .iter()
                .map(|c| (c.min_sample_rate(), c.max_sample_rate()))
                .collect()
        })
    }

    /// 设备支持的缓冲区范围（取第一个 f32 配置）；`None` = 未知。
    pub(crate) fn buffer_range(&self) -> Option<(u32, u32)> {
        f32_configs(&self.0).and_then(|configs| match configs.first()?.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            cpal::SupportedBufferSize::Unknown => None,
        })
    }

    pub(crate) fn build_output<D, E>(
        &self,
        format: OutputFormat,
        buffer: BufferSize,
        mut data: D,
        mut error: E,
    ) -> Result<Stream, String>
    where
        D: FnMut(&mut [f32]) + Send + 'static,
        E: FnMut(StreamError) + Send + 'static,
    {
        let config = cpal::StreamConfig {
            channels: format.channels,
            sample_rate: format.sample_rate,
            buffer_size: to_cpal_buffer(buffer),
        };
        let stream = self
            .0
            .build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| data(out),
                move |err: cpal::Error| {
                    let kind = if err.kind() == cpal::ErrorKind::StreamInvalidated {
                        StreamErrorKind::StreamInvalidated
                    } else {
                        StreamErrorKind::BackendError
                    };
                    error(StreamError {
                        kind,
                        message: err.message().map(str::to_owned),
                    });
                },
                None,
            )
            .map_err(|e| format!("Failed to build stream: {e}"))?;
        Ok(Stream(stream))
    }
}

impl Stream {
    pub(crate) fn play(&self) -> Result<(), String> {
        self.0.play().map_err(|e| e.to_string())
    }

    pub(crate) fn pause(&self) -> Result<(), String> {
        self.0.pause().map_err(|e| e.to_string())
    }
}

pub(crate) fn default_output_device() -> Option<Device> {
    cpal::default_host().default_output_device().map(Device)
}

pub(crate) fn find_output_device(name: Option<&str>) -> Result<Device, String> {
    let host = cpal::default_host();
    match name {
        Some(name) => host
            .output_devices()
            .map_err(|e| format!("Failed to enumerate output devices: {e}"))?
            .find(|d| {
                d.description()
                    .ok()
                    .is_some_and(|desc| desc.to_string() == name)
            })
            .map(Device)
            .ok_or_else(|| format!("Output device not found: {name}")),
        None => host
            .default_output_device()
            .map(Device)
            .ok_or_else(|| "No output device".to_string()),
    }
}

pub(crate) fn list_devices(output: bool) -> Vec<String> {
    let host = cpal::default_host();
    let devices = if output {
        host.output_devices()
    } else {
        host.input_devices()
    };
    devices
        .map(|devices| {
            devices
                .filter_map(|d| d.description().ok().map(|desc| desc.to_string()))
                .collect()
        })
        .unwrap_or_default()
}
