//! 和弦识别（纯逻辑）：把“当前按下的琴键集合”识别为和弦名，供 PR 视图的和弦指示器使用。
//!
//! 防乱按规则：不同音级超过 7 个、或按键跨度超过 24 半音（两个八度）时不识别，
//! 避免双手乱拍被误识别成莫名其妙的和弦。匹配不上任何和弦时也返回 None（宁缺毋滥）。

use yinhe_core::YinModel;
use yinhe_types::NOTE_NAMES;
use yinhe_types::{NoteSource, measure_ticks};

use crate::document::TrackOverride;

/// 音级集合转 bitmask：bit i 置位表示音级 i 出现。
const fn pc_mask(pcs: &[u8]) -> u16 {
    let mut mask = 0u16;
    let mut i = 0;
    while i < pcs.len() {
        mask |= 1 << pcs[i];
        i += 1;
    }
    mask
}

/// 和弦表：相对根音的音级 bitmask（精确相等才匹配）到后缀名，全部为 ASCII。
/// 按音数从多到少排列（虽然精确匹配不会误判，但保持优先级清晰）。
/// 标准取 Real Book/Berklee 文本型（m/maj/dim/aug/sus/b/#），6/9 等计为五音。
const CHORDS: &[(u16, &str)] = &[
    // 五音和弦
    (pc_mask(&[0, 2, 4, 7, 10]), "9"),
    (pc_mask(&[0, 2, 4, 7, 11]), "maj9"),
    (pc_mask(&[0, 2, 3, 7, 10]), "m9"),
    (pc_mask(&[0, 2, 4, 7, 9]), "6/9"),
    (pc_mask(&[0, 2, 3, 7, 9]), "m6/9"),
    (pc_mask(&[0, 1, 4, 7, 10]), "7b9"),
    (pc_mask(&[0, 3, 4, 7, 10]), "7#9"),
    (pc_mask(&[0, 4, 6, 7, 10]), "7#11"),
    (pc_mask(&[0, 4, 6, 7, 11]), "maj7#11"),
    // 四音和弦
    (pc_mask(&[0, 4, 7, 10]), "7"),
    (pc_mask(&[0, 4, 7, 11]), "maj7"),
    (pc_mask(&[0, 3, 7, 10]), "m7"),
    (pc_mask(&[0, 3, 7, 11]), "m(maj7)"),
    (pc_mask(&[0, 3, 6, 9]), "dim7"),
    (pc_mask(&[0, 3, 6, 10]), "m7b5"),
    (pc_mask(&[0, 4, 8, 10]), "aug7"),
    (pc_mask(&[0, 4, 7, 9]), "6"),
    (pc_mask(&[0, 3, 7, 9]), "m6"),
    (pc_mask(&[0, 2, 4, 7]), "add9"),
    (pc_mask(&[0, 2, 3, 7]), "m(add9)"),
    (pc_mask(&[0, 5, 7, 10]), "7sus4"),
    (pc_mask(&[0, 3, 5, 7]), "m11"),
    // 三音和弦（大三和弦后缀为空，只显示根音名）
    (pc_mask(&[0, 4, 7]), ""),
    (pc_mask(&[0, 3, 7]), "m"),
    (pc_mask(&[0, 3, 6]), "dim"),
    (pc_mask(&[0, 4, 8]), "aug"),
    (pc_mask(&[0, 2, 7]), "sus2"),
    (pc_mask(&[0, 5, 7]), "sus4"),
    (pc_mask(&[0, 3, 5]), "m(add4)"),
    (pc_mask(&[0, 4, 5]), "add4"),
    // 二音（power chord）
    (pc_mask(&[0, 7]), "5"),
];

/// 识别按下的琴键集合对应的和弦名。
///
/// - 空输入或单音返回 None（单音不视为和弦，`layout` 层不再显示 `G#3` 这类音名）
/// - 两个及以上键做精确和弦匹配；根音不等于低音时用 slash 记法（如 "C/E"）
/// - 匹配不上、音级超过 7 个、或跨度超过 24 半音时返回 None
pub fn recognize(keys: &[u8]) -> Option<String> {
    if keys.len() < 2 {
        return None;
    }
    let mut mask = 0u16;
    let mut low = u8::MAX;
    let mut high = u8::MIN;
    for &key in keys {
        mask |= 1 << (key % 12);
        low = low.min(key);
        high = high.max(key);
    }
    // 防乱按：音级太多或跨度太大都不识别。
    if mask.count_ones() > 7 || high - low > 24 {
        return None;
    }
    let bass_pc = low % 12;
    // 候选根音：低音优先，其余出现的音级按升序。
    let roots = std::iter::once(bass_pc)
        .chain((0..12u8).filter(|&pc| pc != bass_pc && mask & (1 << pc) != 0));
    for root in roots {
        // 以 root 为根音，把所有出现的音级折算成相对音级。
        let mut rel = 0u16;
        for pc in 0..12u8 {
            if mask & (1 << pc) != 0 {
                rel |= 1 << ((pc + 12 - root) % 12);
            }
        }
        if let Some(&(_, suffix)) = CHORDS.iter().find(|&&(m, _)| m == rel) {
            let name = format!("{}{}", NOTE_NAMES[root as usize], suffix);
            if root == bass_pc {
                return Some(name);
            }
            // 转位：slash 记法标出实际低音。
            return Some(format!("{}/{}", name, NOTE_NAMES[bass_pc as usize]));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::recognize;

    #[test]
    fn single_key_shows_note_name() {
        // 单音不再视为和弦
        assert_eq!(recognize(&[60]), None);
    }

    #[test]
    fn major_triad() {
        assert_eq!(recognize(&[60, 64, 67]), Some("C".to_string()));
    }

    #[test]
    fn minor_triad() {
        assert_eq!(recognize(&[60, 63, 67]), Some("Cm".to_string()));
    }

    #[test]
    fn dominant_seventh() {
        assert_eq!(recognize(&[60, 64, 67, 70]), Some("C7".to_string()));
    }

    #[test]
    fn inversion_slash_notation() {
        // C/E：低音是 E。
        assert_eq!(recognize(&[64, 67, 72]), Some("C/E".to_string()));
    }

    #[test]
    fn sus4() {
        assert_eq!(recognize(&[60, 65, 67]), Some("Csus4".to_string()));
    }

    #[test]
    fn dim7() {
        assert_eq!(recognize(&[60, 63, 66, 69]), Some("Cdim7".to_string()));
    }

    #[test]
    fn add9() {
        assert_eq!(recognize(&[60, 62, 64, 67]), Some("Cadd9".to_string()));
    }

    #[test]
    fn ninth() {
        assert_eq!(recognize(&[60, 62, 64, 67, 70]), Some("C9".to_string()));
    }

    #[test]
    fn too_many_pitch_classes_rejected() {
        // 8 个不同音级，属于乱按，不识别。
        assert_eq!(recognize(&[60, 61, 62, 63, 64, 65, 66, 67]), None);
    }

    #[test]
    fn wide_span_rejected() {
        // 跨度 25 半音 > 24，不识别。
        assert_eq!(recognize(&[60, 85]), None);
    }

    #[test]
    fn unrecognized_returns_none() {
        // {0,1} 不在和弦表里。
        assert_eq!(recognize(&[60, 61]), None);
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(recognize(&[]), None);
    }
}

// ── 和弦指示器（原 yinhe-egui/src/app/layout/chord.rs，下沉至此） ──

/// 和弦指示器文本：实时 MIDI 直通按键优先；无按键且播放中 → 播放头处发声音符的和弦。
///
/// 播放路径逐轨识别：主轨优先、PR 可见次之，跳过静音/不可见/力度≤1 的轨道，
/// 在所有候选轨道中择最完善的多音和弦（音级数最多），无多音和弦时回退单音。
/// 每轨内部复用 `recognize` 的防乱按规则（音级 > 7 或跨度 > 2 八度不识别）。
#[allow(clippy::too_many_arguments)]
pub fn indicator_text(
    thru_keys: &std::collections::HashMap<u8, u8>,
    is_playing: bool,
    cursor_tick: Option<f64>,
    model: Option<&YinModel>,
    main_track: Option<u16>,
    pr_visible: &[bool],
    track_overrides: &[TrackOverride],
    conductor_track_idx: Option<u16>,
) -> Option<String> {
    if !thru_keys.is_empty() {
        let mut keys: Vec<u8> = thru_keys.keys().copied().collect();
        keys.sort_unstable();
        return recognize(&keys);
    }
    if !is_playing {
        return None;
    }
    let tick = cursor_tick?.max(0.0) as u32;
    let model = model?;
    let n = model.tracks.len();
    if n == 0 {
        return None;
    }
    // 静音/独奏遮罩：有独奏时仅独奏轨可发声，其余视为静音。
    let has_solo = track_overrides.iter().any(|ov| ov.soloed);
    let is_muted = |idx: usize| -> bool {
        if let Some(ov) = track_overrides.get(idx) {
            if has_solo {
                return !ov.soloed;
            }
            return ov.muted;
        }
        // 回退到 model 标记（旧工程或长度不一致时）
        model.tracks.get(idx).is_some_and(|t| t.muted)
    };

    // 每首歌自适应的密度阈值：密集视觉轨（黑墙）per_bar 远高于音乐轨，用中位数自适应过滤。
    // tick 处的拍号决定 bar_ticks，避免 3/4、6/8 等误算。
    let (num, den) = model.tempo_map.time_sig_at_tick(tick);
    let bar_ticks = measure_ticks(model.meta.ppq, num, den).max(1);
    let tick_len = model.tick_length.max(1) as f64;
    let mut per_bar_vec = Vec::with_capacity(n);
    for idx in 0..n {
        let cnt = model.track_note_count.get(idx).copied().unwrap_or(0) as f64;
        per_bar_vec.push(cnt * bar_ticks as f64 / tick_len);
    }
    // 中位数 *2.5 自适应，至少 60，避免稀疏歌误杀。
    let mut sorted_pb = per_bar_vec.clone();
    sorted_pb.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_pb = if sorted_pb.is_empty() {
        0.0
    } else if sorted_pb.len() % 2 == 1 {
        sorted_pb[sorted_pb.len() / 2]
    } else {
        (sorted_pb[sorted_pb.len() / 2 - 1] + sorted_pb[sorted_pb.len() / 2]) * 0.5
    };
    let density_thresh = (median_pb * 2.5).max(60.0);
    let is_dense = |idx: usize| -> bool { per_bar_vec[idx] > density_thresh };

    // 每轨发声 key 集合（已过滤静音/不可见/力度≤1），按 key 升序自然有序。
    let mut keys_by_track_sim: Vec<Vec<u8>> = vec![Vec::new(); n];
    // 单次 128 key 扫描，按 tick 覆盖把 key 分发到各轨的 present 标记。
    for key in 0u8..128 {
        // 固定 256 容纳极端多轨（桌面默认 17 轨），避免每 key 分配 HashSet。
        let mut present = [false; 256];
        let mut any = false;
        for note in model.key_notes_in_range(key, tick, tick.saturating_add(1)) {
            if note.velocity <= 1 {
                continue;
            }
            if note.start_tick > tick || tick >= note.end_tick {
                continue;
            }
            let idx = note.track as usize;
            if idx >= n {
                continue;
            }
            if Some(idx as u16) == conductor_track_idx {
                continue;
            }
            if idx >= pr_visible.len() || !pr_visible[idx] {
                continue;
            }
            if is_muted(idx) {
                continue;
            }
            if is_dense(idx) {
                continue;
            }
            // 全局无可听音符的轨道可跳过（由 rebuild_dirty 维护，过滤纯力度1轨道）
            if model.track_audible_count.get(idx).copied().unwrap_or(0) == 0 {
                continue;
            }
            if !present[idx] {
                present[idx] = true;
                any = true;
            }
        }
        if !any {
            continue;
        }
        for idx in 0..n {
            if present[idx] {
                keys_by_track_sim[idx].push(key);
            }
        }
    }

    // 同时发声已能覆盖块状和弦，为降低延迟与开销：若 sim 已有 pcs>=3 的多音和弦，直接返回，不再跑窗口。
    // 仅当 sim 无多音时才做分解和弦回退，回退窗口仅 1 拍（ppq），lookback 避免前瞻预测。
    // 先评估 sim 的 best_multi，若已足够好则提前返回。
    let best_sim: Option<(f64, u8, u16, String)> = {
        let mut tmp_multi: Option<(f64, u8, u16, String)> = None;
        for (idx, keys) in keys_by_track_sim.iter().enumerate() {
            if keys.is_empty() {
                continue;
            }
            if keys.len() == 1 {
                continue;
            }
            if let Some(name) = recognize(keys) {
                let mut mask = 0u16;
                for &k in keys {
                    mask |= 1 << (k % 12);
                }
                let pcs = mask.count_ones() as f64;
                let per_bar = per_bar_vec[idx];
                let score = pcs * 1000.0 / (per_bar + 10.0);
                let prio = if Some(idx as u16) == main_track { 0 } else { 1 };
                match &tmp_multi {
                    None => tmp_multi = Some((score, prio, idx as u16, name.clone())),
                    Some((best_score, best_prio, _, _)) => {
                        if score > *best_score + f64::EPSILON
                            || (score - *best_score).abs() < f64::EPSILON && prio < *best_prio
                        {
                            tmp_multi = Some((score, prio, idx as u16, name.clone()));
                        }
                    }
                }
            }
        }
        tmp_multi
    };
    if let Some((score, _, _, ref name)) = best_sim {
        // 块状和弦已足够完整则立即返回，避免窗口回退带来的延迟与额外扫描
        if score > 30.0 {
            return Some(name.clone());
        }
    }

    // 分解和弦回退：窗口内起音聚合（lookback 1拍），即使不同时也能还原和弦。
    let mut keys_by_track_win: Vec<Vec<u8>> = vec![Vec::new(); n];
    {
        let window = model.meta.ppq; // 1拍，延迟约 1拍/2，随 ppq 自适应
        let lo = tick.saturating_sub(window);
        let hi = tick.saturating_add(1);
        for key in 0u8..128 {
            let mut present = [false; 256];
            let mut any = false;
            for note in model.key_notes(key).range(lo, hi) {
                if note.velocity <= 1 {
                    continue;
                }
                if note.start_tick < lo || note.start_tick >= hi {
                    continue;
                }
                if note.end_tick.saturating_sub(note.start_tick) <= 60 {
                    continue;
                }
                let idx = note.track as usize;
                if idx >= n {
                    continue;
                }
                if Some(idx as u16) == conductor_track_idx {
                    continue;
                }
                if idx >= pr_visible.len() || !pr_visible[idx] {
                    continue;
                }
                if is_muted(idx) {
                    continue;
                }
                if is_dense(idx) {
                    continue;
                }
                if model.track_audible_count.get(idx).copied().unwrap_or(0) == 0 {
                    continue;
                }
                if !present[idx] {
                    present[idx] = true;
                    any = true;
                }
            }
            if !any {
                continue;
            }
            for idx in 0..n {
                if present[idx] {
                    keys_by_track_win[idx].push(key);
                }
            }
        }
    }

    // 密集轨 per_bar 高，pcs 相同则稀疏音乐轨胜出，通用任何歌曲；单音已在 recognize 层过滤
    let mut best_multi: Option<(f64, u8, u16, String)> = None; // (score, priority, track, name)
    for idx in 0..n {
        let idx_u16 = idx as u16;
        let priority: u8 = if Some(idx_u16) == main_track { 0 } else { 1 };
        let per_bar = per_bar_vec[idx];
        let mut candidates: Vec<&Vec<u8>> = Vec::new();
        if !keys_by_track_sim[idx].is_empty() {
            candidates.push(&keys_by_track_sim[idx]);
        }
        if !keys_by_track_win[idx].is_empty() {
            candidates.push(&keys_by_track_win[idx]);
        }
        for keys in candidates {
            if keys.len() < 2 {
                continue;
            }
            if let Some(name) = recognize(keys) {
                let mut mask = 0u16;
                for &k in keys {
                    mask |= 1 << (k % 12);
                }
                let pcs = mask.count_ones() as f64;
                let score = pcs * 1000.0 / (per_bar + 10.0);
                match &best_multi {
                    None => best_multi = Some((score, priority, idx_u16, name.clone())),
                    Some((best_score, best_prio, _, _)) => {
                        if score > *best_score + f64::EPSILON
                            || (score - *best_score).abs() < f64::EPSILON && priority < *best_prio
                        {
                            best_multi = Some((score, priority, idx_u16, name.clone()));
                        }
                    }
                }
            }
        }
    }
    if let Some((_, _, _, name)) = best_multi {
        return Some(name);
    }
    None
}
