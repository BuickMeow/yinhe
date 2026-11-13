//! SFZ 解析：region 展开为 (key, vel) 快照（含加载期烘焙）。
//!
//! 解析走自有 `crate::sfz`（不是 xsynth）：只有它认得 `xfin_*` / `trigger` /
//! `seq_*` / `pitch_random` / `amplfo_*` 这些决定「像不像真乐器」的 opcode。
//! 每个 region 另外产出一条 `LayerExtra`（门控/轮次/释放触发/LFO），
//! `KeyInfo` 只保留 `extra` 索引，避免 670 万条快照膨胀。

use super::*;
use crate::sfz::{Ampeg, LoopMode as SfzLoopMode, SfzRegion, Trigger};

/// SFZ key map 的构建结果：攻击层、释放层、侧表、通道默认 CC。
pub(super) struct SfzBuild {
    pub attack: Vec<Vec<KeyInfo>>,
    pub release: Vec<Vec<KeyInfo>>,
    pub extras: Vec<LayerExtra>,
    pub cc_defaults: Vec<(u8, u8)>,
}

pub(super) fn build_key_map_from_sfz_with_progress(
    sfz_path: &Path,
    sample_rate: u32,
    interp: u32,
    on_progress: &mut dyn FnMut(f32),
) -> Result<SfzBuild, String> {
    let file = crate::sfz::parse(sfz_path).map_err(|e| format!("SFZ parse error: {e}"))?;
    let region_count = file.regions.len().max(1);

    let mut attack: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];
    let mut release: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];
    let mut extras: Vec<LayerExtra> = Vec::new();

    // wav 按路径去重加载并重采样到目标采样率（同一文件被多个 region 引用）
    let mut wav_cache: HashMap<PathBuf, (Arc<[f32]>, u32, bool)> = HashMap::new();

    for (region_idx, region) in file.regions.iter().enumerate() {
        on_progress((region_idx + 1) as f32 / region_count as f32);
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
                } else if raw_stereo {
                    // 重采样前必须分声道解交错：resample_vec 是单声道序列插值，
                    // 直接对 LRLR 交错数据重采样会把左右声道混在一起（波形错乱）
                    resample_interleaved(raw, src_sr, sample_rate)
                } else {
                    // 单声道：按单声道序列重采样（走交错路径会把样本按奇偶
                    // 拆成两个"半速声道"，波形/音调全错）
                    xsynth_soundfonts::resample::resample_vec(
                        raw,
                        src_sr as f32,
                        sample_rate as f32,
                    )
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
        let extra_idx = push_extra(&mut extras, region);

        for key in region.keyrange.clone() {
            // SFZ 允许 `c#-1`（=1）这类边界；统一裁到 MIDI 0..=127
            if !(0..=127).contains(&key) {
                continue;
            }
            let key = key as u8;
            let key_f = key as f32;
            for vel in *region.velrange.start()..=*region.velrange.end() {
                let vel_f = vel as f32;

                // 播放倍率（xsynth: get_speed_mult_from_keys × cents_factor(tune)）
                let speed_mult = 2.0f32.powf((key_f - region.pitch_keycenter as f32) / 12.0)
                    * 2.0f32.powf(region.tune / 1200.0);

                // 音量（xsynth new_sfz 公式）：
                // vel 曲线 -> vol_vel 归一化后平方；键位音量跟踪加到 dB 再转线性
                let a = region.amp_veltrack / 100.0;
                let aabs = a.abs();
                let vol_vel = 127.0 * (1.0 - aabs)
                    + vel_f * (a + aabs) / 2.0
                    + (127.0 - vel_f) * (aabs - a) / 2.0;
                let vol_mult = (vol_vel / 127.0).powi(2);
                let vol_db = (region.volume_db
                    + (key_f - region.amp_keycenter as f32) * region.amp_keytrack)
                    .clamp(-96.0, 12.0);
                let volume = vol_mult * 10.0f32.powf(vol_db / 20.0);

                // 声像（xsynth new_sfz 公式）：vel/key 修正后归一化到 0..1
                let pan_mult = vel_f / 127.0 * region.pan_veltrack
                    + (key_f - region.pan_keycenter as f32) * region.pan_keytrack;
                let pan = ((region.pan + pan_mult).clamp(-100.0, 100.0) / 100.0 + 1.0) / 2.0;

                // 滤波器截止频率（xsynth new_sfz 公式，cutoff >= 1.0 才启用）
                let mut cutoff = 0.0;
                if let Some(cutoff_t) = region.cutoff
                    && cutoff_t >= 1.0
                {
                    let cents = vel_f / 127.0 * region.fil_veltrack
                        + (key_f - region.fil_keycenter as f32) * region.fil_keytrack;
                    cutoff = (cutoff_t * 2.0f32.powf(cents / 1200.0))
                        .clamp(1.0, sample_rate as f32 / 2.0 - 100.0);
                }
                let resonance = 10.0f32.powf(region.resonance_db / 20.0) * Q_BUTTERWORTH;

                // 包络（xsynth: release 随力度加长；sustain/start 从 % 归一化）
                let ampeg: &Ampeg = &region.ampeg;
                let release_sec = ampeg.release + (vel_f / 127.0) * ampeg.vel2release;

                let loop_mode = match region.loop_mode {
                    SfzLoopMode::NoLoop => LoopMode::NoLoop,
                    SfzLoopMode::OneShot => LoopMode::OneShot,
                    SfzLoopMode::LoopContinuous => LoopMode::LoopContinuous,
                    SfzLoopMode::LoopSustain => LoopMode::LoopSustain,
                };
                // 没有 loop 区间时按 no_loop 处理（与旧实现一致）
                let loop_mode = if region.loop_start == region.loop_end {
                    LoopMode::NoLoop
                } else {
                    loop_mode
                };

                let (pan_l, pan_r) = pan_gains(pan);
                let info = KeyInfo {
                    sample_data: samples.clone(),
                    is_stereo,
                    interp: interp as u8,
                    speed_mult,
                    volume,
                    offset: (region.offset as f32 * factor) as u32,
                    stop: u32::MAX,
                    ampeg_start: ampeg.start / 100.0,
                    ampeg_delay: ampeg.delay,
                    ampeg_attack: ampeg.attack.max(0.001),
                    ampeg_hold: ampeg.hold,
                    ampeg_decay: ampeg.decay.max(0.001),
                    ampeg_sustain: (ampeg.sustain / 100.0).clamp(0.0, 1.0),
                    ampeg_release: release_sec.max(0.001),
                    lovel: vel,
                    hivel: vel,
                    loop_mode,
                    loop_start: (region.loop_start as f32 * factor) as u32,
                    loop_end: (region.loop_end as f32 * factor) as u32,
                    cutoff,
                    resonance,
                    filter_type: region.filter_type,
                    exclusive_class: 0,
                    pan_l,
                    pan_r,
                    extra: extra_idx,
                };
                match region.trigger {
                    Trigger::Attack => attack[key as usize].push(info),
                    Trigger::Release => release[key as usize].push(info),
                }
            }
        }
    }

    for layers in attack.iter_mut().chain(release.iter_mut()) {
        layers.sort_by_key(|info| info.lovel);
    }
    Ok(SfzBuild {
        attack,
        release,
        extras,
        cc_defaults: file.cc_defaults,
    })
}

/// 把一条 region 的门控/触发行为转成 `LayerExtra`，返回其索引。
fn push_extra(extras: &mut Vec<LayerExtra>, region: &SfzRegion) -> u32 {
    let mut extra = LayerExtra {
        cc_gates: region.cc_gates.clone(),
        on_gates: region.on_gates.clone(),
        seq_length: region.seq_length.min(u16::MAX as u32) as u16,
        seq_position: region.seq_position.min(u16::MAX as u32) as u16,
        group: region.group.min(u16::MAX as u32) as u16,
        off_by: region.off_by.min(u16::MAX as u32) as u16,
        sw_last: region.sw_last.unwrap_or(0xFF),
        sw_lokey: region.sw_lokey.unwrap_or(0),
        sw_hikey: region.sw_hikey.unwrap_or(127),
        // 只给了 sw_lokey/sw_hikey 没有 sw_last 也视为开关（区域内的键要被吞掉）
        has_sw: region.sw_last.is_some()
            || (region.sw_lokey.is_some() && region.sw_hikey.is_some()),
        release_trigger: region.trigger == Trigger::Release,
        one_shot: region.loop_mode == SfzLoopMode::OneShot,
        lorand: region.lorand.unwrap_or(0.0).clamp(0.0, 1.0),
        hirand: region.hirand.unwrap_or(1.0).clamp(0.0, 1.0),
        pitch_random: region.pitch_random,
        amplfo: region.amplfo.map(|l| Lfo {
            depth: l.depth,
            freq: l.freq,
            phase: l.phase,
        }),
        pitchlfo: region.pitchlfo.map(|l| Lfo {
            depth: l.depth,
            freq: l.freq,
            phase: l.phase,
        }),
    };
    // 空门控归一：避免 `lorand=0 hirand=1` 之外的空数组每层都走判定
    if extra.cc_gates.is_empty() && extra.on_gates.is_empty() {
        // 保持默认（is_plain 会返回 true）
    }
    if extra.hirand >= 1.0 {
        extra.hirand = 1.0;
    }
    extras.push(extra);
    (extras.len() - 1) as u32
}
