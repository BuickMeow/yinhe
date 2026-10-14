use std::collections::HashSet;

use rayon::prelude::*;
use yinhe_types::NoteSource;

use crate::vertex::NoteInstance;
use yinhe_types::ArrangementView;

/// Stack red zone threshold. When stack usage exceeds this, `stacker` will
/// allocate a new stack segment before calling the closure.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size to allocate when the red zone is exceeded.
const STACK_SIZE: usize = 1024 * 1024; // 1MB per segment

/// 子像素合并 + 视口裁剪 + track_visible 过滤，输出 NoteInstance。
///
/// 共享给 `build_notes`（固定层）和 `build_ghost_notes`（拖拽预览层），
/// 保证两者像素级一致。
///
/// `entries`：同一 (key, track) 下按 start_tick 升序的 (start, end, vel)。
/// 合并条件：相邻条目间距 ≤ `merge_gap_ticks` 个 tick（≤1 像素时合并）。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn flush_track_bucket(
    out: &mut Vec<NoteInstance>,
    entries: impl IntoIterator<Item = (u32, u32, u8)>,
    key: u8,
    track: usize,
    tick_start: f64,
    tick_end: f64,
    trk_first: usize,
    trk_last: usize,
    track_visible: &[bool],
    merge_gap_ticks: u32,
) {
    if track < trk_first || track >= trk_last {
        return;
    }
    if !track_visible.get(track).copied().unwrap_or(true) {
        return;
    }

    let flush = |out: &mut Vec<NoteInstance>, start: u32, end: u32, vel: u8| {
        let s = (start as f64).max(tick_start) as u32;
        let e = (end as f64).min(tick_end).max(start as f64) as u32;
        if s >= e {
            return;
        }
        out.push(NoteInstance {
            start_tick: s,
            end_tick: e,
            packed: NoteInstance::pack(key, track.min(65535) as u16, vel),
        });
    };

    // 合并相邻条目，gap ≤ merge_gap_ticks 时合并 end/vel。
    let mut state: Option<(u32, u32, u8)> = None;
    for (s, e, v) in entries {
        match state {
            None => state = Some((s, e, v)),
            Some((ms, me, mv)) => {
                if s <= me + merge_gap_ticks {
                    state = Some((ms, me.max(e), mv.max(v)));
                } else {
                    flush(out, ms, me, mv);
                    state = Some((s, e, v));
                }
            }
        }
    }
    if let Some((ms, me, mv)) = state {
        flush(out, ms, me, mv);
    }
}

/// Build note rectangle instances with sub-pixel merging (layer 2).
/// Dependencies: track_visible, tick range (scroll_x), track range
///
/// track_range：可见音轨范围 [first, last)，由调用方按当前行布局
/// （含展开的自动化 lane）计算——均匀行假设下可用
/// ArrangementView::visible_track_range，展开后须用 ArRowLayout。
///
/// Output is 16B `NoteInstance` (semantic data only: ticks, key, track, vel).
/// All pixel positions and colors are computed in the GPU vertex shader:
///   - x/w: ticks → pixels via ppu + scroll_x (same as before)
///   - y/h: shader computes from lane_height + scroll_y + key + track
///     This means scroll_y changes do NOT invalidate the cache (same optimization
///     as PR notes).
///
/// GPU clips off-screen notes.
///
/// Uses `stacker::maybe_grow` to dynamically allocate new stack segments
/// when the current stack is close to overflowing.
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    midi: &dyn NoteSource,
    view: &ArrangementView,
    track_range: (usize, usize),
    track_visible: &[bool],
    hidden_notes: &HashSet<(u16, u32, u8)>,
) {
    let ppu = view.base.pixels_per_tick;
    let num_tracks = track_visible.len();
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (trk_first, trk_last) = track_range;

    let note_instances: Vec<Vec<NoteInstance>> = (0u8..=yinhe_types::MAX_KEY)
        .into_par_iter()
        .filter_map(|key| {
            // Wrap key processing in stacker to get fresh stack segments on demand.
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = midi.key_notes_in_range(key, tick_start as u32, tick_end as u32);

                let mut local = Vec::new();

                let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

                let mut track_buckets: Vec<Vec<(u32, u32, u8)>> = vec![Vec::new(); num_tracks];
                for note in notes {
                    if note.start_tick as f64 > tick_end {
                        break;
                    }
                    if (note.end_tick as f64) < tick_start {
                        continue;
                    }
                    let ti = note.track as usize;
                    if ti < trk_first || ti >= trk_last {
                        continue;
                    }
                    if !track_visible.get(ti).copied().unwrap_or(true) {
                        continue;
                    }
                    if hidden_notes.contains(&(note.track, note.start_tick, key)) {
                        continue;
                    }
                    track_buckets[ti].push((note.start_tick, note.end_tick, note.velocity));
                }

                for (ti, bucket) in track_buckets
                    .iter()
                    .enumerate()
                    .take(trk_last)
                    .skip(trk_first)
                {
                    flush_track_bucket(
                        &mut local,
                        bucket.iter().copied(),
                        key,
                        ti,
                        tick_start,
                        tick_end,
                        trk_first,
                        trk_last,
                        track_visible,
                        merge_gap_ticks,
                    );
                }

                if local.is_empty() { None } else { Some(local) }
            })
        })
        .collect();

    out.extend(note_instances.into_iter().flatten());
}

/// Build ghost note instances for move-drag preview (layer 3, no cache).
///
/// 与 `build_notes` 走同一条 `flush_track_bucket` 路径，因此 ghost 也会有
/// 子像素合并、tick/track 视口裁剪、track_visible 过滤，像素级与固定层一致。
///
/// `ghost_notes`：`(start_tick, end_tick, key, track)`，会被原地按
/// (key, track, start_tick) 排序以供合并。vel 恒为 127（= 原色，
/// 不参与力度着色，见 shader `MAX_FILL_LIGHTEN`）。
pub fn build_ghost_notes(
    out: &mut Vec<NoteInstance>,
    ghost_notes: &mut [(u32, u32, u8, u16)],
    w: f32,
    view: &ArrangementView,
    track_range: (usize, usize),
    track_visible: &[bool],
) {
    if ghost_notes.is_empty() {
        return;
    }
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (trk_first, trk_last) = track_range;
    let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

    // 原地按 (key, track, start_tick) 排序，使同一 (key, track) 连续且升序。
    ghost_notes.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)).then(a.0.cmp(&b.0)));

    // 遍历每个 (key, track) 分组，合并 + 裁剪 + push。
    let mut i = 0;
    while i < ghost_notes.len() {
        let key = ghost_notes[i].2;
        let track = ghost_notes[i].3;
        let mut j = i + 1;
        while j < ghost_notes.len() && ghost_notes[j].2 == key && ghost_notes[j].3 == track {
            j += 1;
        }
        let bucket = ghost_notes[i..j]
            .iter()
            .map(|&(s, e, _, _)| (s, e.max(s), 127u8));
        flush_track_bucket(
            out,
            bucket,
            key,
            track as usize,
            tick_start,
            tick_end,
            trk_first,
            trk_last,
            track_visible,
            merge_gap_ticks,
        );
        i = j;
    }
}

/// 通道 AR（MIX 主区域）音符实例构建：按「行」分桶 + 子像素合并。
///
/// `track_rows`：每轨道的音符需要绘制的行 id 列表（index = 轨道索引）。
/// 折叠通道 → 该通道全部轨道指向同一合并行（跨轨合并）；展开 → 合并行
/// （变窄总览）+ 各轨道行。输出 `instance.track = 行 id`，shader 用
/// `track_offsets[row]` / `row_heights[row]` 定位、`track_colors[row]` 着色，
/// 因此调用方需按行 id 组织 offsets/heights/colors 三张表。
pub fn build_channel_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    midi: &dyn NoteSource,
    view: &ArrangementView,
    track_rows: &[Vec<u16>],
    track_visible: &[bool],
    hidden_notes: &HashSet<(u16, u32, u8)>,
) {
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

    let note_instances: Vec<Vec<NoteInstance>> = (0u8..=yinhe_types::MAX_KEY)
        .into_par_iter()
        .filter_map(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = midi.key_notes_in_range(key, tick_start as u32, tick_end as u32);

                // (行 id, start, end, vel)；同一音符展开时可能入多个行。
                let mut entries: Vec<(u16, u32, u32, u8)> = Vec::new();
                for note in notes {
                    if note.start_tick as f64 > tick_end {
                        break;
                    }
                    if (note.end_tick as f64) < tick_start {
                        continue;
                    }
                    let ti = note.track as usize;
                    if !track_visible.get(ti).copied().unwrap_or(true) {
                        continue;
                    }
                    if hidden_notes.contains(&(note.track, note.start_tick, key)) {
                        continue;
                    }
                    let Some(rows) = track_rows.get(ti) else {
                        continue;
                    };
                    for &row in rows {
                        entries.push((row, note.start_tick, note.end_tick, note.velocity));
                    }
                }
                if entries.is_empty() {
                    return None;
                }
                // 按 (行, start) 排序：合并桶内条目相邻且升序。
                entries.sort_unstable_by_key(|e| (e.0, e.1));

                let mut local = Vec::new();
                let mut i = 0;
                while i < entries.len() {
                    let row = entries[i].0;
                    let mut state: Option<(u32, u32, u8)> = None;
                    while i < entries.len() && entries[i].0 == row {
                        let (_, s, e, v) = entries[i];
                        match state {
                            None => state = Some((s, e, v)),
                            Some((ms, me, mv)) => {
                                if s <= me + merge_gap_ticks {
                                    state = Some((ms, me.max(e), mv.max(v)));
                                } else {
                                    flush_row_entry(
                                        &mut local, ms, me, mv, key, row, tick_start, tick_end,
                                    );
                                    state = Some((s, e, v));
                                }
                            }
                        }
                        i += 1;
                    }
                    if let Some((ms, me, mv)) = state {
                        flush_row_entry(&mut local, ms, me, mv, key, row, tick_start, tick_end);
                    }
                }
                if local.is_empty() { None } else { Some(local) }
            })
        })
        .collect();

    out.extend(note_instances.into_iter().flatten());
}

/// 单个已合并条目的裁剪 + push（行 id 直通为 instance.track）。
#[allow(clippy::too_many_arguments)]
fn flush_row_entry(
    out: &mut Vec<NoteInstance>,
    start: u32,
    end: u32,
    vel: u8,
    key: u8,
    row: u16,
    tick_start: f64,
    tick_end: f64,
) {
    let s = (start as f64).max(tick_start) as u32;
    let e = (end as f64).min(tick_end).max(start as f64) as u32;
    if s >= e {
        return;
    }
    out.push(NoteInstance {
        start_tick: s,
        end_tick: e,
        packed: NoteInstance::pack(key, row, vel),
    });
}

/// 通道 AR 拖拽 ghost 实例构建（layer 1，无缓存）。
///
/// `ghost_notes`：`(start_tick, end_tick, key, 行 id)`，会被原地按
/// `(key, 行 id, start_tick)` 排序以供合并。vel 恒为 127（原色）。
/// 行可画性与轨道可见性由调用方在收集 ghost 时已过滤，这里不再过 `track_visible`。
pub fn build_channel_ghost_notes(
    out: &mut Vec<NoteInstance>,
    ghost_notes: &mut [(u32, u32, u8, u16)],
    w: f32,
    view: &ArrangementView,
) {
    if ghost_notes.is_empty() {
        return;
    }
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

    ghost_notes.sort_by(|a, b| a.2.cmp(&b.2).then(a.3.cmp(&b.3)).then(a.0.cmp(&b.0)));

    let mut i = 0;
    while i < ghost_notes.len() {
        let key = ghost_notes[i].2;
        let row = ghost_notes[i].3;
        let mut j = i + 1;
        while j < ghost_notes.len() && ghost_notes[j].2 == key && ghost_notes[j].3 == row {
            j += 1;
        }
        // 组内按 start 有序（排序保证），与 build_channel_notes 同一合并路径。
        let mut state: Option<(u32, u32, u8)> = None;
        for k in i..j {
            let (s, e, _, _) = ghost_notes[k];
            match state {
                None => state = Some((s, e.max(s), 127)),
                Some((ms, me, mv)) => {
                    if s <= me + merge_gap_ticks {
                        state = Some((ms, me.max(e.max(s)), mv));
                    } else {
                        flush_row_entry(out, ms, me, mv, key, row, tick_start, tick_end);
                        state = Some((s, e.max(s), 127));
                    }
                }
            }
        }
        if let Some((ms, me, mv)) = state {
            flush_row_entry(out, ms, me, mv, key, row, tick_start, tick_end);
        }
        i = j;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_midi;

    /// 回归测试：a0f4492 曾用 `notes.next()?` 做非空检查，把 range 内第一个
    /// 音符吞掉（每个 key 的可见首音符永远不渲染 → 视觉丢音符、跨轨道）。
    #[test]
    fn build_notes_keeps_first_note_of_range() {
        let midi = make_midi(vec![
            (60, 0, 100, 0, 100),
            (60, 200, 300, 0, 100),
            (60, 400, 500, 0, 100),
        ]);
        let view = ArrangementView::default();
        let mut out = Vec::new();
        build_notes(
            &mut out,
            2000.0,
            &midi,
            &view,
            (0, 1),
            &[true],
            &HashSet::new(),
        );
        let starts: Vec<u32> = out.iter().map(|n| n.start_tick).collect();
        assert!(starts.contains(&0), "range 内第一个音符被吞: {starts:?}");
        assert_eq!(out.len(), 3, "3 个音符都应被构建: {starts:?}");
    }

    fn packed_track(packed: u32) -> u16 {
        ((packed >> 8) & 0xFFFF) as u16
    }

    /// 折叠通道合并行：两条轨道的音符经同一行 id 跨轨合并（子像素间隙内）。
    #[test]
    fn build_channel_notes_merges_tracks_into_same_row() {
        // ppu 默认 0.08 → merge_gap = ceil(12.5) = 13 tick；间隙 10 ≤ 13 合并。
        let midi = make_midi(vec![(60, 0, 100, 0, 100), (60, 110, 200, 1, 60)]);
        let view = ArrangementView::default();
        let mut out = Vec::new();
        build_channel_notes(
            &mut out,
            2000.0,
            &midi,
            &view,
            &[vec![0], vec![0]],
            &[true, true],
            &HashSet::new(),
        );
        assert_eq!(out.len(), 1, "跨轨子像素间隙应合并为一条: {out:?}");
        assert_eq!(out[0].start_tick, 0);
        assert_eq!(out[0].end_tick, 200);
        assert_eq!(packed_track(out[0].packed), 0, "行 id 应为合并行 0");
        assert_eq!(out[0].velocity(), 100, "合并 vel 取 max");
    }

    /// 展开通道：同一音符复制到「合并行 + 轨道行」；跨轨只在合并行合并。
    #[test]
    fn build_channel_notes_expands_to_track_rows() {
        let midi = make_midi(vec![(60, 0, 100, 0, 100), (60, 0, 100, 1, 80)]);
        let view = ArrangementView::default();
        let mut out = Vec::new();
        build_channel_notes(
            &mut out,
            2000.0,
            &midi,
            &view,
            &[vec![0, 1], vec![0, 2]],
            &[true, true],
            &HashSet::new(),
        );
        let mut rows: Vec<u16> = out.iter().map(|n| packed_track(n.packed)).collect();
        rows.sort_unstable();
        assert_eq!(
            rows,
            vec![0, 1, 2],
            "同一 (key,start) 在合并行合并，轨道行各自一条"
        );
        assert_eq!(out.len(), 3);
    }

    /// hidden_notes 与 track_visible 过滤对合并行同样生效。
    #[test]
    fn build_channel_notes_respects_filters() {
        let midi = make_midi(vec![(60, 0, 100, 0, 100), (61, 0, 100, 1, 100)]);
        let view = ArrangementView::default();
        let mut hidden = HashSet::new();
        hidden.insert((0u16, 0u32, 60u8));
        let mut out = Vec::new();
        build_channel_notes(
            &mut out,
            2000.0,
            &midi,
            &view,
            &[vec![0], vec![0]],
            &[true, false],
            &hidden,
        );
        assert!(out.is_empty(), "轨道 0 被 hidden、轨道 1 不可见 → 无实例");
    }

    /// ghost 构建：行 id 直通 + 子像素合并。
    #[test]
    fn build_channel_ghost_notes_merges_per_row() {
        let view = ArrangementView::default();
        let mut ghosts = vec![(150u32, 250u32, 60u8, 3u16), (255, 300, 60, 3)];
        let mut out = Vec::new();
        build_channel_ghost_notes(&mut out, &mut ghosts, 2000.0, &view);
        assert_eq!(out.len(), 1, "同 (key,row) 子像素间隙应合并");
        assert_eq!(packed_track(out[0].packed), 3);
    }
}
