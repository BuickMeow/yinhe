//! 音频录音：cpal 输入流采集 → 停止后生成音频片段。
//!
//! 与 MIDI 录音共用 REC 按钮入口（main_loop 同一分支启动/停止）。
//! 输入样本在回调里降混为交错立体声存入共享缓冲；停止时重采样到引擎
//! 采样率、编码 16-bit WAV 内嵌工程，并直接把 PCM 放进素材库（免二次解码）。
//!
//! 监听：录音期间把输入写进引擎的监听缓冲（输出回调混入），开关在设置里。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cpal::FromSample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rust_i18n::t;

use yinhe_audio::DecodedAudio;

use crate::app::App;

/// 监听缓冲上限（交错立体声样本数，约 0.37s@44.1k）。
const MONITOR_MAX_SAMPLES: usize = 16_384 * 2;

/// 进行中的音频录音（存在 = 录音中）。
pub(crate) struct AudioRecording {
    /// 输入流；drop = 停止采集。
    _stream: cpal::Stream,
    /// 输入原始交错样本（已转 f32）。
    samples: Arc<Mutex<Vec<f32>>>,
    /// 输入声道数。
    input_channels: usize,
    /// 输入采样率。
    input_sample_rate: u32,
    /// 录音片段的起点（歌曲时间秒，已扣延迟补偿）。
    start_secs: f64,
    /// 监听开关（drop 时置 false）。
    monitor_enabled: Arc<AtomicBool>,
}

impl Drop for AudioRecording {
    fn drop(&mut self) {
        self.monitor_enabled.store(false, Ordering::Release);
    }
}

impl App {
    /// 开始音频录音（由 REC 按钮与 `start_recording` 同时触发）。
    /// 失败时弹错误通知并保持未录音状态。
    pub(crate) fn start_audio_recording(&mut self) {
        if self.audio_recording.is_some() {
            return;
        }
        let Some(audio) = self.audio_state.handle.as_ref() else {
            self.show_error(
                t!("toast.record_failed").to_string(),
                t!("record.no_engine").to_string(),
            );
            return;
        };
        let engine_sr = audio.sample_rate;
        let monitor_buf = Arc::clone(&audio.monitor);

        // 输入设备：设置里指定，否则系统默认。
        let host = cpal::default_host();
        let device = match &self.audio_settings.input_device_name {
            Some(name) => host.input_devices().ok().and_then(|mut it| {
                it.find(|d| {
                    d.description()
                        .ok()
                        .map(|desc| desc.to_string() == *name)
                        .unwrap_or(false)
                })
            }),
            None => host.default_input_device(),
        };
        let Some(device) = device else {
            self.show_error(
                t!("toast.record_failed").to_string(),
                t!("record.no_input_device").to_string(),
            );
            return;
        };

        // 优先引擎采样率（免重采样）；不支持时退回设备默认并事后重采样。
        let config = device
            .supported_input_configs()
            .ok()
            .and_then(|mut cfgs| {
                cfgs.find(|c| c.min_sample_rate() <= engine_sr && engine_sr <= c.max_sample_rate())
                    .map(|c| c.with_sample_rate(engine_sr))
            })
            .or_else(|| device.default_input_config().ok());
        let Some(config) = config else {
            self.show_error(
                t!("toast.record_failed").to_string(),
                t!("record.no_input_config").to_string(),
            );
            return;
        };
        let input_sample_rate = config.sample_rate();
        let input_channels = config.channels() as usize;

        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let monitor_enabled = Arc::new(AtomicBool::new(self.audio_settings.record_monitor));
        let stream = match build_input_stream(
            &device,
            &config,
            Arc::clone(&samples),
            monitor_buf,
            Arc::clone(&monitor_enabled),
        ) {
            Ok(s) => s,
            Err(e) => {
                self.show_error(t!("toast.record_failed").to_string(), e);
                return;
            }
        };
        if let Err(e) = stream.play() {
            self.show_error(
                t!("toast.record_failed").to_string(),
                format!("{}: {e}", t!("record.start_failed")),
            );
            return;
        }

        // 起点 = 光标位置（与 MIDI 录音同基点）− 延迟补偿。
        let cursor_tick = self.last_cursor_tick.unwrap_or(0.0).max(0.0);
        let cursor_secs = self
            .workspace
            .active_doc
            .map(|idx| {
                self.workspace.documents[idx]
                    .data
                    .model
                    .tempo_map
                    .tick_to_seconds(cursor_tick as u64)
            })
            .unwrap_or(0.0);
        let offset = (self.audio_settings.record_offset_ms as f64 / 1000.0).max(0.0);
        let start_secs = (cursor_secs - offset).max(0.0);

        self.audio_recording = Some(AudioRecording {
            _stream: stream,
            samples,
            input_channels,
            input_sample_rate,
            start_secs,
            monitor_enabled,
        });
    }

    /// 停止音频录音：重采样 + 编码 WAV + 插入片段（与导入共用落地路径）。
    pub(crate) fn stop_audio_recording(&mut self) {
        let Some(rec) = self.audio_recording.take() else {
            return;
        };
        let raw = rec
            .samples
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        if raw.is_empty() {
            return;
        }
        let channels = rec.input_channels.max(1);
        let frames = raw.len() / channels;
        if frames == 0 {
            return;
        }
        // 降混交错立体声。
        let mut left: Vec<f32> = Vec::with_capacity(frames);
        let mut right: Vec<f32> = Vec::with_capacity(frames);
        for f in 0..frames {
            let base = f * channels;
            let l = raw[base];
            let r = if channels >= 2 { raw[base + 1] } else { l };
            left.push(l);
            right.push(r);
        }
        // 重采样到引擎采样率。
        let engine_sr = self
            .audio_state
            .handle
            .as_ref()
            .map(|a| a.sample_rate)
            .unwrap_or(self.audio_settings.sample_rate);
        let (mut left, mut right) = if rec.input_sample_rate == engine_sr {
            (left, right)
        } else {
            let l = match yinhe_audio::resample_channel(&left, rec.input_sample_rate, engine_sr) {
                Ok(v) => v,
                Err(e) => {
                    self.show_error(t!("toast.record_failed").to_string(), e);
                    return;
                }
            };
            let r = match yinhe_audio::resample_channel(&right, rec.input_sample_rate, engine_sr) {
                Ok(v) => v,
                Err(e) => {
                    self.show_error(t!("toast.record_failed").to_string(), e);
                    return;
                }
            };
            (l, r)
        };
        let frames = left.len().min(right.len());
        left.truncate(frames);
        right.truncate(frames);
        if frames == 0 {
            return;
        }
        let duration_seconds = frames as f64 / engine_sr as f64;
        let wav = match yinhe_audio::encode_wav_bytes(&left, &right, engine_sr) {
            Ok(w) => w,
            Err(e) => {
                self.show_error(t!("toast.record_failed").to_string(), e);
                return;
            }
        };

        // 直接构造 decoded 入素材库（免二次解码），并推引擎。
        let peaks = yinhe_audio::WavePeaks::build(&left, &right);
        let decoded = Arc::new(DecodedAudio {
            sample_rate: engine_sr,
            frames,
            left: Arc::from(left),
            right: Arc::from(right),
            peaks,
        });
        let uuid = uuid::Uuid::new_v4().to_string();
        let name = t!("record.clip_name").to_string();
        self.audio_library
            .insert_decoded(uuid.clone(), Arc::clone(&decoded));
        if let Some(audio) = &self.audio_state.handle {
            self.audio_library
                .push_to_engine(&audio.handle, uuid.clone(), decoded);
        }

        // 插入模型（起点用录音起点，而非光标）。
        let wav = Arc::new(wav);
        self.insert_imported_audio_at(None, &[(name, wav, duration_seconds)], Some(rec.start_secs));
    }

    /// 每帧轮询：录音中但引擎已消失（设备切换/关闭工程）→ 停止录音。
    pub(crate) fn poll_audio_recording(&mut self) {
        if self.audio_recording.is_some() && self.audio_state.handle.is_none() {
            self.stop_audio_recording();
        }
    }
}

/// 按设备支持的采样格式构建输入流；样本转 f32 存共享缓冲 + 监听缓冲。
fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    samples: Arc<Mutex<Vec<f32>>>,
    monitor: Arc<Mutex<VecDeque<f32>>>,
    monitor_enabled: Arc<AtomicBool>,
) -> Result<cpal::Stream, String> {
    let stream_config: cpal::StreamConfig = (*config).into();
    let channels = config.channels() as usize;
    let err_fn = |e| tracing::error!("音频输入流错误: {e}");

    macro_rules! build {
        ($fmt:ty) => {{
            let samples = Arc::clone(&samples);
            let monitor = Arc::clone(&monitor);
            let monitor_enabled = Arc::clone(&monitor_enabled);
            device
                .build_input_stream(
                    stream_config.clone(),
                    move |data: &[$fmt], _: &cpal::InputCallbackInfo| {
                        push_samples(data, channels, &samples, &monitor, &monitor_enabled)
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| format!("无法创建输入流: {e}"))
        }};
    }

    match config.sample_format() {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::F64 => build!(f64),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        other => Err(format!("不支持的输入采样格式: {other:?}")),
    }
}

/// 输入回调：原始样本 → f32 录音缓冲；监听开启时降混立体声推监听缓冲。
fn push_samples<T>(
    data: &[T],
    channels: usize,
    samples: &Arc<Mutex<Vec<f32>>>,
    monitor: &Arc<Mutex<VecDeque<f32>>>,
    monitor_enabled: &Arc<AtomicBool>,
) where
    T: cpal::Sample + cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    if let Ok(mut buf) = samples.lock() {
        buf.extend(data.iter().map(|s| f32::from_sample_(*s)));
    }
    if monitor_enabled.load(Ordering::Relaxed)
        && let Ok(mut mon) = monitor.try_lock()
    {
        let ch = channels.max(1);
        for frame in data.chunks(ch) {
            let l = f32::from_sample_(frame[0]);
            let r = if frame.len() >= 2 {
                f32::from_sample_(frame[1])
            } else {
                l
            };
            mon.push_back(l);
            mon.push_back(r);
        }
        while mon.len() > MONITOR_MAX_SAMPLES {
            mon.pop_front();
        }
    }
}
