use eframe::egui;
use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{NoteSource, PianoRollView, TimeSigEvent};

/// 一次拖拽中收集的选中音符信息（move / resize 共用）。
#[derive(Clone, Debug)]
pub struct CollectedNote {
    pub track: u16,
    pub start_tick: u32,
    pub end_tick: u32,
    pub key: u8,
    /// 原力度（拖拽预览用）。
    pub velocity: u8,
}

/// 共享实现已抽至 `widgets::auto_scroll`，此处保留薄封装以兼容既有调用方。
pub use crate::widgets::auto_scroll::{auto_scroll_on_drag, auto_scroll_on_drag_dir};

// ── 方向感知的坐标访问器 ─────────────────────────────────────────────────────
// 交互状态机全部工作在 (tick, key) 领域坐标，只在入口/出口做方向化：
// 主轴 = 时间轴（横向 X / 纵向 Y），副轴 = 音高（横向 Y / 纵向 X）。
// 访问器返回 content-relative 像素；local = pos - content_rect.min。
//
// 注意：横向 tick 的原点在**音乐区左缘**（content 左缘 + keyboard_width，与
// wgpu shader / cull / 旧 `x_to_tick` 一致），而新 `tick_to_main_px` 以 content
// 原点为 0 —— 因此横向必须走旧数学（base.x_to_tick / tick_to_x）才能与渲染
// 逐像素对齐；纵向才用新的 main_px 访问器（tick 0 = 顶部）。

/// 把 content-relative 的 (x, y) 拆成 (主轴 px, 副轴 px)。
/// 横向：主轴 = x（tick），副轴 = y（key）；纵向：主轴 = y（tick），副轴 = x（key）。
#[inline]
pub(crate) fn main_cross_x_y(view: &PianoRollView, (x, y): (f32, f32)) -> (f32, f32) {
    if view.is_vertical() { (y, x) } else { (x, y) }
}

/// 主轴像素 → tick（与 `tick_to_main_px_dir` 互逆）。
/// 纵向用 `main_px_to_tick`；横向保持旧数学 `base.x_to_tick`（tick 0 = 音乐区左缘）。
#[inline]
pub(crate) fn main_px_to_tick_dir(view: &PianoRollView, main_px: f32) -> f64 {
    if view.is_vertical() {
        view.main_px_to_tick(main_px)
    } else {
        view.base.x_to_tick(main_px)
    }
}

/// tick → 主轴像素（与 `main_px_to_tick_dir` 互逆）。
/// 纵向用 `tick_to_main_px`；横向保持旧数学 `tick_to_x`。
#[inline]
pub(crate) fn tick_to_main_px_dir(view: &PianoRollView, tick: f64) -> f32 {
    if view.is_vertical() {
        view.tick_to_main_px(tick)
    } else {
        view.tick_to_x(tick)
    }
}

/// 组装方向感知矩形：主轴两端 `(a0, a1)`（tick 像素）× 副轴单元 `(b0, b1)`（key 像素）。
/// 横向：Rect.x = tick、Rect.y = key；纵向：Rect.x = key、Rect.y = tick。
#[inline]
pub(crate) fn orient_rect(view: &PianoRollView, a0: f32, a1: f32, b0: f32, b1: f32) -> egui::Rect {
    let (x0, y0, x1, y1) = if view.is_vertical() {
        (b0.min(b1), a0.min(a1), b0.max(b1), a0.max(a1))
    } else {
        (a0.min(a1), b0.min(b1), a0.max(a1), b0.max(b1))
    };
    egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1))
}

/// Convert a persisted music selection `(t_start, t_end, key_lo, key_hi)` to
/// a pixel-space `Rect` in the pianoroll view.
///
/// 输出为 content-relative 矩形：横向 (x = tick, y = key)；纵向 (x = key, y = tick)。
pub fn music_sel_to_pixel_rect(
    view: &PianoRollView,
    t_start: f64,
    t_end: f64,
    key_lo: u8,
    key_hi: u8,
) -> egui::Rect {
    let (sx, ex) = (
        tick_to_main_px_dir(view, t_start),
        tick_to_main_px_dir(view, t_end),
    );
    // 副轴统一走 keys_to_cross_span：正确处理 88 键起点、SCALE 折叠，
    // 并包含 key_lo / key_hi 两端整行（横向高音在上时尤其重要）。
    let (cross_a, cross_b) = view.keys_to_cross_span(key_lo, key_hi);
    if view.is_vertical() {
        egui::Rect::from_min_max(
            egui::pos2(cross_a, sx.min(ex)),
            egui::pos2(cross_b, sx.max(ex)),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(sx.min(ex), cross_a),
            egui::pos2(sx.max(ex), cross_b),
        )
    }
}

/// 指针是否命中选区的时间终点边缘（横向 = 右缘，纵向 = 顶端）。
///
/// 三联选择器选中 Gate/Scale/Riff 后，不再画三个方块控制点；右缘就是
/// 统一的拖动入口。`local` 与 `music_sel_to_pixel_rect` 同为 content-local。
pub fn selection_main_edge_hit(
    view: &PianoRollView,
    rect: (f64, f64, u8, u8),
    local: egui::Pos2,
    threshold: f32,
) -> bool {
    let (t0, t1, k0, k1) = rect;
    let pixel = music_sel_to_pixel_rect(view, t0, t1, k0, k1);
    rect_main_edge_hit(pixel, local, view.is_vertical(), threshold)
}

/// 像素矩形的“时间终点边”命中：横向为右缘，纵向为顶缘。
///
/// PR 与 AR 共用；AR 的时间轴恒为横向，直接传 `vertical = false`。
pub fn rect_main_edge_hit(
    rect: egui::Rect,
    local: egui::Pos2,
    vertical: bool,
    threshold: f32,
) -> bool {
    if vertical {
        (local.y - rect.min.y).abs() <= threshold
            && local.x >= rect.min.x - threshold
            && local.x <= rect.max.x + threshold
    } else {
        (local.x - rect.max.x).abs() <= threshold
            && local.y >= rect.min.y - threshold
            && local.y <= rect.max.y + threshold
    }
}

/// 预计算选中音符信息（move 和 resize 共用）。
///
/// 使用 `Selection::accepts_note` 做精确的半开 tick 过滤 + 属性筛选，
/// 并叠加 track_selected 和 track_visible 过滤，与 note layer 的
/// build_notes 对齐。`track_selected` 传空集合表示不过滤轨道（空 = 全部轨道）。
pub fn collect_selected_notes(
    selected: &yinhe_core::Selection,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
    track_locked: &[bool],
) -> Vec<CollectedNote> {
    let Some(midi) = midi else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // 与 batch_ops::for_each_selected 相同的「合并 tick 区间」遍历：重叠选框下
    // 同一音符只收集一次（此前逐 rect 遍历会重复收集，导致 ghost/hidden 与
    // 拖拽预览重复构建）。
    let ranges = selected.merged_tick_ranges_by_key();
    for (key, key_ranges) in ranges.iter().enumerate() {
        let key = key as u8;
        for &(ts, te) in key_ranges {
            for n in midi.key_notes_in_range(key, ts, te) {
                if !selected.accepts_note(n, key) {
                    continue;
                }
                if !track_selected.is_empty() && !track_selected.contains(&n.track) {
                    continue;
                }
                if !track_visible.get(n.track as usize).copied().unwrap_or(true) {
                    continue;
                }
                if track_locked.get(n.track as usize).copied().unwrap_or(false) {
                    continue;
                }
                out.push(CollectedNote {
                    track: n.track,
                    start_tick: n.start_tick,
                    end_tick: n.end_tick,
                    key,
                    velocity: n.velocity,
                });
            }
        }
    }
    out
}

/// PR 工具的轨道作用范围：track_selected（空 = 全部轨道，否则 min..=max）。
pub fn pr_track_range(track_selected: &std::collections::HashSet<u16>) -> (u16, u16) {
    let lo = track_selected.iter().min().copied().unwrap_or(0);
    let hi = track_selected.iter().max().copied().unwrap_or(u16::MAX);
    (lo, hi)
}

/// 选框的轨道范围是否只覆盖单个轨道。
/// track_selected 恰好只含一个轨道时返回它，否则 None。
fn pr_single_track(track_selected: &std::collections::HashSet<u16>) -> Option<u16> {
    if track_selected.is_empty() {
        return None;
    }
    let mut it = track_selected.iter().copied();
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

/// 把选框 `(t_start..t_end, key_lo..=key_hi)` 按 PR 工具作用域添加到选区。
///
/// track_selected 空 → 全部轨道；单轨 → 只加该轨；
/// 多轨选中 → 逐轨添加（避免 [min,max] 连续范围误伤中间未选中的轨道）。
///
/// `midi` 存在时同时把命中音符物化为显式成员：此后选区跟着这批音符走，
/// 移动/复制到落点不会把落点处的其他音符拉进选区（不再是"矩形重新查询"）。
#[allow(clippy::too_many_arguments)]
pub fn add_pr_selection_rect(
    selected: &mut yinhe_core::Selection,
    t_start: u32,
    t_end: u32,
    key_lo: u8,
    key_hi: u8,
    track_selected: &std::collections::HashSet<u16>,
    midi: Option<&dyn NoteSource>,
    track_locked: &[bool],
) {
    // 无锁定信息（长度 0）时保持原有「全轨范围」语义。
    if track_locked.is_empty() {
        if let Some(t) = pr_single_track(track_selected) {
            selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
        } else if track_selected.is_empty() {
            selected.add_rect_track(t_start, t_end, key_lo, key_hi, 0, u16::MAX);
        } else {
            for &t in track_selected {
                selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
            }
        }
    } else {
        let locked = |t: u16| track_locked.get(t as usize).copied().unwrap_or(false);
        if let Some(t) = pr_single_track(track_selected) {
            if !locked(t) {
                selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
            }
        } else if track_selected.is_empty() {
            for t in 0..track_locked.len() as u16 {
                if !locked(t) {
                    selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
                }
            }
        } else {
            for &t in track_selected {
                if !locked(t) {
                    selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
                }
            }
        }
    }
    // 追加的矩形统一物化（包括此前矩形态未物化的旧矩形，如全选）。
    if let Some(source) = midi {
        selected.materialize_pending(source);
    }
}

/// 计算 resize 的 dt（按量化 snap），并约束最小宽度为一个量化间隔。
///
/// - `origin_boundary_tick`：被拖动边缘的原 tick
/// - `other_boundary_tick`：另一个边缘的原 tick
/// - 返回 `(snapped_boundary, dt)`：dt 已 clamp 使得 new_width >= interval
#[allow(clippy::too_many_arguments)]
pub fn compute_resize_dt(
    raw_tick: f64,
    side: ResizeSide,
    origin_boundary_tick: f64,
    other_boundary_tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
) -> (f64, i64) {
    let interval = quantize.tick_interval(ppq) as f64;
    let original_width = (origin_boundary_tick - other_boundary_tick).abs();
    // 最小允许宽度：如果原选框已经 < interval，不允许再压缩（min = original_width）
    let min_width = original_width.min(interval).max(1.0);

    match side {
        ResizeSide::Right => {
            let snapped =
                crate::view_interaction::snap_tick_ceil(raw_tick, quantize, ppq, bar_line_data);
            let mut dt = (snapped - origin_boundary_tick).round() as i64;
            // new_width = original_width + dt >= min_width
            let dt_min = (min_width - original_width).ceil() as i64;
            dt = dt.max(dt_min);
            (origin_boundary_tick + dt as f64, dt)
        }
        ResizeSide::Left => {
            let snapped =
                crate::view_interaction::snap_tick_floor(raw_tick, quantize, ppq, bar_line_data);
            let mut dt = (snapped - origin_boundary_tick).round() as i64;
            // new_width = original_width - dt >= min_width → dt <= original_width - min_width
            let dt_max = (original_width - min_width).floor() as i64;
            dt = dt.min(dt_max);
            (origin_boundary_tick + dt as f64, dt)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_core::Selection;

    /// 回归：重叠选框下同一音符只收集一次（此前逐 rect 遍历会重复收集）。
    #[test]
    fn collect_selected_notes_dedups_overlapping_rects() {
        let midi = yinhe_test_helpers::make_midi(vec![(60, 0, 480, 0, 100)]);
        let mut sel = Selection::default();
        sel.add_rect_track(0, 480, 0, 127, 0, 0);
        sel.add_rect_track(240, 720, 0, 127, 0, 0); // 与第一个重叠

        let notes = collect_selected_notes(&sel, Some(&midi), &[true], &Default::default(), &[]);
        assert_eq!(notes.len(), 1, "重叠选框不得重复收集：{notes:?}");
        assert_eq!(notes[0].start_tick, 0);
    }

    /// 轨道过滤与可见性过滤保持原语义（track_selected 空 = 不过滤）。
    #[test]
    fn collect_selected_notes_filters_track_and_visibility() {
        let midi = yinhe_test_helpers::make_midi(vec![(60, 0, 480, 0, 100), (60, 0, 480, 1, 100)]);
        let mut sel = Selection::default();
        sel.add_rect_track(0, 480, 0, 127, 0, 1);

        // 空 track_selected：不过滤轨道，但 track_visible[0]=false 会过滤掉轨 0
        let notes =
            collect_selected_notes(&sel, Some(&midi), &[false, true], &Default::default(), &[]);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].track, 1);

        // 显式 track_selected 只留轨 1
        let only1: std::collections::HashSet<u16> = [1u16].into_iter().collect();
        let notes = collect_selected_notes(&sel, Some(&midi), &[true, true], &only1, &[]);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].track, 1);
    }

    /// 锁定轨的音符不被收集（不可移动/编辑）。
    #[test]
    fn collect_selected_notes_skips_locked_tracks() {
        let midi = yinhe_test_helpers::make_midi(vec![(60, 0, 480, 0, 100), (60, 0, 480, 1, 100)]);
        let mut sel = Selection::default();
        sel.add_rect_track(0, 480, 0, 127, 0, 1);

        // 轨 0 锁定：只剩轨 1。
        let notes = collect_selected_notes(
            &sel,
            Some(&midi),
            &[true, true],
            &Default::default(),
            &[true, false],
        );
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].track, 1);
    }

    /// 锁定轨不进入 PR 框选（add_pr_selection_rect）。
    #[test]
    fn add_pr_selection_rect_skips_locked_tracks() {
        let midi = yinhe_test_helpers::make_midi(vec![(60, 0, 480, 0, 100), (60, 0, 480, 1, 100)]);
        let mut sel = Selection::default();
        add_pr_selection_rect(
            &mut sel,
            0,
            480,
            0,
            127,
            &Default::default(),
            Some(&midi),
            &[true, false],
        );
        let notes =
            collect_selected_notes(&sel, Some(&midi), &[true, true], &Default::default(), &[]);
        assert_eq!(notes.len(), 1, "锁定轨 0 不应被框选中");
        assert_eq!(notes[0].track, 1);
    }
    #[test]
    fn rect_main_edge_hit_is_shared_by_pr_and_ar() {
        let r = egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(200.0, 150.0));
        // 横向：时间终点在右缘。
        assert!(rect_main_edge_hit(r, egui::pos2(203.0, 100.0), false, 3.0));
        assert!(!rect_main_edge_hit(r, egui::pos2(196.0, 100.0), false, 3.0));
        // 纵向：时间终点在顶缘。
        assert!(rect_main_edge_hit(r, egui::pos2(150.0, 47.0), true, 3.0));
        assert!(!rect_main_edge_hit(r, egui::pos2(150.0, 60.0), true, 3.0));
    }

    /// 回归：41..=60 的选框必须覆盖 41 和 60 两整行。
    ///
    /// 横向高音在上，曾用 key_to_cross_px(key_lo) 当底缘、key_to_cross_px(key_hi)
    /// 当顶缘，结果画成 42..=59（内容选中仍正确，只有框少两行）。
    #[test]
    fn selection_rect_includes_key_range_boundaries() {
        let view = yinhe_types::PianoRollView {
            key_height: 10.0,
            ..Default::default()
        };
        let rect = music_sel_to_pixel_rect(&view, 0.0, 480.0, 41, 60);
        let top = view.key_to_cross_px(60);
        let bottom = view.key_to_cross_px(41) + view.key_height;
        assert!((rect.min.y - top).abs() < 0.01, "上界应包含 key 60");
        assert!((rect.max.y - bottom).abs() < 0.01, "下界应包含 key 41");
        assert!(
            (rect.height() - 20.0 * view.key_height).abs() < 0.01,
            "41..=60 应覆盖 20 行，实际 {}",
            rect.height()
        );
    }

    /// 88 键 + SCALE 折叠下，选框的副轴边界必须与 key 行实际位置一致。
    #[test]
    fn selection_rect_respects_fold_and_88_key_start() {
        let mut layout = yinhe_types::KeyLayout::for_key_count(88);
        layout.fold_mask = 0b1010_1011_0101; // C 大调
        let view = yinhe_types::PianoRollView {
            layout,
            key_height: 10.0,
            ..Default::default()
        };
        let rect = music_sel_to_pixel_rect(&view, 0.0, 100.0, 60, 64);
        let top = view.key_to_cross_px(64);
        let bottom = view.key_to_cross_px(60) + view.layout.row_units(60) * view.key_height;
        assert_eq!(rect.min.y, top.min(bottom));
        assert_eq!(rect.max.y, top.max(bottom));
    }
}
