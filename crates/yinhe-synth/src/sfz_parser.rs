//! SoundFont 解析器 — 统一支持 SFZ 和 SF2 格式。
//!
//! 委托 xsynth-soundfonts 解析 SFZ/SF2，然后提取每个 key 所需的合成参数。
//! SFZ: 采样数据从 WAV 文件加载
//! SF2: 采样数据内嵌在文件中，已解析为 f32

use std::path::{Path, PathBuf};

/// 每个 MIDI key 对应的合成参数。
#[derive(Clone, Debug)]
pub struct KeyInfo {
    // 采样数据来源（二选一）
    pub sample_path: Option<PathBuf>,    // SFZ: WAV 文件路径
    pub sample_data: Option<Vec<f32>>,   // SF2: 内嵌采样数据
    pub sample_rate: u32,

    // 合成参数
    pub pitch_keycenter: u8,
    pub tune: i16,
    pub volume: f32,       // dB → 线性增益
    pub pan: f32,          // -1 (左) .. +1 (右)
    pub offset: u32,       // 采样起始偏移
    pub ampeg_start: f32,
    pub ampeg_delay: f32,
    pub ampeg_attack: f32,
    pub ampeg_hold: f32,
    pub ampeg_decay: f32,
    pub ampeg_sustain: f32,   // 0..1
    pub ampeg_release: f32,
    pub lovel: u8,
    pub hivel: u8,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub amp_veltrack: f32,
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
            sample_path: None,
            sample_data: None,
            sample_rate: 44100,
            pitch_keycenter: 60,
            tune: 0,
            volume: 1.0,
            pan: 0.0,
            offset: 0,
            ampeg_start: 0.0,
            ampeg_delay: 0.0,
            ampeg_attack: 0.01,
            ampeg_hold: 0.0,
            ampeg_decay: 0.0,
            ampeg_sustain: 1.0,
            ampeg_release: 0.01,
            lovel: 0,
            hivel: 127,
            loop_mode: LoopMode::NoLoop,
            loop_start: 0,
            loop_end: 0,
            amp_veltrack: 100.0,
        }
    }
}

/// 根据文件扩展名自动检测格式并构建 key map。
/// 支持 .sfz 和 .sf2 格式。
pub fn build_key_map(path: &Path) -> Result<Vec<Vec<KeyInfo>>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase());
    match ext.as_deref() {
        Some("sfz") => build_key_map_from_sfz(path),
        Some("sf2") => build_key_map_from_sf2(path),
        _ => Err(format!("Unsupported soundfont format: {:?}", path)),
    }
}

/// 根据 key 和 velocity 选择对应的 KeyInfo（力度分层）。
pub fn select_key_info<'a>(key_map: &'a [Vec<KeyInfo>], key: u8, velocity: u8) -> Option<&'a KeyInfo> {
    let layers = &key_map[key as usize];
    if layers.is_empty() { return None; }
    for info in layers {
        if velocity >= info.lovel && velocity <= info.hivel {
            return Some(info);
        }
    }
    layers.iter().min_by_key(|info| {
        (velocity as i16 - info.lovel as i16).unsigned_abs()
            .min((velocity as i16 - info.hivel as i16).unsigned_abs())
    })
}

// ── SFZ ──

fn build_key_map_from_sfz(sfz_path: &Path) -> Result<Vec<Vec<KeyInfo>>, String> {
    let regions = xsynth_soundfonts::sfz::parse_soundfont(sfz_path)
        .map_err(|e| format!("SFZ parse error: {}", e))?;

    let mut key_map: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];

    for region in &regions {
        let vol_linear = 10.0f32.powf(region.volume as f32 / 20.0);
        let pan_norm = (region.pan as f32 / 100.0).clamp(-1.0, 1.0);
        let sustain_norm = (region.ampeg_envelope.ampeg_sustain / 100.0).clamp(0.0, 1.0);
        let loop_mode = convert_loop_mode(region.loop_mode);

        let info = KeyInfo {
            sample_path: Some(region.sample_path.clone()),
            sample_data: None,
            sample_rate: 44100,
            pitch_keycenter: region.pitch_keycenter as u8,
            tune: region.tune,
            volume: vol_linear,
            pan: pan_norm,
            offset: region.offset,
            ampeg_start: region.ampeg_envelope.ampeg_start,
            ampeg_delay: region.ampeg_envelope.ampeg_delay,
            ampeg_attack: region.ampeg_envelope.ampeg_attack.max(0.001),
            ampeg_hold: region.ampeg_envelope.ampeg_hold,
            ampeg_decay: region.ampeg_envelope.ampeg_decay.max(0.001),
            ampeg_sustain: sustain_norm,
            ampeg_release: region.ampeg_envelope.ampeg_release.max(0.001),
            lovel: *region.velrange.start(),
            hivel: *region.velrange.end(),
            loop_mode,
            loop_start: region.loop_start,
            loop_end: region.loop_end,
            amp_veltrack: region.amp_veltrack,
        };

        for key in region.keyrange.clone() {
            let k = key as usize;
            if k < 128 { key_map[k].push(info.clone()); }
        }
    }

    for layers in key_map.iter_mut() {
        layers.sort_by_key(|info| info.lovel);
    }
    Ok(key_map)
}

// ── SF2 ──

/// 根据文件扩展名自动检测格式并构建 key map 时，SF2 需要目标采样率。
/// 这里用 44100 让 load_soundfont 做重采样，后续 GpuSynth 会再次重采样到目标。
const SF2_LOAD_SAMPLE_RATE: u32 = 44100;

fn build_key_map_from_sf2(sf2_path: &Path) -> Result<Vec<Vec<KeyInfo>>, String> {
    let presets = xsynth_soundfonts::sf2::load_soundfont(sf2_path, SF2_LOAD_SAMPLE_RATE)
        .map_err(|e| format!("SF2 parse error: {}", e))?;

    let mut key_map: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];

    // 取第一个 preset（后续可扩展为用户选择 preset）
    let preset = presets.first().ok_or("SF2: no presets found")?;

    for region in &preset.regions {
        let loop_mode = convert_loop_mode(region.loop_mode);

        // SF2 sample 是 Arc<[Arc<[f32]>]>：单声道=1通道，立体声=2通道
        // GPU 渲染器是单声道的，立体声样本取左右平均
        let sample_data: Vec<f32> = if region.sample.len() == 2 {
            // 立体声：左右平均为单声道
            let left = &region.sample[0];
            let right = &region.sample[1];
            let len = left.len().min(right.len());
            (0..len).map(|i| (left[i] + right[i]) * 0.5).collect()
        } else if region.sample.len() == 1 {
            region.sample[0].to_vec()
        } else {
            continue;
        };

        // volume 已经是线性值（10^(-attenuation/200)），不需要再转换
        // pan 是 i16（-500..+500），归一化到 -1..+1
        let pan_norm = (region.pan as f32 / 500.0).clamp(-1.0, 1.0);
        // sustain 已经是百分比（0..100），归一化到 0..1
        let sustain_norm = (region.ampeg_envelope.ampeg_sustain / 100.0).clamp(0.0, 1.0);

        let tune = region.fine_tune.wrapping_add(
            (region.coarse_tune.wrapping_mul(100))
        );

        let info = KeyInfo {
            sample_path: None,
            sample_data: Some(sample_data),
            // load_soundfont 传入 SF2_LOAD_SAMPLE_RATE 后数据已被重采样，
            // 但 Sf2Region.sample_rate 仍是原始采样率。
            // 因为数据已经是重采样后的，用 SF2_LOAD_SAMPLE_RATE
            sample_rate: SF2_LOAD_SAMPLE_RATE,
            pitch_keycenter: region.root_key,
            tune,
            volume: region.volume,
            pan: pan_norm,
            offset: region.offset,
            ampeg_start: region.ampeg_envelope.ampeg_start,
            ampeg_delay: region.ampeg_envelope.ampeg_delay,
            ampeg_attack: region.ampeg_envelope.ampeg_attack.max(0.001),
            ampeg_hold: region.ampeg_envelope.ampeg_hold,
            ampeg_decay: region.ampeg_envelope.ampeg_decay.max(0.001),
            ampeg_sustain: sustain_norm,
            ampeg_release: region.ampeg_envelope.ampeg_release.max(0.001),
            lovel: *region.velrange.start(),
            hivel: *region.velrange.end(),
            loop_mode,
            loop_start: region.loop_start,
            loop_end: region.loop_end,
            amp_veltrack: 100.0,
        };

        for key in region.keyrange.clone() {
            let k = key as usize;
            if k < 128 { key_map[k].push(info.clone()); }
        }
    }

    for layers in key_map.iter_mut() {
        layers.sort_by_key(|info| info.lovel);
    }
    Ok(key_map)
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

/// Load a WAV file as f32 samples (mono, normalized to -1..1).
/// 返回 (samples, sample_rate)。
pub fn load_wav_as_f32(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| format!("Failed to open WAV {:?}: {}", path, e))?;

    let spec = reader.spec();
    let samples: Vec<f32> = match spec.bits_per_sample {
        16 => reader
            .samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect(),
        24 => reader
            .samples::<i32>()
            .map(|s| {
                let v = s.unwrap();
                (v >> 8) as f32 / (i16::MAX as f32)
            })
            .collect(),
        32 => reader
            .samples::<i32>()
            .map(|s| s.unwrap() as f32 / i32::MAX as f32)
            .collect(),
        _ => return Err(format!("Unsupported bit depth: {}", spec.bits_per_sample)),
    };

    let mono: Vec<f32> = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|pair| {
                if pair.len() == 2 { (pair[0] + pair[1]) * 0.5 } else { pair[0] }
            })
            .collect()
    } else {
        samples
    };

    Ok((mono, spec.sample_rate))
}
