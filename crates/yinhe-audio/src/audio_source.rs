//! 音频素材解码（symphonia）与波形峰值金字塔。
//!
//! 解码在 worker/后台线程执行；产物 `DecodedAudio` 由 UI 与引擎经 `Arc` 共享：
//! - 引擎按片段（AudioClip）读取 PCM 混音；
//! - UI 读取 `WavePeaks` 绘制波形（不重新扫描 PCM）。

use std::sync::Arc;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// 解码后的音频素材（立体声 planar，目标采样率 = 引擎采样率）。
pub struct DecodedAudio {
    /// 解码目标采样率（= 引擎采样率）。
    pub sample_rate: u32,
    /// 帧数（采样点数）。
    pub frames: usize,
    /// 左声道 PCM（长度 = frames）。
    pub left: Arc<[f32]>,
    /// 右声道 PCM（长度 = frames）。
    pub right: Arc<[f32]>,
    /// 波形峰值金字塔（绘制用）。
    pub peaks: WavePeaks,
}

impl DecodedAudio {
    /// 时长（秒）。
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.frames as f64 / self.sample_rate as f64
    }
}

/// 波形峰值金字塔：level 0 每 `base_bucket` 帧一个 (min, max)，
/// 之后每级桶宽翻倍。绘制时按"每像素多少帧"选级，避免扫全量 PCM。
pub struct WavePeaks {
    pub base_bucket: usize,
    /// `levels[l]` 的桶宽 = `base_bucket << l`。
    pub levels: Vec<Vec<(f32, f32)>>,
}

impl WavePeaks {
    /// 从立体声 PCM 构建金字塔（幅度取左右平均，保持视觉对称）。
    pub fn build(left: &[f32], right: &[f32]) -> Self {
        const BASE: usize = 256;
        let frames = left.len().max(right.len());
        if frames == 0 {
            return Self {
                base_bucket: BASE,
                levels: Vec::new(),
            };
        }
        let mut level0: Vec<(f32, f32)> = Vec::with_capacity(frames.div_ceil(BASE));
        for chunk in 0..frames.div_ceil(BASE) {
            let start = chunk * BASE;
            let end = ((chunk + 1) * BASE).min(frames);
            let mut lo = f32::INFINITY;
            let mut hi = f32::NEG_INFINITY;
            for i in start..end {
                let l = left.get(i).copied().unwrap_or(0.0);
                let r = right.get(i).copied().unwrap_or(l);
                let v = (l + r) * 0.5;
                lo = lo.min(v);
                hi = hi.max(v);
            }
            level0.push((lo, hi));
        }
        let mut levels = vec![level0];
        loop {
            let prev = levels.last().expect("levels 至少一层");
            if prev.len() <= 1 {
                break;
            }
            let mut next = Vec::with_capacity(prev.len().div_ceil(2));
            for pair in prev.chunks(2) {
                let lo = pair.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                let hi = pair.iter().map(|p| p.1).fold(f32::NEG_INFINITY, f32::max);
                next.push((lo, hi));
            }
            levels.push(next);
        }
        Self {
            base_bucket: BASE,
            levels,
        }
    }

    /// 每桶帧数（level `l`）。
    pub fn bucket_frames(&self, level: usize) -> usize {
        self.base_bucket << level.min(self.levels.len().saturating_sub(1))
    }

    /// 选择"每桶帧数 >= frames_per_bucket_wanted"的最细一级。
    pub fn level_for(&self, frames_per_bucket_wanted: f64) -> usize {
        if frames_per_bucket_wanted <= self.base_bucket as f64 || self.levels.is_empty() {
            return 0;
        }
        let ratio = frames_per_bucket_wanted / self.base_bucket as f64;
        let level = ratio.log2().ceil() as usize;
        level.min(self.levels.len() - 1)
    }

    /// 取某一级某桶的 (min, max)；越界返回 (0, 0)。
    pub fn bucket(&self, level: usize, index: usize) -> (f32, f32) {
        self.levels
            .get(level)
            .and_then(|l| l.get(index))
            .copied()
            .unwrap_or((0.0, 0.0))
    }
}

/// 解码音频字节到目标采样率立体声。
///
/// `data` 是内嵌原始文件字节（wav/mp3/flac/ogg/m4a 等，由 symphonia 探测）。
/// 返回 Err 时 UI 应提示用户该素材无法解码。整个解码在 worker/后台线程执行。
pub fn decode_audio(data: &[u8], target_sample_rate: u32) -> Result<DecodedAudio, String> {
    let hint = Hint::new();
    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(data.to_vec())),
        Default::default(),
    );
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("无法识别音频格式: {e}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "音频文件没有可用轨道".to_string())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("不支持的音频编码: {e}"))?;

    let mut left: Vec<f32> = Vec::new();
    let mut right: Vec<f32> = Vec::new();
    let mut source_rate: u32 = 0;
    let mut sample_buf: Option<SampleBuffer<f32>> = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(SymphoniaError::ResetRequired) => break,
            Err(e) => return Err(format!("读取音频失败: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break;
            }
            Err(e) => return Err(format!("解码音频失败: {e}")),
        };
        let spec = *decoded.spec();
        if source_rate == 0 {
            source_rate = spec.rate;
        }
        let channels = spec.channels.count().max(1);
        // 统一走 SampleBuffer 的交错输出（symphonia 官方推荐路径，覆盖所有采样格式）。
        let buf = sample_buf
            .get_or_insert_with(|| SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        // 容量不足（规格变化）时重建。
        if buf.capacity() < decoded.capacity() {
            *buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        }
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();
        let frames = samples.len() / channels;
        for f in 0..frames {
            let base = f * channels;
            let l = samples[base];
            let r = if channels >= 2 { samples[base + 1] } else { l };
            left.push(l);
            right.push(r);
        }
    }

    if source_rate == 0 || left.is_empty() {
        return Err("音频文件没有可解码的采样".to_string());
    }

    // 重采样到引擎采样率（大部分素材同率，直接跳过）。
    let (left, right) = if source_rate == target_sample_rate {
        (left, right)
    } else {
        let l = resample_channel(&left, source_rate, target_sample_rate)?;
        let r = resample_channel(&right, source_rate, target_sample_rate)?;
        (l, r)
    };
    let frames = left.len().min(right.len());
    let peaks = WavePeaks::build(&left, &right);
    Ok(DecodedAudio {
        sample_rate: target_sample_rate,
        frames,
        left: Arc::from(left),
        right: Arc::from(right),
        peaks,
    })
}

/// 音频文件基本信息（导入时探测，不解码 PCM）。
pub struct AudioInfo {
    /// 时长（秒）。
    pub duration_seconds: f64,
    /// 原始采样率（0 = 未知）。
    pub sample_rate: u32,
    /// 声道数。
    pub channels: u16,
}

/// 探测音频文件信息（时长/采样率/声道数）。
///
/// 优先用 codec 声明的总帧数；没有时遍历 packet 头累加时间戳（不解码 payload）。
/// 导入时同步调用（几十毫秒内），完整解码走后台线程。
pub fn probe_audio_info(data: &[u8]) -> Result<AudioInfo, String> {
    let hint = Hint::new();
    let mss = MediaSourceStream::new(
        Box::new(std::io::Cursor::new(data.to_vec())),
        Default::default(),
    );
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions {
                enable_gapless: true,
                ..Default::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("无法识别音频格式: {e}"))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| "音频文件没有可用轨道".to_string())?;
    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(0);
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(2);

    let duration_seconds = match (track.codec_params.n_frames, sample_rate) {
        (Some(n_frames), sr) if sr > 0 => n_frames as f64 / sr as f64,
        _ => {
            // 无总帧数：遍历 packet 头，取时间戳 + 时长的最大值。
            let time_base = track.codec_params.time_base;
            let mut max_end = 0.0f64;
            loop {
                let packet = match format.next_packet() {
                    Ok(p) => p,
                    Err(SymphoniaError::IoError(e))
                        if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        break;
                    }
                    Err(_) => break,
                };
                if packet.track_id() != track_id {
                    continue;
                }
                if let Some(tb) = time_base {
                    let t = tb.calc_time(packet.ts().saturating_add(packet.dur()));
                    max_end = max_end.max(t.seconds as f64 + t.frac);
                }
            }
            if sample_rate > 0 && max_end <= 0.0 {
                return Err("音频文件没有可用的时长信息".to_string());
            }
            max_end
        }
    };

    Ok(AudioInfo {
        duration_seconds,
        sample_rate,
        channels,
    })
}

/// 单声道分块重采样（rubato SincFixedIn）。
///
/// 分块处理避免整段一次性缓冲（长音频内存翻数倍）；最后一块不足一个 chunk
/// 时用 `process_partial` 补零收尾。
fn resample_channel(input: &[f32], src_rate: u32, dst_rate: u32) -> Result<Vec<f32>, String> {
    use rubato::{
        Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
    };

    if input.is_empty() {
        return Ok(Vec::new());
    }
    const CHUNK_FRAMES: usize = 1 << 18;
    let params = SincInterpolationParameters {
        sinc_len: 32,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 128,
        window: WindowFunction::BlackmanHarris2,
    };
    let ratio = dst_rate as f64 / src_rate as f64;
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, params, CHUNK_FRAMES, 1)
        .map_err(|e| format!("重采样初始化失败: {e}"))?;
    // 期望输出帧数（比例精确已知）；rubato 的 process_partial 会补零整块，
    // 尾部多出的补零帧数与重采样器内部延迟帧都要按期望长度裁掉/补齐。
    let expected = (input.len() as f64 * ratio).round() as usize;
    let mut out: Vec<f32> = Vec::with_capacity(expected + 1024);
    let mut pos = 0usize;
    while pos < input.len() {
        let end = (pos + CHUNK_FRAMES).min(input.len());
        let chunk = &input[pos..end];
        let produced = if chunk.len() < CHUNK_FRAMES {
            resampler.process_partial(Some(&[chunk]), None)
        } else {
            resampler.process(&[chunk], None)
        }
        .map_err(|e| format!("重采样失败: {e}"))?;
        if let Some(ch) = produced.first() {
            out.extend_from_slice(ch);
        }
        pos = end;
    }
    out.truncate(expected);
    if out.len() < expected {
        out.resize(expected, 0.0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_build_and_levels_shrink() {
        let left: Vec<f32> = (0..1000)
            .map(|i| (i as f32 * std::f32::consts::TAU / 256.0).sin())
            .collect();
        let right = left.clone();
        let peaks = WavePeaks::build(&left, &right);
        assert_eq!(peaks.base_bucket, 256);
        assert_eq!(peaks.levels[0].len(), 4);
        // 逐级减半到 1。
        assert_eq!(peaks.levels.len(), 3);
        assert_eq!(peaks.levels[2].len(), 1);
        // 全局 min/max 应覆盖正弦范围。
        let (lo, hi) = peaks.levels[2][0];
        assert!(lo < -0.9 && hi > 0.9, "lo={lo} hi={hi}");
    }

    #[test]
    fn peaks_empty_is_safe() {
        let peaks = WavePeaks::build(&[], &[]);
        assert!(peaks.levels.is_empty());
        assert_eq!(peaks.bucket(0, 0), (0.0, 0.0));
    }

    /// 生成 0.5 秒 44.1k 单声道正弦 WAV，解码后应得到约 0.5 秒立体声。
    #[test]
    fn decode_wav_roundtrip() {
        let sr = 44_100u32;
        let frames = sr / 2;
        let mut wav: Vec<u8> = Vec::new();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut wav), spec).unwrap();
            for i in 0..frames {
                let v = (i as f32 / sr as f32 * 440.0 * std::f32::consts::TAU).sin();
                writer.write_sample((v * 32767.0) as i16).unwrap();
            }
            writer.finalize().unwrap();
        }
        let decoded = decode_audio(&wav, sr).expect("解码应成功");
        assert_eq!(decoded.sample_rate, sr);
        assert!((decoded.frames as i64 - frames as i64).abs() < 64);
        // 单声道降混后左右应一致。
        assert_eq!(decoded.left[100], decoded.right[100]);
        assert!(!decoded.peaks.levels.is_empty());
    }

    /// 采样率不一致时走重采样路径，输出长度按比例变化。
    #[test]
    fn decode_resamples_to_target() {
        let sr = 22_050u32;
        let frames = sr / 2;
        let mut wav: Vec<u8> = Vec::new();
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: sr,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        {
            let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut wav), spec).unwrap();
            for i in 0..frames {
                let v =
                    ((i as f32 / sr as f32 * 220.0 * std::f32::consts::TAU).sin() * 20000.0) as i16;
                writer.write_sample(v).unwrap();
                writer.write_sample(-v).unwrap();
            }
            writer.finalize().unwrap();
        }
        let decoded = decode_audio(&wav, 44_100).expect("解码应成功");
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(
            (decoded.frames as i64 - frames as i64 * 2).abs() < 256,
            "期望约 {} 帧, got {}",
            frames * 2,
            decoded.frames
        );
    }
}
