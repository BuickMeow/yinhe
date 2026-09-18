//! SFZ 解析：region 展开为 (key, vel) 快照（含加载期烘焙）。

use super::*;

pub(super) fn build_key_map_from_sfz(
    sfz_path: &Path,
    sample_rate: u32,
    interp: u32,
) -> Result<Vec<Vec<KeyInfo>>, String> {
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

                let (pan_l, pan_r) = pan_gains(pan);
                let biquad = bake_biquad(cutoff, resonance, region.filter_type, sample_rate);
                key_map[key as usize].push(KeyInfo {
                    sample_data: samples.clone(),
                    sample_rate,
                    is_stereo,
                    interp,
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
                    biquad,
                    pan_l,
                    pan_r,
                });
            }
        }
    }

    for layers in key_map.iter_mut() {
        layers.sort_by_key(|info| info.lovel);
    }
    Ok(key_map)
}
