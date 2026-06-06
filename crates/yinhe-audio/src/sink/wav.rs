use hound::{SampleFormat, WavSpec, WavWriter};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use crate::engine::AudioEngine;

use super::AudioSink;

pub struct WavSink {
    writer: Option<WavWriter<BufWriter<File>>>,
}

impl WavSink {
    pub fn new(path: &Path, sample_rate: u32, channels: u16) -> Result<Self, String> {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        let writer = WavWriter::create(path, spec)
            .map_err(|e| format!("Failed to create WAV file: {}", e))?;

        Ok(Self {
            writer: Some(writer),
        })
    }

    pub fn export(engine: &mut AudioEngine, path: &Path) -> Result<(), String> {
        let mut sink = Self::new(path, engine.sample_rate(), 2)?;
        let total = engine.duration_samples();
        let buf_size = engine.sample_rate() as usize;
        let mut buf = vec![0.0f32; buf_size * 2];

        engine.reset();
        while engine.sample_position() < total {
            let remaining = total - engine.sample_position();
            let frames = buf_size.min(remaining as usize);
            engine.read_samples(&mut buf[..frames * 2]);
            sink.write(&buf[..frames * 2]);
        }
        sink.flush();
        Ok(())
    }
}

impl AudioSink for WavSink {
    fn write(&mut self, samples: &[f32]) {
        if let Some(ref mut writer) = self.writer {
            for &s in samples {
                let sample = (s * 32767.0).clamp(-32768.0, 32767.0) as i16;
                let _ = writer.write_sample(sample);
            }
        }
    }

    fn flush(&mut self) {
        if let Some(writer) = self.writer.take() {
            let _ = writer.finalize();
        }
    }
}
