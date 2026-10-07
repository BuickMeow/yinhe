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
/// 最普通排列：每音符一条 bar，按 (tick ASC, velocity DESC, track ASC) 排序后
/// 直接绘制（rayon 并行收集顺序不确定，此排序保证逐帧确定性）。同 tick 的
/// 响音先画（被覆盖）、弱音后画（顶层可见）。
///
/// Uses `stacker::maybe_grow` to prevent stack overflow when processing
/// many notes at very low zoom levels.
pub fn build_velocity_bars(
    out: &mut Vec<VelocityBarInstance>,
    w: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
    summary: Option<&super::velocity_summary::VelocitySummary>,
) {
    let (tick_start, tick_end) = view.base.visible_tick_range(w);

    // LOD：ppu 很小（块宽 ≤ SUMMARY_MAX_PX）时从预计算摘要切片，
    // 每块一条 bar = 该块「gate 最长音符」的真实区间（长度仍指示 gate）。
    if let Some(summary) = summary
        && let Some(level) = summary.level_for_ppu(view.base.pixels_per_tick)
    {
        for b in summary.level(level) {
            if f64::from(b.start_tick) > tick_end {
                break;
            }
            if f64::from(b.end_tick) <= tick_start {
                continue;
            }
            out.push(VelocityBarInstance {
                tick: b.start_tick,
                length: b.end_tick.saturating_sub(b.start_tick),
                packed: VelocityBarInstance::pack(b.track, b.velocity),
                reserved: 0,
            });
        }
        return;
    }

    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;

    let mut bars: Vec<VelocityBarInstance> = (0u8..=yinhe_types::MAX_KEY)
        .into_par_iter()
        .flat_map_iter(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = midi.key_notes_in_range(key, pad_start, pad_end);
                let mut local: Vec<VelocityBarInstance> = Vec::new();
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
                    // velocity=1（MIDI 静音）不显示：面板 127 级 → 126 级
                    // （shader y = (vel-1)/126，vel=2 → 1 单位高度）。
                    if note.velocity <= 1 {
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

    // 最普通排列：按 (tick ASC, velocity DESC, track ASC) 排序后直接绘制。
    // rayon 并行收集顺序不确定，此排序保证逐帧确定性；同 tick 响音先画
    // （被覆盖），弱音后画（顶层可见）。
    bars.sort_unstable_by(|a, b| {
        a.tick
            .cmp(&b.tick)
            .then(b.velocity().cmp(&a.velocity()))
            .then(a.track().cmp(&b.track()))
    });

    out.extend(bars);
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{Note, NoteBucket, NoteSource};

    /// 测试音符全部放在 key 60（velocity 面板不分 key）。
    struct MockSource {
        notes: NoteBucket, // 按 start_tick 升序
    }

    impl NoteSource for MockSource {
        fn key_notes(&self, key: u8) -> &NoteBucket {
            if key == 60 {
                &self.notes
            } else {
                static EMPTY: std::sync::LazyLock<NoteBucket> =
                    std::sync::LazyLock::new(NoteBucket::default);
                &EMPTY
            }
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
        let mut notes = notes;
        notes.sort_by_key(|n| n.start_tick);
        let src = MockSource {
            notes: NoteBucket::from_sorted(notes),
        };
        let view = AutomationPanelView::default();
        let tv = vec![true; 4];
        let mut out = Vec::new();
        build_velocity_bars(&mut out, 800.0, &src, &view, &tv, None);
        out.into_iter()
            .map(|b| (b.tick, b.length, b.velocity(), b.track()))
            .collect()
    }

    #[test]
    fn keep_partially_covered_long_bar() {
        // 同 tick 部分重叠：不做去重，两条都画。
        let out = build(vec![make_bar(100, 50, 80, 0), make_bar(100, 10, 80, 1)]);
        assert_eq!(out.len(), 2, "重叠都要画: {out:?}");
    }

    #[test]
    fn keep_different_velocity_bars() {
        // vel 不同（100 与 50）：两条都保留。
        let out = build(vec![make_bar(100, 10, 100, 0), make_bar(100, 20, 50, 1)]);
        assert_eq!(out.len(), 2, "不同 vel 全画: {out:?}");
    }

    #[test]
    fn keep_non_overlapping_bars() {
        let out = build(vec![make_bar(100, 10, 80, 0), make_bar(200, 10, 80, 1)]);
        assert_eq!(out.len(), 2);
    }

    /// 最普通排列：按 tick 升序；同 tick 响音先画（底层），弱音后画（顶层）。
    #[test]
    fn order_by_tick_then_velocity_desc() {
        let out = build(vec![
            make_bar(150, 10, 100, 1), // tick 靠后
            make_bar(100, 10, 20, 0),  // tick 靠前，弱音
            make_bar(100, 10, 90, 0),  // 同 tick，响音
        ]);
        // 输出顺序 = 绘制顺序：先 tick 靠前，再同 tick 响音→弱音。
        assert_eq!(out[0].0, 100);
        assert_eq!(out[1].0, 100);
        assert_eq!(out[2].0, 150);
        assert_eq!(out[0].2, 90, "同 tick 响音先画: {out:?}");
        assert_eq!(out[1].2, 20, "同 tick 弱音后画: {out:?}");
    }

    /// velocity=1 的音符不显示（126 级映射的配套）。
    #[test]
    fn skip_velocity_one_bars() {
        let out = build(vec![make_bar(100, 10, 1, 0), make_bar(100, 10, 100, 1)]);
        assert_eq!(out.len(), 1, "vel=1 应被过滤: {out:?}");
        assert_eq!(out[0].2, 100);
    }

    /// 真实 MIDI 力度条构建成本：视口内音符数 vs 输出 bar 数。
    /// 运行：cargo test -p yinhe-wgpu --release -- --ignored --nocapture dedup_real_midi
    #[test]
    #[ignore]
    fn velocity_real_midi_stats() {
        let path = std::env::var("YIN_BENCH_MIDI")
            .unwrap_or_else(|_| "/Users/jieneng/Music/MIDIs/start.mid".to_string());
        let model = yinhe_midi::parse_path(&path).expect("parse 失败");
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
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            ..Default::default()
        };
        let tv = vec![true; model.tracks.len()];
        let (ts, te) = view.base.visible_tick_range(w);
        let before: u64 = (0..128u8)
            .map(|k| {
                model
                    .key_notes_in_range(k, ts as u32, te as u32)
                    .filter(|n| n.velocity > 1) // 与构建的 vel=1 过滤一致
                    .count() as u64
            })
            .sum();
        let t0 = std::time::Instant::now();
        let mut out = Vec::new();
        build_velocity_bars(&mut out, w, &model, &view, &tv, None);
        let ms = t0.elapsed().as_secs_f64() * 1e3;
        println!(
            "{}: 可见 bar={before} 输出={} 保留率={:.1}% 构建+排序耗时={ms:.0}ms",
            path.rsplit('/').next().unwrap_or(path.as_str()),
            out.len(),
            out.len() as f64 * 100.0 / before.max(1) as f64,
        );

        // 真实滚动视口（350 小节附近，ppu=0.026，宽 1376px）：
        // 滚动时 bars_key 失配，每帧重建。测每帧构建+排序成本。
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
                follow_target: None,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
            },
            ..Default::default()
        };
        let mut out2 = Vec::new();
        // 暖机 1 次后取最优（3 次）。
        build_velocity_bars(&mut out2, w2, &model, &view2, &tv, None);
        let mut frame_ms = f64::MAX;
        for _ in 0..3 {
            out2.clear();
            let t = std::time::Instant::now();
            build_velocity_bars(&mut out2, w2, &model, &view2, &tv, None);
            frame_ms = frame_ms.min(t.elapsed().as_secs_f64() * 1e3);
        }
        println!(
            "真实视口(87%): 视口内 bar={} 输出={} 每帧构建+排序={frame_ms:.1}ms ≈ {:.0} FPS",
            before as f64 * (w2 / ppu2 / 3_092_040.0f32) as f64,
            out2.len(),
            1000.0 / frame_ms,
        );
    }
}
