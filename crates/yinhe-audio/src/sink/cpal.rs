use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

pub struct CpalSink {
    _stream: cpal::Stream,
}

impl CpalSink {
    pub fn new(
        sample_rate: u32,
        _sample_position: Arc<AtomicU64>,
        playing: Arc<AtomicBool>,
        render_callback: Arc<std::sync::Mutex<dyn FnMut(&mut [f32]) + Send>>,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No output device found".to_string())?;

        let supported = device
            .default_output_config()
            .map_err(|e| format!("Failed to get output config: {}", e))?;

        let channels = supported.channels() as usize;

        let config = cpal::StreamConfig {
            channels: channels as u16,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let err_fn = |err: cpal::StreamError| {
            eprintln!("Audio stream error: {}", err);
        };

        let callback = Arc::clone(&render_callback);

        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    if !playing.load(Ordering::Relaxed) {
                        data.fill(0.0);
                        return;
                    }
                    if let Ok(mut render) = callback.lock() {
                        render(data);
                    } else {
                        data.fill(0.0);
                    }
                },
                err_fn,
                None,
            )
            .map_err(|e| format!("Failed to build stream: {}", e))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {}", e))?;

        Ok(Self { _stream: stream })
    }
}
