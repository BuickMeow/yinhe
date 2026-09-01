use eframe::egui;
use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
use yinhe_types::{NoteSource, PianoRollView, TimeSigEvent};

/// Hit-test 边缘的像素阈值（与铅笔工具一致）。
const EDGE_THRESHOLD_PX: f32 = 6.0;

/// 一次拖拽中收集的选中音符信息（move / resize 共用）。
#[derive(Clone)]
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
    if view.is_vertical() {
        // 纵向瀑布流：key→X（key0 在左、增大向右），tick→Y（tick0 在顶、向下增大）。
        // 副轴覆盖 key_lo..=key_hi 各含一个小边界：左 = key_lo 左缘，右 = key_hi 右缘。
        let x0 = view.key_to_cross_px(key_lo);
        let x1 = view.key_to_cross_px(key_hi) + view.key_height;
        egui::Rect::from_min_max(
            egui::pos2(x0.min(x1), sx.min(ex)),
            egui::pos2(x0.max(x1), sx.max(ex)),
        )
    } else {
        // 横向（必须与旧代码逐像素等价）：tick→X、key→Y（key127 在顶）。
        let kh = view.key_height;
        let scroll_y = view.base.scroll_y;
        let sy = (127.0 - key_hi as f32) * kh - scroll_y;
        let ey = (127.0 - key_lo as f32 + 1.0) * kh - scroll_y;
        egui::Rect::from_min_max(
            egui::pos2(sx.min(ex), sy.min(ey)),
            egui::pos2(sx.max(ex), sy.max(ey)),
        )
    }
}

/// 预计算选中音符信息（move 和 resize 共用）。
///
/// 使用 `Selection::contains` 做精确的半开 tick 过滤，并叠加 track_selected
/// 和 track_visible 过滤，与 note layer 的 build_notes 对齐。
/// `track_selected` 传空集合表示不过滤轨道（空 = 全部轨道）。
pub fn collect_selected_notes(
    selected: &yinhe_core::Selection,
    midi: Option<&dyn NoteSource>,
    track_visible: &[bool],
    track_selected: &std::collections::HashSet<u16>,
) -> Vec<CollectedNote> {
    selected
        .rects
        .iter()
        .flat_map(|&(ts, te, kl, kh, _tl, _th)| {
            (kl..=kh).flat_map(move |key| {
                midi.map(|m| {
                    m.key_notes_in_range(key, ts, te)
                        .filter(|n| selected.contains(n.track, n.start_tick, key))
                        .filter(|n| track_selected.is_empty() || track_selected.contains(&n.track))
                        .filter(|n| track_visible.get(n.track as usize).copied().unwrap_or(true))
                        .map(|n| CollectedNote {
                            track: n.track,
                            start_tick: n.start_tick,
                            end_tick: n.end_tick,
                            key,
                            velocity: n.velocity,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
            })
        })
        .collect()
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
pub fn add_pr_selection_rect(
    selected: &mut yinhe_core::Selection,
    t_start: u32,
    t_end: u32,
    key_lo: u8,
    key_hi: u8,
    track_selected: &std::collections::HashSet<u16>,
) {
    if let Some(t) = pr_single_track(track_selected) {
        selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
    } else if track_selected.is_empty() {
        selected.add_rect_track(t_start, t_end, key_lo, key_hi, 0, u16::MAX);
    } else {
        for &t in track_selected {
            selected.add_rect_track(t_start, t_end, key_lo, key_hi, t, t);
        }
    }
}

/// Hit-test 鼠标是否在某个选框的主轴两端边缘 `EDGE_THRESHOLD_PX` 内。
///
/// 返回 `(side, origin_boundary_tick, other_boundary_tick)`：
/// - `origin_boundary_tick`：被拖动边缘的原 tick
/// - `other_boundary_tick`：另一个边缘的原 tick（用于计算最小宽度约束）
///
/// Left = 主轴起始端（t_start 侧），Right = 主轴末端（t_end 侧）。
/// 横向主轴的屏幕方向是 X，纵向是 Y。副轴范围判定横向用屏幕 Y、纵向用屏幕 X。
///
/// 窄选框（主轴方向宽度 <= 2×阈值）跳过边缘检测，避免与 move 冲突。
pub fn hit_test_sel_edge(
    eff_rects: &[(f64, f64, u8, u8)],
    view: &PianoRollView,
    local: egui::Pos2,
) -> Option<(ResizeSide, f64, f64)> {
    for &(t_start, t_end, key_lo, key_hi) in eff_rects {
        let pixel_rect = music_sel_to_pixel_rect(view, t_start, t_end, key_lo, key_hi);
        // 副轴不在选框范围（含阈值）内 → 跳过。横向副轴 = 屏幕 Y；纵向副轴 = 屏幕 X。
        let (cross_local, cross_lo, cross_hi) = if view.is_vertical() {
            (local.x, pixel_rect.min.x, pixel_rect.max.x)
        } else {
            (local.y, pixel_rect.min.y, pixel_rect.max.y)
        };
        if cross_local < cross_lo - EDGE_THRESHOLD_PX || cross_local > cross_hi + EDGE_THRESHOLD_PX
        {
            continue;
        }
        // 窄选框跳过边缘检测，避免与 move 冲突（主轴方向跨度）。
        let main_span = if view.is_vertical() {
            pixel_rect.height()
        } else {
            pixel_rect.width()
        };
        if main_span <= 2.0 * EDGE_THRESHOLD_PX {
            continue;
        }
        // 主轴边缘距离判定：横向用 local.x 与 min.x/max.x；纵向用 local.y 与 min.y/max.y。
        let (main_local, main_lo, main_hi) = if view.is_vertical() {
            (local.y, pixel_rect.min.y, pixel_rect.max.y)
        } else {
            (local.x, pixel_rect.min.x, pixel_rect.max.x)
        };
        // 起点边缘（主轴起始端 = t_start）
        if (main_local - main_lo).abs() <= EDGE_THRESHOLD_PX {
            return Some((ResizeSide::Left, t_start, t_end));
        }
        // 终点边缘（主轴末端 = t_end）
        if (main_local - main_hi).abs() <= EDGE_THRESHOLD_PX {
            return Some((ResizeSide::Right, t_end, t_start));
        }
    }
    None
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
