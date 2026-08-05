use rayon::prelude::*;
use yinhe_types::AutomationPanelView;
use yinhe_types::NoteSource;

use crate::vertex::VelocityBarInstance;

/// Stack red zone threshold for stacker.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size for stacker.
const STACK_SIZE: usize = 1024 * 1024;

/// Build velocity bar instances from NoteSource (automation panel, velocity mode).
///
/// Outputs `VelocityBarInstance` (16B) — semantic data only (tick, length, track,
/// velocity). All pixel positions and colors are computed on the GPU in
/// `vs_main_velocity` from uniforms + track_colors storage buffer.
///
/// Unified border-based mode (fill + border), same as note rendering.
/// No occlusion sorting: border ensures visibility for overlapping bars.
/// A simple (tick, track) sort is applied for deterministic frame-to-frame output
/// (rayon parallel collection order is otherwise non-deterministic).
///
/// Uses `stacker::maybe_grow` to prevent stack overflow when processing
/// many notes at very low zoom levels.
///
/// 去重规则（velocity 面板所有 bar 在同一行——track 只影响颜色）：
/// - 同 (tick, gate, vel) 完全重叠的 bar（同轨复制重叠 / 多轨和弦）只画一个；
/// - 同 vel（同高）的 bar：短 gate 后画（顶层），被一组更短的同 vel bar
///   完全覆盖的长 bar 不画，露出部分仍保留；
/// - vel 不同的 bar 只部分重叠（高的露头、矮的可见），全部保留。
pub fn build_velocity_bars(
    out: &mut Vec<VelocityBarInstance>,
    w: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
) {
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;

    let mut bars: Vec<VelocityBarInstance> = (0u8..128)
        .into_par_iter()
        .flat_map_iter(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let mut local: Vec<VelocityBarInstance> = Vec::new();
                let notes = midi.key_notes_in_range(key, pad_start, pad_end);
                for note in notes {
                    if note.start_tick as f64 > pad_end as f64 {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start as f64 {
                        continue;
                    }
                    let trk_idx = note.track as usize;
                    if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                        continue;
                    }
                    local.push(VelocityBarInstance {
                        tick: note.start_tick,
                        length: note.end_tick - note.start_tick,
                        packed: VelocityBarInstance::pack(note.track, note.velocity),
                        reserved: 0,
                    });
                }
                local
            })
        })
        .collect();

    // Deterministic z-order：主键 vel DESC（大 vel 底层先画，小 vel 顶层后画）——
    // 跨 tick 也成立：tick 靠后的大 vel bar 不会盖住前面未放完的小 vel bar；
    // 次键 gate DESC（同力度时短 gate 顶层后画）→ tick ASC → track ASC。
    bars.sort_by(|a, b| {
        b.velocity()
            .cmp(&a.velocity())
            .then(b.length.cmp(&a.length))
            .then(a.tick.cmp(&b.tick))
            .then(a.track().cmp(&b.track()))
    });

    dedup_overlapped(&mut bars);

    out.extend(bars);
}

/// 按 (vel, gate) 优先级去重完全被覆盖的 bar（见 [`build_velocity_bars`] 的规则）。
///
/// 覆盖检测按 vel 分桶：同 vel 的 bar 同高，短 gate 的后画（顶层）；
/// 桶内按 gate ASC 处理，维护已覆盖的 tick 区间并集（合并、按起点有序），
/// 区间完全在并集内的 bar 不可见。不同 vel 的 bar 只部分重叠，不处理。
/// 同 (tick, gate, vel) 的完全重复被区间覆盖自然去重（第二个的区间已在并集内）。
fn dedup_overlapped(bars: &mut Vec<VelocityBarInstance>) {
    let mut buckets: [Vec<u32>; 128] = core::array::from_fn(|_| Vec::new());
    for (i, b) in bars.iter().enumerate() {
        buckets[b.velocity() as usize].push(i as u32);
    }
    let mut keep = vec![true; bars.len()];
    for bucket in buckets.iter_mut() {
        if bucket.len() < 2 {
            continue;
        }
        // 短 gate 优先（顶层后画）：gate ASC，同 gate 按 tick DESC——
        // 同 gate 的 bar 互不覆盖（同高同宽、tick 偏移只部分重叠），但 tick 更
        // 晚的 bar 可能是覆盖长 bar 的联合成员，先入并集才能让被联合覆盖的
        // 长 bar 正确判删。
        bucket.sort_by_key(|&i| {
            let b = &bars[i as usize];
            (b.length, std::cmp::Reverse(b.tick))
        });
        // 已覆盖的 tick 区间（合并后不重叠、按起点有序）。
        let mut covered: Vec<(u32, u32)> = Vec::new();
        for &i in bucket.iter() {
            let b = &bars[i as usize];
            let s = b.tick;
            let e = b.tick + b.length;
            if is_fully_covered(&covered, s, e) {
                keep[i as usize] = false;
            } else {
                insert_interval(&mut covered, s, e);
            }
        }
    }
    // 原位过滤（保持 z-order 顺序）。
    let mut w = 0;
    for r in 0..bars.len() {
        if keep[r] {
            bars[w] = bars[r];
            w += 1;
        }
    }
    bars.truncate(w);
}

/// [s, e) 是否完全落在已覆盖区间并集内（covered 合并后不重叠、按起点有序）。
fn is_fully_covered(covered: &[(u32, u32)], s: u32, e: u32) -> bool {
    if e <= s {
        return false; // 空区间：无像素，不参与覆盖判定
    }
    let idx = covered.partition_point(|&(cs, _)| cs <= s);
    if idx == 0 {
        return false;
    }
    let (cs, ce) = covered[idx - 1];
    cs <= s && ce >= e
}

/// 把 [s, e) 插入 covered 并合并所有重叠区间（保持有序、无重叠）。
fn insert_interval(covered: &mut Vec<(u32, u32)>, s: u32, e: u32) {
    let start_idx = covered.partition_point(|&(cs, _)| cs < s);
    let mut new_s = s;
    let mut new_e = e;
    let mut first = start_idx;
    // 前一个区间与 [s, e) 重叠（起点更早、终点 >= s）。
    if start_idx > 0 && covered[start_idx - 1].1 >= s {
        first = start_idx - 1;
        new_s = covered[first].0;
    }
    let mut last = first;
    while last < covered.len() && covered[last].0 <= new_e {
        new_e = new_e.max(covered[last].1);
        last += 1;
    }
    covered.splice(first..last, [(new_s, new_e)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{Note, NoteSource};

    /// 测试音符全部放在 key 60（velocity 面板不分 key）。
    struct MockSource {
        notes: Vec<Note>, // 按 start_tick 升序
    }

    impl NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &[Note] {
            if key == 60 { &self.notes } else { &[] }
        }

        fn duration(&self) -> f64 {
            10.0
        }
    }

    fn make_bar(tick: u32, gate: u32, vel: u8, track: u16) -> Note {
        Note {
            id: 0,
            start_tick: tick,
            end_tick: tick + gate,
            velocity: vel,
            track,
        }
    }

    fn build(notes: Vec<Note>) -> Vec<(u32, u32, u8, u16)> {
        let src = MockSource { notes };
        let view = AutomationPanelView::default();
        let tv = vec![true; 4];
        let mut out = Vec::new();
        build_velocity_bars(&mut out, 800.0, &src, &view, &tv);
        out.into_iter()
            .map(|b| (b.tick, b.length, b.velocity(), b.track()))
            .collect()
    }

    #[test]
    fn dedup_identical_bars_across_tracks() {
        // 同 tick 同 gate 同 vel，不同 track（多轨和弦）→ 只画一个。
        let out = build(vec![
            make_bar(100, 10, 100, 0),
            make_bar(100, 10, 100, 1),
            make_bar(100, 10, 100, 2),
        ]);
        assert_eq!(out.len(), 1, "完全重复只画一个: {out:?}");
    }

    #[test]
    fn dedup_short_bars_cover_long_bar() {
        // 同 vel：短 bar [100,110) + [110,160) 完全覆盖长 bar [100,150) → 长 bar 删除。
        let out = build(vec![
            make_bar(100, 50, 80, 0), // 长
            make_bar(100, 10, 80, 1),
            make_bar(110, 50, 80, 2),
        ]);
        assert_eq!(out.len(), 2, "长 bar 被完全覆盖应删除: {out:?}");
        assert!(
            out.iter().all(|&(t, g, _, _)| !(t == 100 && g == 50)),
            "长 bar 不应保留: {out:?}"
        );
    }

    #[test]
    fn keep_partially_covered_long_bar() {
        // 同 vel：短 bar [100,110) 只盖住长 bar [100,150) 的左边 → 长 bar 露出右侧，保留。
        let out = build(vec![make_bar(100, 50, 80, 0), make_bar(100, 10, 80, 1)]);
        assert_eq!(out.len(), 2, "部分覆盖都要画: {out:?}");
    }

    #[test]
    fn keep_different_velocity_bars() {
        // vel 不同（100 与 50）：高的露头、矮的可见，都保留。
        let out = build(vec![make_bar(100, 10, 100, 0), make_bar(100, 20, 50, 1)]);
        assert_eq!(out.len(), 2, "不同 vel 全画: {out:?}");
    }

    #[test]
    fn keep_non_overlapping_bars() {
        let out = build(vec![make_bar(100, 10, 80, 0), make_bar(200, 10, 80, 1)]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn zorder_large_vel_behind_small_vel() {
        // 回归：tick 靠后的大 vel bar 必须先画（底层），
        // 不能盖住前面未放完（gate 长）的小 vel bar。
        let out = build(vec![
            make_bar(100, 200, 20, 0), // 小 vel 长 bar，未放完
            make_bar(150, 10, 100, 1), // tick 靠后的大 vel 短 bar
        ]);
        assert_eq!(out.len(), 2);
        // 输出顺序 = 绘制顺序：vel DESC → 大 vel（100）在前（底层）。
        assert_eq!(out[0].2, 100, "大 vel 应先画（底层）: {out:?}");
        assert_eq!(out[1].2, 20, "小 vel 后画（顶层）: {out:?}");
    }

    #[test]
    fn covered_intervals_merge() {
        // 覆盖区间合并：两个相邻短 bar 合并后覆盖中间的缺口。
        let out = build(vec![
            make_bar(100, 30, 60, 0), // [100,130)
            make_bar(100, 5, 60, 1),  // [100,105)
            make_bar(105, 30, 60, 2), // [105,135) 与 [100,105) 接续
        ]);
        // [100,135) 完全覆盖 [100,130) → 长 bar 删除，两个短 bar 保留。
        assert_eq!(out.len(), 2, "合并覆盖应删除长 bar: {out:?}");
    }

    /// 真实 MIDI 去重效果统计：视口内音符数（去重前 bar 数）vs 去重后 bar 数。
    /// 运行：cargo test -p yinhe-wgpu --release -- --ignored --nocapture dedup_real_midi
    #[test]
    #[ignore]
    fn dedup_real_midi_stats() {
        let path = std::env::var("YIN_BENCH_MIDI")
            .unwrap_or_else(|_| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
        let model = yinhe_mid2::parse_path(&path).expect("parse 失败");
        // 全曲视口：ppu=0.01 → 31000px 覆盖 309 万 tick。
        let (ppu, w) = (0.01f32, 31000.0f32);
        let view = AutomationPanelView {
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 0.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            ..Default::default()
        };
        let tv = vec![true; model.tracks.len()];
        let (ts, te) = view.base.visible_tick_range(w);
        let before: u64 = (0..128u8)
            .map(|k| model.key_notes_in_range(k, ts as u32, te as u32).len() as u64)
            .sum();
        let t0 = std::time::Instant::now();
        let mut out = Vec::new();
        build_velocity_bars(&mut out, w, &model, &view, &tv);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "{}: 去重前 bar={before} 去重后={} 保留率={:.1}% 构建+去重耗时={ms:.0}ms",
            path.rsplit('/').next().unwrap_or(path.as_str()),
            out.len(),
            out.len() as f64 * 100.0 / before.max(1) as f64,
        );

        // 真实滚动视口（350 小节附近，ppu=0.026，宽 1376px）：
        // 滚动时 bars_key 失配，每帧重建。测每帧构建+排序+去重成本。
        let (w2, ppu2) = (1376.0f32, 0.026372144f32);
        let kb = 60.0f32;
        let max_end = model.tick_length().unwrap_or(0).max(1) as f32;
        let scroll_x2 = (kb + max_end * ppu2 * 0.87 - w2 / 2.0).max(0.0);
        let view2 = AutomationPanelView {
            base: yinhe_types::TimelineViewBase {
                pixels_per_tick: ppu2,
                scroll_x: scroll_x2,
                scroll_y: 0.0,
                left_panel_width: kb,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            ..Default::default()
        };
        let mut out2 = Vec::new();
        // 暖机 1 次后取最优（3 次）。
        build_velocity_bars(&mut out2, w2, &model, &view2, &tv);
        let mut frame_ms = f64::MAX;
        for _ in 0..3 {
            out2.clear();
            let t = std::time::Instant::now();
            build_velocity_bars(&mut out2, w2, &model, &view2, &tv);
            frame_ms = frame_ms.min(t.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "真实视口(87%): 视口内 bar={} 去重后={} 每帧构建+排序+去重={frame_ms:.1}ms ≈ {:.0} FPS",
            before as f64 * (w2 / ppu2 / 3_092_040.0f32) as f64,
            out2.len(),
            1000.0 / frame_ms,
        );
    }
}
