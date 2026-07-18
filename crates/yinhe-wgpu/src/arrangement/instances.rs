use std::collections::HashSet;

use rayon::prelude::*;
use yinhe_types::{key_notes_in_range, NoteSource, TimeSigEvent};

use yinhe_types::ArrangementView;
use crate::grid;
use crate::vertex::{DrawInstance, NoteInstance};

/// Stack red zone threshold. When stack usage exceeds this, `stacker` will
/// allocate a new stack segment before calling the closure.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size to allocate when the red zone is exceeded.
const STACK_SIZE: usize = 1024 * 1024; // 1MB per segment

/// Build grid line instances (layer 0).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &ArrangementView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
    theme: &yinhe_theme::GpuTheme,
) {
    grid::build_timeline_grid(
        out,
        w,
        h,
        &view.base,
        tpb,
        default_num,
        default_den,
        time_sig_events,
        theme.ar_measure_line,
        theme.ar_beat_line,
        None,
        scroll_x_pixel,
    );
}

/// 子像素合并 + 视口裁剪 + track_visible 过滤，输出 NoteInstance。
///
/// 共享给 `build_notes`（固定层）和 `build_ghost_notes`（拖拽预览层），
/// 保证两者像素级一致。
///
/// `entries`：同一 (key, track) 下按 start_tick 升序的 (start, end, vel)。
/// 合并条件：相邻条目间距 ≤ `merge_gap_ticks` 个 tick（≤1 像素时合并）。
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
            reserved: 0,
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
/// Dependencies: track_visible, tick range (scroll_x), track range (scroll_y)
///
/// Output is 16B `NoteInstance` (semantic data only: ticks, key, track, vel).
/// All pixel positions and colors are computed in the GPU vertex shader:
///   - x/w: ticks → pixels via ppu + scroll_x (same as before)
///   - y/h: shader computes from lane_height + scroll_y + key + track
/// This means scroll_y changes do NOT invalidate the cache (same optimization
/// as PR notes).
///
/// GPU clips off-screen notes.
///
/// Uses `stacker::maybe_grow` to dynamically allocate new stack segments
/// when the current stack is close to overflowing.
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &ArrangementView,
    track_visible: &[bool],
    hidden_notes: &HashSet<(u16, u32, u8)>,
) {
    let ppu = view.base.pixels_per_tick;
    let num_tracks = track_visible.len();
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);

    let note_instances: Vec<Vec<NoteInstance>> = (0u8..128)
        .into_par_iter()
        .filter_map(|key| {
            // Wrap key processing in stacker to get fresh stack segments on demand.
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = key_notes_in_range(midi.key_notes(key), tick_start as u32, tick_end as u32);
                if notes.is_empty() {
                    return None;
                }

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

                for ti in trk_first..trk_last {
                    flush_track_bucket(
                        &mut local,
                        track_buckets[ti].iter().copied(),
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
/// (key, track, start_tick) 排序以供合并。vel 恒为 0（AR 不使用 velocity）。
pub fn build_ghost_notes(
    out: &mut Vec<NoteInstance>,
    ghost_notes: &mut [(f64, f64, u8, u16)],
    w: f32,
    h: f32,
    view: &ArrangementView,
    track_visible: &[bool],
) {
    if ghost_notes.is_empty() {
        return;
    }
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let num_tracks = track_visible.len();
    let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
    let merge_gap_ticks = (1.0 / ppu).ceil() as u32;

    // 原地按 (key, track, start_tick) 排序，使同一 (key, track) 连续且升序。
    ghost_notes.sort_by(|a, b| {
        a.2.cmp(&b.2)
            .then(a.3.cmp(&b.3))
            .then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 遍历每个 (key, track) 分组，合并 + 裁剪 + push。
    let mut i = 0;
    while i < ghost_notes.len() {
        let key = ghost_notes[i].2;
        let track = ghost_notes[i].3;
        let mut j = i + 1;
        while j < ghost_notes.len() && ghost_notes[j].2 == key && ghost_notes[j].3 == track {
            j += 1;
        }
        let bucket = ghost_notes[i..j].iter().map(|&(s, e, _, _)| {
            let s = s.max(0.0) as u32;
            let e = e.max(s as f64).max(0.0) as u32;
            (s, e, 0u8)
        });
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

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_midi;

    #[test]
    fn test_no_duplicate_grid_lines_at_time_sig_boundary() {
        let _mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let tpb = 480;
        let mut view = ArrangementView::default();
        view.base.pixels_per_tick = 0.5;
        view.base.left_panel_width = 0.0;
        view.base.scroll_x = 0.0;
        view.base.dirty = true;

        let events = vec![
            TimeSigEvent {
                tick: 0,
                numerator: 4,
                denominator: 2,
            },
            TimeSigEvent {
                tick: 1920,
                numerator: 7,
                denominator: 2,
            },
        ];

        let mut grid_lines = Vec::new();
        build_grid(&mut grid_lines, 2000.0, 400.0, &view, tpb, 4, 2, &events, 0.0, &yinhe_theme::GpuTheme::default());

        let ticks: Vec<u32> = grid_lines.iter().map(|i| i.tag).collect();

        let mut sorted = ticks.clone();
        sorted.sort();
        let deduped = {
            let mut d = sorted.clone();
            d.dedup();
            d
        };
        assert_eq!(
            sorted.len(),
            deduped.len(),
            "Duplicate grid lines at same tick!\n  ticks: {:?}\n  sorted: {:?}",
            ticks,
            sorted,
        );

        let count_1920 = ticks.iter().filter(|&&t| t == 1920).count();
        assert_eq!(
            count_1920, 1,
            "Boundary tick 1920 must appear exactly once, got {}",
            count_1920
        );
    }
}
