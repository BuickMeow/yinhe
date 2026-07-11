//! SFZ key-map builder using xsynth-soundfonts' standard parser.
//!
//! 委托 xsynth-soundfonts 解析 SFZ（处理 <group>/<master>/#include/继承等），
//! 然后提取每个 key 所需的合成参数。

use std::path::{Path, PathBuf};

/// 每个 MIDI key 对应的 SFZ 合成参数。
#[derive(Clone, Debug)]
pub struct SfzKeyInfo {
    pub sample_path: PathBuf,
    pub pitch_keycenter: u8,
    pub tune: i16,
    pub volume: f32,       // dB → 线性增益
    pub pan: f32,          // -1 (左) .. +1 (右)
    pub offset: u32,       // 采样起始偏移
    pub ampeg_attack: f32,
    pub ampeg_decay: f32,
    pub ampeg_sustain: f32,   // 0..1 (SFZ 是 0..100，已转换)
    pub ampeg_release: f32,
    pub lovel: u8,
    pub hivel: u8,
    pub loop_mode: LoopMode,
    pub loop_start: u32,
    pub loop_end: u32,
    pub amp_veltrack: f32,
}

/// 采样循环模式（与 xsynth LoopMode 对应）
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopMode {
    NoLoop,
    LoopContinuous,
    LoopSustain,
    OneShot,
}

impl Default for SfzKeyInfo {
    fn default() -> Self {
        Self {
            sample_path: PathBuf::from("missing"),
            pitch_keycenter: 60,
            tune: 0,
            volume: 1.0,
            pan: 0.0,
            offset: 0,
            ampeg_attack: 0.01,
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

/// Build a lookup table: MIDI key → SfzKeyInfo.
/// 选择 keyrange 最窄的 region（最精确匹配）。
pub fn build_key_map_from_sfz(sfz_path: &Path) -> Result<Vec<SfzKeyInfo>, String> {
    let regions = xsynth_soundfonts::sfz::parse_soundfont(sfz_path)
        .map_err(|e| format!("SFZ parse error: {}", e))?;

    let mut key_map: Vec<Option<(SfzKeyInfo, u8)>> = vec![None; 128]; // (info, range_width)

    for region in &regions {
        let pkc = region.pitch_keycenter as u8;
        let range_width = (*region.keyrange.end() as i16 - *region.keyrange.start() as i16).unsigned_abs() as u8;

        // volume: dB → 线性
        let vol_linear = 10.0f32.powf(region.volume as f32 / 20.0);

        // pan: -100..100 → -1..1
        let pan_norm = (region.pan as f32 / 100.0).clamp(-1.0, 1.0);

        // ampeg_sustain: SFZ 0..100 → 0..1
        let sustain_norm = (region.ampeg_envelope.ampeg_sustain / 100.0).clamp(0.0, 1.0);

        // loop_mode 转换
        let loop_mode = match region.loop_mode {
            xsynth_soundfonts::LoopMode::NoLoop => LoopMode::NoLoop,
            xsynth_soundfonts::LoopMode::LoopContinuous => LoopMode::LoopContinuous,
            xsynth_soundfonts::LoopMode::LoopSustain => LoopMode::LoopSustain,
            xsynth_soundfonts::LoopMode::OneShot => LoopMode::OneShot,
        };

        let info = SfzKeyInfo {
            sample_path: region.sample_path.clone(),
            pitch_keycenter: pkc,
            tune: region.tune,
            volume: vol_linear,
            pan: pan_norm,
            offset: region.offset,
            ampeg_attack: region.ampeg_envelope.ampeg_attack.max(0.001),
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
            if k < 128 {
                let replace = match &key_map[k] {
                    None => true,
                    Some((_, prev_width)) => range_width < *prev_width,
                };
                if replace {
                    key_map[k] = Some((info.clone(), range_width));
                }
            }
        }
    }

    Ok(key_map
        .into_iter()
        .map(|opt| opt.map(|(info, _)| info).unwrap_or_default())
        .collect())
}

/// Load a WAV file as f32 samples (mono, normalized to -1..1).
pub fn load_wav_as_f32(path: &Path) -> Result<Vec<f32>, String> {
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

    if spec.channels == 2 {
        let mono: Vec<f32> = samples
            .chunks(2)
            .map(|pair| {
                if pair.len() == 2 {
                    (pair[0] + pair[1]) * 0.5
                } else {
                    pair[0]
                }
            })
            .collect();
        Ok(mono)
    } else {
        Ok(samples)
    }
}
