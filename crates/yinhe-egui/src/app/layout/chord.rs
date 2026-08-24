#![allow(unused_imports)]
use yinhe_core::YinModel;
use yinhe_types::{NoteSource, measure_ticks};

/// 和弦指示器文本：实时 MIDI 直通按键优先；无按键且播放中 → 播放头处发声音符的和弦。
///
/// 播放路径逐轨识别：主轨优先、PR 可见次之，跳过静音/不可见/力度≤1 的轨道，
/// 在所有候选轨道中择最完善的多音和弦（音级数最多），无多音和弦时回退单音。
/// 每轨内部复用 `chord::recognize` 的防乱按规则（音级 > 7 或跨度 > 2 八度不识别）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn chord_indicator_text(
    thru_keys: &std::collections::HashMap<u8, u8>,
    is_playing: bool,
    cursor_tick: Option<f64>,
    model: Option<&yinhe_core::YinModel>,
    main_track: Option<u16>,
    pr_visible: &[bool],
    track_overrides: &[yinhe_editor_core::document::TrackOverride],
    conductor_track_idx: Option<u16>,
) -> Option<String> {
    if !thru_keys.is_empty() {
        let mut keys: Vec<u8> = thru_keys.keys().copied().collect();
        keys.sort_unstable();
        return yinhe_editor_core::chord::recognize(&keys);
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
            if let Some(name) = yinhe_editor_core::chord::recognize(keys) {
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
            if let Some(name) = yinhe_editor_core::chord::recognize(keys) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_test_document;

    fn chord_tick(doc: &yinhe_editor_core::document::Document, tick: f64) -> Option<String> {
        let main = doc.edit.main_track();
        chord_indicator_text(
            &std::collections::HashMap::new(),
            true,
            Some(tick),
            Some(doc.model()),
            main,
            &doc.edit.track_pianoroll_visible,
            &doc.edit.track_overrides,
            doc.edit.conductor_track_idx,
        )
    }

    fn add_chord(
        doc: &mut yinhe_editor_core::document::Document,
        track: u16,
        keys: &[u8],
        tick: u32,
        vel: u8,
    ) {
        for &key in keys {
            let ev = yinhe_core::NoteEvent {
                id: 0,
                start_tick: tick,
                end_tick: tick + 480,
                key,
                velocity: vel,
            };
            doc.add_note(track, ev).expect("add_note");
        }
    }

    #[test]
    fn chord_per_track_prefers_main_over_visible_when_same_pcs() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[67, 71, 74], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_picks_most_complete_across_tracks() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should recognize");
        assert_eq!(chord, "Cmaj7");
    }

    #[test]
    fn chord_skips_muted_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        doc.edit.track_overrides[2].muted = true;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_invisible_track() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 100);
        doc.edit.track_pianoroll_visible[2] = false;
        let chord = chord_tick(&doc, 0.0).expect("should fallback");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_skips_velocity_one_notes() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60, 64, 67], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67, 71], 0, 1);
        let chord = chord_tick(&doc, 0.0).expect("should pick audible");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_per_track_anti_garbage_global_span_would_fail() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[36, 40, 43], 0, 100);
        add_chord(&mut doc, 2, &[84, 88, 91], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("per-track should still recognize");
        assert_eq!(chord, "C");
    }

    #[test]
    fn chord_prefers_multi_over_single() {
        let mut doc = yinhe_editor_core::document::Document::empty();
        doc.edit.track_selected.clear();
        doc.edit.track_selected.insert(1);
        add_chord(&mut doc, 1, &[60], 0, 100);
        add_chord(&mut doc, 2, &[60, 64, 67], 0, 100);
        let chord = chord_tick(&doc, 0.0).expect("should prefer multi");
        assert_eq!(chord, "C");
    }
}
