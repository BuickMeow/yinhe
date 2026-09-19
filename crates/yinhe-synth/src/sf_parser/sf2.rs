//! SF2 解析：preset/instrument/zone 展开为 (key, vel) 快照（含 modulator
//! 折算与加载期烘焙）。

use super::*;

// ── SF2 ──

pub(super) fn build_key_maps_from_sf2(
    sf2_path: &Path,
    sample_rate: u32,
    interp: u32,
) -> Result<Vec<KeyMapEntry>, String> {
    // 直接以目标采样率加载，一次重采样到位（避免先转 44100 再转目标的双重重采样）
    let presets = xsynth_soundfonts::sf2::load_soundfont(sf2_path, sample_rate)
        .map_err(|e| format!("SF2 parse error: {}", e))?;
    if presets.is_empty() {
        return Err("SF2: no presets found".to_string());
    }

    // 每个 preset 一个条目（bank, preset）→ key map，program 切换直接索引。
    // preset 间采样数据 Arc 共享，上传 GPU 时按 Arc 身份去重，无重复拷贝。
    //
    // 立体声交错缓存：同一对 (left, right) 样本可能被多个 preset/region
    // 引用（GM 库常态），必须复用同一份交错数据——否则每个 region 各拷一份
    // （实测 TWGMD Ultimate 311MB 的 sf2 膨胀到 15GB，且前 600MB 之外的
    // 采样被 chunk 上限截断导致整库静音）。
    let mut stereo_cache: HashMap<(usize, usize), Arc<[f32]>> = HashMap::new();
    // 样本峰值缓存：peak==0 的全静音层跳过（有些库把静音样本放在
    // 同 key/vel 的候选前面，只取第一个匹配层时会选中它导致整音静音）
    let mut peak_cache: HashMap<usize, f32> = HashMap::new();
    let mut entries = Vec::with_capacity(presets.len());
    for preset in &presets {
        let mut key_map: Vec<Vec<KeyInfo>> = vec![Vec::new(); 128];
        for region in &preset.regions {
            // 采样数据：单声道 Arc 共享零拷贝；立体声交错存储（左右声道各一个 Arc）
            let (sample_data, is_stereo): (Arc<[f32]>, bool) = if region.sample.len() == 2 {
                let left = &region.sample[0];
                let right = &region.sample[1];
                let key = (
                    Arc::as_ptr(left) as *const f32 as usize,
                    Arc::as_ptr(right) as *const f32 as usize,
                );
                if let Some(cached) = stereo_cache.get(&key) {
                    (Arc::clone(cached), true)
                } else {
                    let len = left.len().min(right.len());
                    let mut interleaved = Vec::with_capacity(len * 2);
                    for i in 0..len {
                        interleaved.push(left[i]);
                        interleaved.push(right[i]);
                    }
                    let arc: Arc<[f32]> = Arc::from(interleaved);
                    stereo_cache.insert(key, Arc::clone(&arc));
                    (arc, true)
                }
            } else if region.sample.len() == 1 {
                (Arc::clone(&region.sample[0]), false)
            } else {
                continue;
            };

            let sample_ptr = sample_data.as_ptr() as usize;
            let peak = *peak_cache.entry(sample_ptr).or_insert_with(|| {
                sample_data.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
            });
            if peak == 0.0 {
                continue;
            }

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
                    let resonance = 10.0f32.powf(np.resonance / 20.0) * Q_BUTTERWORTH;
                    let filter_type = FilterType::LowPass;
                    let (pan_l, pan_r) = pan_gains(pan);
                    let biquad = bake_biquad(cutoff, resonance, filter_type, sample_rate);
                    let loop_mode = if region.loop_start == region.loop_end {
                        LoopMode::NoLoop
                    } else {
                        convert_loop_mode(region.loop_mode)
                    };

                    key_map[key as usize].push(KeyInfo {
                        sample_data: sample_data.clone(),
                        sample_rate,
                        is_stereo,
                        interp,
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
                        resonance,
                        filter_type,
                        biquad,
                        pan_l,
                        pan_r,
                    });
                }
            }
        }

        // 排序：lovel 升序；同 lovel 时**峰值大的层优先**——同 key/vel 有多个
        // 候选（多层采样）时，只取第一个匹配层会选中轻层/静音层（已过滤静音），
        // 优先取最强层使单层播放尽量接近 xsynth 的多层叠加响度。
        for layers in key_map.iter_mut() {
            layers.sort_by(|a, b| {
                a.lovel.cmp(&b.lovel).then_with(|| {
                    let pa = peak_cache
                        .get(&(a.sample_data.as_ptr() as usize))
                        .copied()
                        .unwrap_or(0.0);
                    let pb = peak_cache
                        .get(&(b.sample_data.as_ptr() as usize))
                        .copied()
                        .unwrap_or(0.0);
                    pb.total_cmp(&pa)
                })
            });
        }
        entries.push(KeyMapEntry {
            bank: preset.bank as u8,
            preset: preset.preset as u8,
            map: key_map,
        });
    }
    Ok(entries)
}
