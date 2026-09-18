//! SoundFont 解析器 — 统一支持 SFZ 和 SF2 格式。
//!
//! 委托 xsynth-soundfonts 解析 SFZ/SF2，按 (key, vel) 展开为**最终合成参数快照**，
//! 公式与 xsynth `SampleSoundfont` 的 spawner 构建逻辑完全对齐
//! （音量曲线、声像、滤波器、包络、vel2release 修正全部在 build 时算好）。
//! `note_on` 时零公式计算，直接消费快照字段。
//!
//! 采样数据统一重采样到目标采样率后以 `Arc<[f32]>` 共享（SFZ/SF2 同一路径），
//! offset/loop 索引同步换算到目标采样率，消除双重重采样。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use xsynth_soundfonts::FilterType;

/// Butterworth Q 值（与 xsynth 一致，来自 biquad crate）。
const Q_BUTTERWORTH: f32 = std::f32::consts::FRAC_1_SQRT_2;

mod sf2;
mod sfz;

/// 滤波器类型 → 渲染器编码（WGSL/CPU voice 共用；shader `filter_type` 字段）。
pub(crate) fn filter_type_code(ft: FilterType) -> u32 {
    match ft {
        FilterType::LowPass => 0,
        FilterType::HighPass => 1,
        FilterType::BandPass => 2,
        FilterType::LowPassPole => 3,
    }
}

/// 每个 (MIDI key, velocity) 对应的最终合成参数快照。
///
/// 展开语义与 xsynth `SampleVoiceSpawnerParams` 对齐：
/// - SFZ 的 `amp_veltrack` 二次曲线、`amp_keytrack`、`pan_veltrack/keytrack`、
///   `fil_veltrack/keytrack`、`ampeg_vel2release` 均已折算进各字段
/// - SF2 的 `note_params`（modulator 系统）在 build 时按 (key, vel) 展开
#[derive(Clone, Debug)]
pub struct KeyInfo {
    /// 重采样到目标采样率后的采样数据（Arc 共享，clone 零拷贝）。
    /// 立体声样本为 LRLR 交错存储，`is_stereo` 标记布局。
    pub sample_data: Arc<[f32]>,
    pub sample_rate: u32,
    /// 采样是否为交错立体声（false = 单声道）。
    pub is_stereo: bool,
    /// 插值器：0=Nearest, 1=Linear（默认 0，与 xsynth `SoundfontInitOptions` 默认一致）。
    pub interp: u32,

    /// 采样播放倍率（键位频率比 × 调音音分，等价 xsynth `get_speed_mult_from_keys` × `cents_factor`）。
    pub speed_mult: f32,
    /// 线性增益（含 vel 曲线与键位音量跟踪，等价 xsynth spawner 的 `volume`）。
    pub volume: f32,
    /// 声像 0..1（0=左, 0.5=中, 1=右，含 vel/key 修正，等价 xsynth spawner 的 `pan`）。
    pub pan: f32,
    /// 采样起始偏移（帧，已按目标采样率换算）。
    pub offset: u32,
    /// 采样结束位置（帧，已换算；SF2 的 sample_end，等价 xsynth LoopParams.stop；
    /// SFZ 无此概念为 None）。播放长度 = min(采样长度, stop) - offset。
    pub stop: Option<u32>,

    // ── ADSR 包络（秒；攻击/释放已有 0.001s 下限防除零）──
    pub ampeg_start: f32, // 0..1
    pub ampeg_delay: f32,
    pub ampeg_attack: f32,
    pub ampeg_hold: f32,
    pub ampeg_decay: f32,
    pub ampeg_sustain: f32, // 0..1
    pub ampeg_release: f32, // 已含 ampeg_vel2release 修正

    // ── 力度分层（展开后为精确单值）──
    pub lovel: u8,
    pub hivel: u8,

    // ── 循环 ──
    pub loop_mode: LoopMode,
    pub loop_start: u32, // 帧，已按目标采样率换算
    pub loop_end: u32,

    // ── 滤波器（cutoff=0 表示无滤波器，与 xsynth `use_effects` 一致）──
    pub cutoff: f32,    // Hz，已含 fil_veltrack/keytrack 修正并 clamp
    pub resonance: f32, // 线性（db_to_amp(dB) × Q_BUTTERWORTH）
    pub filter_type: FilterType,

    // ── 加载期烘焙（运行期每 note_on 零三角函数）──
    /// per-voice biquad 系数 `[b0,b1,b2,a1,a2]`（cutoff<=0 时 None）。
    pub biquad: Option<[f32; 5]>,
    /// 等功率声像增益（`pan` 的 cos/sin 预计算，与 CpuVoice::new 原公式一致）。
    pub pan_l: f32,
    pub pan_r: f32,
}

/// 加载期烘焙：等功率声像增益（`CpuVoice::new` 原公式的预计算）。
fn pan_gains(pan: f32) -> (f32, f32) {
    let angle = pan * std::f32::consts::FRAC_PI_2;
    ((angle.cos() * 1.42).min(1.0), (angle.sin() * 1.42).min(1.0))
}

/// 加载期烘焙：biquad 系数（cutoff<=0 → None，与 `CpuVoice::new` 一致）。
fn bake_biquad(
    cutoff: f32,
    resonance: f32,
    filter_type: FilterType,
    sample_rate: u32,
) -> Option<[f32; 5]> {
    (cutoff > 0.0).then(|| {
        let (b0, b1, b2, a1, a2) = crate::synth::biquad_coeffs(
            filter_type_code(filter_type),
            cutoff,
            resonance,
            sample_rate as f32,
        );
        [b0, b1, b2, a1, a2]
    })
}

/// 采样循环模式
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopMode {
    NoLoop,
    LoopContinuous,
    LoopSustain,
    OneShot,
}

impl Default for KeyInfo {
    fn default() -> Self {
        Self {
            sample_data: Arc::from([]),
            sample_rate: 0,
            is_stereo: false,
            interp: 0,
            speed_mult: 1.0,
            volume: 1.0,
            pan: 0.5,
            offset: 0,
            stop: None,
            ampeg_start: 0.0,
            ampeg_delay: 0.0,
            // 默认对齐 xsynth（`AmpegEnvelopeParams::default`）：attack/release
            // 10ms 而非 SFZ 规范的 1ms——音色库不写 ampeg_release 时（如 Starry
            // Studio Grand），1ms 包络让每个音符结束都像硬切（密集音符连成
            // click），xsynth 的 10ms 听感自然。
            ampeg_attack: 0.01,
            ampeg_hold: 0.0,
            ampeg_decay: 0.001,
            ampeg_sustain: 1.0,
            ampeg_release: 0.01,
            lovel: 0,
            hivel: 127,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            cutoff: 0.0,
            resonance: Q_BUTTERWORTH,
            filter_type: FilterType::default(),
            biquad: None,
            pan_l: 1.0,
            pan_r: 1.0,
        }
    }
}

/// 一个音色库文件中的一个（bank, preset）条目及其 key map。
///
/// 语义与 xsynth `SoundfontInstrument` 对齐：SFZ 无 preset 概念，整个文件是
/// (0, 0) 一个条目（等价 xsynth `SoundfontInitOptions` 默认）；SF2 每个 preset
/// 一个条目。program（bank, preset）选择条目，选不到则静音。
#[derive(Clone, Debug)]
pub struct KeyMapEntry {
    pub bank: u8,
    pub preset: u8,
    /// (key, vel) 展开后的参数快照（与 xsynth `spawner_params_list` 等价）。
    pub map: Vec<Vec<KeyInfo>>,
}

/// 根据文件扩展名自动检测格式并构建 key map 条目列表。
/// `sample_rate` 为目标采样率：SFZ 的 wav 与 SF2 都在此采样率下加载/重采样一次。
pub fn build_key_maps(
    path: &Path,
    sample_rate: u32,
    interp: u32,
) -> Result<Vec<KeyMapEntry>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("sfz") => Ok(vec![KeyMapEntry {
            bank: 0,
            preset: 0,
            map: sfz::build_key_map_from_sfz(path, sample_rate, interp)?,
        }]),
        Some("sf2") => sf2::build_key_maps_from_sf2(path, sample_rate, interp),
        _ => Err(format!("Unsupported soundfont format: {:?}", path)),
    }
}

/// 多音色库 + program 选择（与 xsynth `ChannelSoundfont::rebuild_matrix` 一致）：
/// 1. 主选：按文件顺序找 (bank, preset) 匹配且该 (key, vel) 有 region 的条目；
/// 2. 兜底：全部主选落空后，鼓组（bank==128）找 (128, 0)，旋律找 (0, preset)；
/// 3. 都落空 → 静音（与 xsynth 缺失 preset 静音一致）。
pub fn select_key_info_multi(
    entries: &[KeyMapEntry],
    bank: u8,
    preset: u8,
    key: u8,
    velocity: u8,
) -> Option<&KeyInfo> {
    for e in entries {
        if e.bank == bank
            && e.preset == preset
            && let Some(info) = select_key_info(&e.map, key, velocity)
        {
            return Some(info);
        }
    }
    let (rb, rp) = if bank == 128 { (128, 0) } else { (0, preset) };
    for e in entries {
        if e.bank == rb
            && e.preset == rp
            && let Some(info) = select_key_info(&e.map, key, velocity)
        {
            return Some(info);
        }
    }
    None
}

/// 根据 key 和 velocity 选择对应的 KeyInfo（力度分层）。
/// 展开后每个 vel 恰好一层，正常情况精确命中；兜底选距离最近的层。
///
/// key ≥ 128（256 键扩展音域）时音色库无对应区域：返回 None（静音），不越界。
pub fn select_key_info(key_map: &[Vec<KeyInfo>], key: u8, velocity: u8) -> Option<&KeyInfo> {
    let layers = key_map.get(key as usize)?;
    if layers.is_empty() {
        return None;
    }
    for info in layers {
        if velocity >= info.lovel && velocity <= info.hivel {
            return Some(info);
        }
    }
    layers.iter().min_by_key(|info| {
        (velocity as i16 - info.lovel as i16)
            .unsigned_abs()
            .min((velocity as i16 - info.hivel as i16).unsigned_abs())
    })
}

// ── SFZ ──

// ── 工具函数 ──

fn convert_loop_mode(mode: xsynth_soundfonts::LoopMode) -> LoopMode {
    match mode {
        xsynth_soundfonts::LoopMode::NoLoop => LoopMode::NoLoop,
        xsynth_soundfonts::LoopMode::LoopContinuous => LoopMode::LoopContinuous,
        xsynth_soundfonts::LoopMode::LoopSustain => LoopMode::LoopSustain,
        xsynth_soundfonts::LoopMode::OneShot => LoopMode::OneShot,
    }
}

/// 对交错立体声数据重采样：先按声道解交错，逐声道重采样后再交错。
fn resample_interleaved(raw: Vec<f32>, src_sr: u32, dst_sr: u32) -> Arc<[f32]> {
    let left: Vec<f32> = raw.iter().step_by(2).copied().collect();
    let right: Vec<f32> = raw.iter().skip(1).step_by(2).copied().collect();
    let left = xsynth_soundfonts::resample::resample_vec(left, src_sr as f32, dst_sr as f32);
    let right = xsynth_soundfonts::resample::resample_vec(right, src_sr as f32, dst_sr as f32);
    let len = left.len().min(right.len());
    let mut out = Vec::with_capacity(len * 2);
    for i in 0..len {
        out.push(left[i]);
        out.push(right[i]);
    }
    Arc::from(out)
}

/// Load a WAV file as f32 samples.
/// 立体声返回 LRLR 交错数据（不再平均成单声道），单声道返回原样。
/// 返回 (samples, sample_rate, is_stereo)。
pub fn load_wav_as_f32(path: &Path) -> Result<(Vec<f32>, u32, bool), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open WAV {:?}: {}", path, e))?;

    let spec = reader.spec();
    // 读取样本必须显式处理 Err：损坏/截断的 WAV 会令 hound 返回错误，
    // 若 unwrap 则 GPU 加载音色库时直接 panic（release=abort）闪退。
    let samples: Vec<f32> = match spec.bits_per_sample {
        16 => reader
            .samples::<i16>()
            .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 16-bit samples from {:?}: {}", path, e))?,
        24 => reader
            .samples::<i32>()
            .map(|s| s.map(|v| (v >> 8) as f32 / (i16::MAX as f32)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 24-bit samples from {:?}: {}", path, e))?,
        32 => reader
            .samples::<i32>()
            .map(|s| s.map(|v| v as f32 / i32::MAX as f32))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read 32-bit samples from {:?}: {}", path, e))?,
        _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
    };

    // 声道数：1=单声道，2=立体声（已是 LRLR 交错），>2 取前两声道
    match spec.channels {
        1 => Ok((samples, spec.sample_rate, false)),
        2 => Ok((samples, spec.sample_rate, true)),
        n => {
            let stereo: Vec<f32> = samples
                .chunks(n as usize)
                .flat_map(|ch| [ch[0], ch[1]])
                .collect();
            Ok((stereo, spec.sample_rate, true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归：单声道 wav 且源采样率 != 目标采样率时，重采样必须按单声道序列
    /// 处理（曾走交错路径，样本被按奇偶拆成两个"半速声道"，波形错乱）。
    #[test]
    fn mono_wav_resampled_as_mono() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");

        let src_sr = 44_100u32;
        let len = 4096usize;
        let samples: Vec<f32> = (0..len)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / src_sr as f32).sin() * 0.5)
            .collect();
        let mut writer = hound::WavWriter::create(
            &wav_path,
            hound::WavSpec {
                channels: 1,
                sample_rate: src_sr,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .expect("wav create");
        for &s in &samples {
            writer
                .write_sample((s * i16::MAX as f32) as i16)
                .expect("wav write");
        }
        writer.finalize().expect("wav finalize");
        std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").expect("sfz write");

        let dst_sr = 48_000u32;
        let entries = build_key_maps(&sfz_path, dst_sr, 0).expect("build key maps");
        let info = &entries[0].map[60][0];
        assert!(!info.is_stereo, "单声道样本必须标记为非立体声");
        assert_eq!(info.sample_rate, dst_sr);

        // 与单声道重采样参考逐样本一致（交错路径的输出会明显不同）
        let (raw, _, is_stereo) = load_wav_as_f32(&wav_path).expect("load wav");
        assert!(!is_stereo);
        let expected = xsynth_soundfonts::resample::resample_vec(raw, src_sr as f32, dst_sr as f32);
        assert_eq!(info.sample_data.len(), expected.len());
        for (i, (a, b)) in info.sample_data.iter().zip(expected.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "sample {i}: {a} vs {b}");
        }
    }
}
