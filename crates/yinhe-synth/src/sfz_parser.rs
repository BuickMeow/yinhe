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
            ampeg_attack: 0.001,
            ampeg_hold: 0.0,
            ampeg_decay: 0.001,
            ampeg_sustain: 1.0,
            ampeg_release: 0.001,
            lovel: 0,
            hivel: 127,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            cutoff: 0.0,
            resonance: Q_BUTTERWORTH,
            filter_type: FilterType::default(),
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
pub fn build_key_maps(path: &Path, sample_rate: u32) -> Result<Vec<KeyMapEntry>, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("sfz") => Ok(vec![KeyMapEntry {
            bank: 0,
            preset: 0,
            map: build_key_map_from_sfz(path, sample_rate)?,
        }]),
        Some("sf2") => build_key_maps_from_sf2(path, sample_rate),
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
pub fn select_key_info(key_map: &[Vec<KeyInfo>], key: u8, velocity: u8) -> Option<&KeyInfo> {
    let layers = &key_map[key as usize];
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

fn build_key_map_from_sfz(sfz_path: &Path, sample_rate: u32) -> Result<Vec<Vec<KeyInfo>>, String> {
    let regions = xsynth_soundfonts::sfz::parse_soundfont(sfz_path)
        .map_err(|e| format!("SFZ parse error: {}", e))?;

    let mut key_map: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];

    // wav 按路径去重加载并重采样到目标采样率（同一文件被多个 region 引用）
    let mut wav_cache: HashMap<PathBuf, (Arc<[f32]>, u32, bool)> = HashMap::new();

    for region in &regions {
        // 采样加载失败只跳过该 region（损坏/缺失的 wav 不应拖垮整个音色库）
        let (samples, src_sr, is_stereo) = match wav_cache.get(&region.sample_path) {
            Some(entry) => entry.clone(),
            None => {
                let Ok((raw, src_sr, raw_stereo)) = load_wav_as_f32(&region.sample_path) else {
                    eprintln!(
                        "[yinhe-synth] Warning: failed to load {:?}",
                        region.sample_path
                    );
                    continue;
                };
                let out = if src_sr == sample_rate {
                    Arc::<[f32]>::from(raw)
                } else {
                    // 重采样前必须分声道解交错：resample_vec 是单声道序列插值，
                    // 直接对 LRLR 交错数据重采样会把左右声道混在一起（波形错乱）
                    resample_interleaved(raw, src_sr, sample_rate)
                };
                wav_cache.insert(
                    region.sample_path.clone(),
                    (out.clone(), src_sr, raw_stereo),
                );
                (out, src_sr, raw_stereo)
            }
        };

        // 采样率换算因子：offset/loop 索引从源采样率映射到目标采样率
        let factor = sample_rate as f32 / src_sr as f32;

        for key in region.keyrange.clone() {
            let key_f = key as f32;
            for vel in *region.velrange.start()..=*region.velrange.end() {
                let vel_f = vel as f32;

                // 播放倍率（xsynth: get_speed_mult_from_keys × cents_factor(tune)）
                let speed_mult = 2.0f32.powf((key_f - region.pitch_keycenter as f32) / 12.0)
                    * 2.0f32.powf(region.tune as f32 / 1200.0);

                // 音量（xsynth new_sfz 公式）：
                // vel 曲线 -> vol_vel 归一化后平方；键位音量跟踪加到 dB 再转线性
                let a = region.amp_veltrack / 100.0;
                let aabs = a.abs();
                let vol_vel = 127.0 * (1.0 - aabs)
                    + vel_f * (a + aabs) / 2.0
                    + (127.0 - vel_f) * (aabs - a) / 2.0;
                let vol_mult = (vol_vel / 127.0).powi(2);
                let vol_db = (region.volume as f32
                    + (key_f - region.amp_keycenter as f32) * region.amp_keytrack)
                    .clamp(-96.0, 12.0);
                let volume = vol_mult * 10.0f32.powf(vol_db / 20.0);

                // 声像（xsynth new_sfz 公式）：vel/key 修正后归一化到 0..1
                let pan_mult = vel_f / 127.0 * region.pan_veltrack
                    + (key_f - region.pan_keycenter as f32) * region.pan_keytrack;
                let pan = ((region.pan as f32 + pan_mult).clamp(-100.0, 100.0) / 100.0 + 1.0) / 2.0;

                // 滤波器截止频率（xsynth new_sfz 公式，cutoff >= 1.0 才启用）
                let mut cutoff = 0.0;
                if let Some(cutoff_t) = region.cutoff
                    && cutoff_t >= 1.0
                {
                    let cents = vel_f / 127.0 * region.fil_veltrack as f32
                        + (key_f - region.fil_keycenter as f32) * region.fil_keytrack as f32;
                    cutoff = (cutoff_t * 2.0f32.powf(cents / 1200.0))
                        .clamp(1.0, sample_rate as f32 / 2.0 - 100.0);
                }
                let resonance = 10.0f32.powf(region.resonance / 20.0) * Q_BUTTERWORTH;

                // 包络（xsynth: release 随力度加长；sustain/start 从 % 归一化）
                let ampeg = &region.ampeg_envelope;
                let release = ampeg.ampeg_release + (vel_f / 127.0) * ampeg.ampeg_vel2release;

                let loop_mode = if region.loop_start == region.loop_end {
                    LoopMode::NoLoop
                } else {
                    convert_loop_mode(region.loop_mode)
                };

                key_map[key as usize].push(KeyInfo {
                    sample_data: samples.clone(),
                    sample_rate,
                    is_stereo,
                    interp: 0,
                    speed_mult,
                    volume,
                    pan,
                    offset: (region.offset as f32 * factor) as u32,
                    stop: None,
                    ampeg_start: ampeg.ampeg_start / 100.0,
                    ampeg_delay: ampeg.ampeg_delay,
                    ampeg_attack: ampeg.ampeg_attack.max(0.001),
                    ampeg_hold: ampeg.ampeg_hold,
                    ampeg_decay: ampeg.ampeg_decay.max(0.001),
                    ampeg_sustain: (ampeg.ampeg_sustain / 100.0).clamp(0.0, 1.0),
                    ampeg_release: release.max(0.001),
                    lovel: vel,
                    hivel: vel,
                    loop_mode,
                    loop_start: (region.loop_start as f32 * factor) as u32,
                    loop_end: (region.loop_end as f32 * factor) as u32,
                    cutoff,
                    resonance,
                    filter_type: region.filter_type,
                });
            }
        }
    }

    for layers in key_map.iter_mut() {
        layers.sort_by_key(|info| info.lovel);
    }
    Ok(key_map)
}

// ── SF2 ──

fn build_key_maps_from_sf2(sf2_path: &Path, sample_rate: u32) -> Result<Vec<KeyMapEntry>, String> {
    // 直接以目标采样率加载，一次重采样到位（避免先转 44100 再转目标的双重重采样）
    let presets = xsynth_soundfonts::sf2::load_soundfont(sf2_path, sample_rate)
        .map_err(|e| format!("SF2 parse error: {}", e))?;
    if presets.is_empty() {
        return Err("SF2: no presets found".to_string());
    }

    // 每个 preset 一个条目（bank, preset）→ key map，program 切换直接索引。
    // preset 间采样数据 Arc 共享，上传 GPU 时按 Arc 身份去重，无重复拷贝。
    let mut entries = Vec::with_capacity(presets.len());
    for preset in &presets {
        let mut key_map: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];
        for region in &preset.regions {
            // 采样数据：单声道 Arc 共享零拷贝；立体声交错存储（左右声道各一个 Arc）
            let (sample_data, is_stereo): (Arc<[f32]>, bool) = if region.sample.len() == 2 {
                let left = &region.sample[0];
                let right = &region.sample[1];
                let len = left.len().min(right.len());
                let mut interleaved = Vec::with_capacity(len * 2);
                for i in 0..len {
                    interleaved.push(left[i]);
                    interleaved.push(right[i]);
                }
                (Arc::from(interleaved), true)
            } else if region.sample.len() == 1 {
                (Arc::clone(&region.sample[0]), false)
            } else {
                continue;
            };

            for key in region.keyrange.clone() {
                let key_f = key as f32;
                for vel in *region.velrange.start()..=*region.velrange.end() {
                    // note_params 展开 SF2 modulator 系统（vel 曲线、包络 keytrack 等）
                    let np = region.note_params(key, vel);
                    let ampeg = np.ampeg_envelope;

                    // 播放倍率（xsynth new_sf2: scale_tuning + fine/coarse tune + modulator）
                    let tuned_key_cents =
                        (key_f - region.root_key as f32) * region.scale_tuning as f32;
                    let speed_mult = 2.0f32.powf(
                        (tuned_key_cents
                            + region.fine_tune as f32
                            + region.coarse_tune as f32 * 100.0
                            + np.tune_cents)
                            / 1200.0,
                    );

                    let cutoff = np
                        .cutoff
                        .map(|c| c.clamp(1.0, sample_rate as f32 / 2.0 - 100.0))
                        .unwrap_or(0.0);
                    let pan = ((np.pan as f32 / 500.0) + 1.0) / 2.0;
                    let loop_mode = if region.loop_start == region.loop_end {
                        LoopMode::NoLoop
                    } else {
                        convert_loop_mode(region.loop_mode)
                    };

                    key_map[key as usize].push(KeyInfo {
                        sample_data: sample_data.clone(),
                        sample_rate,
                        is_stereo,
                        interp: 0,
                        speed_mult,
                        volume: np.volume,
                        pan,
                        offset: region.offset,
                        ampeg_start: ampeg.ampeg_start / 100.0,
                        ampeg_delay: ampeg.ampeg_delay,
                        ampeg_attack: ampeg.ampeg_attack.max(0.001),
                        ampeg_hold: ampeg.ampeg_hold,
                        ampeg_decay: ampeg.ampeg_decay.max(0.001),
                        ampeg_sustain: (ampeg.ampeg_sustain / 100.0).clamp(0.0, 1.0),
                        ampeg_release: ampeg.ampeg_release.max(0.001),
                        lovel: vel,
                        hivel: vel,
                        loop_mode,
                        loop_start: region.loop_start,
                        loop_end: region.loop_end,
                        stop: Some(region.sample_end),
                        cutoff,
                        resonance: 10.0f32.powf(np.resonance / 20.0) * Q_BUTTERWORTH,
                        filter_type: FilterType::LowPass,
                    });
                }
            }
        }

        for layers in key_map.iter_mut() {
            layers.sort_by_key(|info| info.lovel);
        }
        entries.push(KeyMapEntry {
            bank: preset.bank as u8,
            preset: preset.preset as u8,
            map: key_map,
        });
    }
    Ok(entries)
}

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
