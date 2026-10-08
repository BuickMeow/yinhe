use eframe::egui;
use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{
    MarkerEvent, Orientation, TimeSigEvent, build_time_sig_segments, compute_measure_divisor,
    measure_ticks,
};

// ── Constants ──

use crate::theme;
const MIN_LABEL_SPACING: f32 = 38.0;
const SUB_BEAT_DIV: u32 = 4;
/// 标签（marker）pill 高度（横向）；纵向为 pill 宽度。
const MARKER_PILL_THICKNESS: f32 = 16.0;

/// 标尺上的标签（marker）编辑动作。由调用方应用到文档（以便接入 undo）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MarkerEdit {
    /// 重命名：`tick` 处标签文本改为 `text`。
    Rename { tick: u32, text: String },
    /// 删除 `tick` 处标签。
    Delete { tick: u32 },
    /// 移动标签：从 `from` 到 `to`。
    Move { from: u32, to: u32 },
}

/// 标尺交互结果。
pub(crate) struct RulerOutcome {
    /// 本帧标尺被点击/拖动（用于清除选框）。
    pub jumped: bool,
    /// 标签编辑动作（若本帧触发）。
    pub marker_edit: Option<MarkerEdit>,
}

/// 一个已绘制的标签：命中测试用。
struct MarkerLabel {
    tick: u32,
    rect: egui::Rect,
}

// ── TimeRulerView trait ──

/// View information needed by the time ruler.
/// Both `PianoRollView` and `ArrangementView` implement this.
pub(crate) trait TimeRulerView {
    fn tick_to_x(&self, tick: f64) -> f32;
    fn x_to_tick(&self, x: f32) -> f64;
    fn pixels_per_tick(&self) -> f32;
    /// Minimum x where content (and ruler labels) should appear.
    fn content_left(&self) -> f32;
    /// 水平缩放（围绕指定 x，x 已转换为 view 局部坐标）。
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32);
    /// 标记 view 为 dirty，触发重绘。
    fn mark_dirty(&mut self);

    // ── 主轴（时间轴）语义访问器 ──
    // 默认实现 = 横向（AR 不覆写，行为完全不变）；PianoRollView 覆写以支持纵向瀑布流。
    // 主轴像素均相对 ruler 起点：横向 = X 偏移、纵向 = Y 偏移。

    /// 时间轴方向。
    fn orientation(&self) -> Orientation {
        Orientation::Horizontal
    }

    /// 主轴像素 → tick。
    fn main_px_to_tick(&self, px: f32) -> f64 {
        self.x_to_tick(px + self.content_left())
    }

    /// tick → 主轴像素（相对内容区左缘 / 顶部）。
    fn tick_to_main_px(&self, tick: f64) -> f32 {
        self.tick_to_x(tick) - self.content_left()
    }

    /// 沿主轴（时间轴）缩放，main_px 相对 ruler 起点；view_size = 主轴视口长度。
    fn zoom_main_around(&mut self, main_px: f32, factor: f32, _view_size: f32) {
        self.zoom_around_x(main_px + self.content_left(), factor);
    }

    /// 主轴滚动位置的可变引用（时间轴：横向=scroll_x，纵向=scroll_y）。
    fn scroll_main_mut(&mut self) -> &mut f32;
}

impl TimeRulerView for yinhe_types::PianoRollView {
    fn tick_to_x(&self, tick: f64) -> f32 {
        self.tick_to_x(tick)
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn pixels_per_tick(&self) -> f32 {
        self.base.pixels_per_tick
    }
    fn content_left(&self) -> f32 {
        self.base.left_panel_width
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn mark_dirty(&mut self) {
        self.base.dirty = true;
    }

    // 覆写主轴语义：委托给 PianoRollView 固有访问器（横向 = X / 纵向 = Y）。
    fn orientation(&self) -> Orientation {
        self.orientation
    }

    fn main_px_to_tick(&self, px: f32) -> f64 {
        self.main_px_to_tick(px)
    }

    fn tick_to_main_px(&self, tick: f64) -> f32 {
        self.tick_to_main_px(tick)
    }

    fn zoom_main_around(&mut self, main_px: f32, factor: f32, view_size: f32) {
        if self.is_vertical() {
            // 纵向：时间沿 Y，沿 Y 缩放（zoom_around_y 纵向 = 主轴时间缩放）。
            self.zoom_around_y(main_px, factor, view_size);
        } else {
            // 横向：时间沿 X（与默认实现一致，main_px 换算回全局 x）。
            self.zoom_around_x(main_px + self.content_left(), factor);
        }
    }

    fn scroll_main_mut(&mut self) -> &mut f32 {
        if self.is_vertical() {
            &mut self.base.scroll_y
        } else {
            &mut self.base.scroll_x
        }
    }
}

impl TimeRulerView for yinhe_types::ArrangementView {
    fn tick_to_x(&self, tick: f64) -> f32 {
        self.tick_to_x(tick)
    }
    fn x_to_tick(&self, x: f32) -> f64 {
        self.x_to_tick(x)
    }
    fn pixels_per_tick(&self) -> f32 {
        self.base.pixels_per_tick
    }
    fn content_left(&self) -> f32 {
        self.base.left_panel_width
    }
    fn zoom_around_x(&mut self, pointer_x: f32, factor: f32) {
        self.zoom_around_x(pointer_x, factor);
    }
    fn mark_dirty(&mut self) {
        self.base.dirty = true;
    }

    fn scroll_main_mut(&mut self) -> &mut f32 {
        &mut self.base.scroll_x
    }
}

// ── Public API ──

/// Paint a horizontal time ruler into the given rect.
///
/// Labels are aligned with the measure/beat/sub-beat grid lines rendered by wgpu.
/// Density adapts to `pixels_per_tick`:
/// - sparse → measure numbers only
/// - medium → `bar.beat`
/// - dense  → `bar.beat.sub_beat`
/// - very dense → `bar.beat.tick` (e.g. `1.1.234`)
///
/// Paint the ruler background and bottom divider.
fn paint_background(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 0.0, theme::track_bg());
}

/// Paint an interactive time ruler that also jumps the cursor when clicked or dragged.
///
/// `snap` receives the raw tick under the pointer and should return the snapped tick.
/// `id_salt` must be unique for each ruler in the same UI scope (e.g. "piano_ruler"
/// vs "arrange_ruler").
///
/// Returns [`RulerOutcome`]：是否点击/拖动（调用方据此清除选框），
/// 以及本帧触发的标签编辑动作。
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub(crate) fn interactive_ruler(
    ui: &mut egui::Ui,
    ruler_rect: egui::Rect,
    view: &mut impl TimeRulerView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    markers: &[MarkerEvent],
    snap: impl Fn(f64) -> f64,
    id_salt: &str,
    cursor_tick: &mut Option<f64>,
) -> RulerOutcome {
    let painter = ui.painter_at(ruler_rect);
    paint_background(&painter, ruler_rect);

    // 主轴方向与两带中心：上行标签、下行小节号（纵向：外标签、内小节号）。
    let orientation = view.orientation();
    let (labels_center, numbers_center) = cross_bands(ruler_rect, orientation);
    paint_labels(
        &painter,
        ruler_rect,
        view,
        tpb,
        default_num,
        default_den,
        time_sig_events,
        numbers_center,
    );

    // 标签（marker）绘制 + 命中矩形。
    let drag_preview = marker_drag_preview(ui, id_salt);
    let labels = paint_marker_labels(
        &painter,
        ruler_rect,
        view,
        markers,
        tpb,
        default_num,
        default_den,
        time_sig_events,
        labels_center,
        drag_preview,
    );

    // 相对主轴原点的主轴像素：横向 = 距左缘 X；纵向瀑布流 = 距**底缘** Y
    //（时间轴反转：tick 0 在底部、越大越靠上）。
    let main_px = |pos: egui::Pos2| -> f32 {
        match orientation {
            Orientation::Horizontal => pos.x - ruler_rect.min.x,
            Orientation::Vertical => ruler_rect.max.y - pos.y,
        }
    };
    // 主轴视口长度：横向为宽、纵向为高（供 zoom_main_around 锚定计算）。
    let view_size = match orientation {
        Orientation::Horizontal => ruler_rect.width(),
        Orientation::Vertical => ruler_rect.height(),
    };

    let ruler_resp = ui.interact(
        ruler_rect,
        ui.id().with(id_salt),
        egui::Sense::click_and_drag(),
    );

    // ── 标签（marker）交互：点击跳转、双击改名、右键删除、拖动移动 ──
    // 标签在 ruler 之后注册 → 命中优先级更高；用 pointer_over_label 避免
    // 与 ruler 自身的点击/缩放重复触发。
    let mut marker_edit: Option<MarkerEdit> = None;
    let mut jumped = false;
    for label in &labels {
        let resp = ui.interact(
            label.rect,
            ui.id().with((id_salt, "marker", label.tick)),
            egui::Sense::click_and_drag(),
        );
        let tick = label.tick;
        resp.context_menu(|ui| {
            if crate::widgets::flat::flat_button(ui, rust_i18n::t!("menu.delete_marker")).clicked()
            {
                marker_edit = Some(MarkerEdit::Delete { tick });
                ui.close();
            }
        });
        if resp.double_clicked() {
            start_marker_rename(ui, id_salt, tick, markers);
        } else if resp.dragged() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let new_tick = snap(view.main_px_to_tick(main_px(pos))).max(0.0) as u32;
                set_marker_drag_preview(ui, id_salt, tick, new_tick);
            }
        } else if resp.drag_stopped() {
            if let Some((from, to)) = take_marker_drag_preview(ui, id_salt)
                && from == tick
                && from != to
            {
                marker_edit = Some(MarkerEdit::Move { from, to });
            }
        } else if resp.clicked() {
            // 点击标签：光标跳到该标签 tick + 滚动到可见。
            *cursor_tick = Some(tick as f64);
            scroll_tick_into_view(view, tick as f64, view_size);
            view.mark_dirty();
            ui.ctx().request_repaint();
            jumped = true;
        }
    }
    if let Some(edit) = marker_rename_ui(ui, id_salt, &labels, markers) {
        marker_edit = Some(edit);
    }
    let pointer_over_label = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| labels.iter().any(|l| l.rect.contains(p)));

    // ── 按下标尺后沿副轴拖动 → 时间缩放（防误触：仅起点在标尺内才生效）──
    // 参考 scrollbar 的 press_origin 守卫，避免别处按下拖到标尺上误缩放。
    let press_on_ruler = ui
        .input(|i| i.pointer.press_origin())
        .is_some_and(|p| ruler_rect.contains(p));
    let mut is_zoom_drag = false;
    if press_on_ruler && !pointer_over_label && ruler_resp.dragged() {
        let d = ruler_resp.drag_delta();
        let cross = match orientation {
            Orientation::Horizontal => d.y,
            Orientation::Vertical => d.x,
        };
        let main = match orientation {
            Orientation::Horizontal => d.x,
            Orientation::Vertical => d.y,
        };
        // 副轴位移需显著且大于主轴，才视为缩放意图，避免与光标跳转冲突
        if cross.abs() > 1.0 && cross.abs() > main.abs() {
            let factor = 1.0 + cross * 0.005; // 倒转：上拖缩小、下拖放大（与之前相反）
            // 锚定在当前指针的主轴位置，保持指针下 tick 不动
            if let Some(pos) = ruler_resp
                .interact_pointer_pos()
                .or_else(|| ui.input(|i| i.pointer.hover_pos()))
            {
                let pointer_main = main_px(pos);
                if factor.is_finite() && factor > 0.0 && factor != 1.0 {
                    view.zoom_main_around(pointer_main, factor, view_size);
                    view.mark_dirty();
                    ui.ctx().request_repaint();
                    is_zoom_drag = true;
                }
            }
        }
    }

    if !is_zoom_drag
        && !pointer_over_label
        && (ruler_resp.clicked() || ruler_resp.dragged())
        && let Some(pos) = ruler_resp.interact_pointer_pos()
    {
        let tick = view.main_px_to_tick(main_px(pos));
        *cursor_tick = Some(snap(tick).max(0.0));
        ui.ctx().request_repaint();
        jumped = true;
    }

    // ── 拖出窗口边缘自动滚动（主轴）──
    // 仅当起点在标尺内才生效，避免别处按下拖入误触发；复用共享 delta（MARGIN/BASE_SPEED）。
    if press_on_ruler
        && !pointer_over_label
        && ruler_resp.dragged()
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
    {
        let delta = crate::widgets::auto_scroll::auto_scroll_main(ui, ruler_rect, pos, orientation);
        if delta != 0.0 {
            let scroll = view.scroll_main_mut();
            *scroll += delta;
            // 左边界 clamp：拖出第一小节最左时不得产生负滚动（PR/AR 网格已 clamp，但
            // AM 面板在同一帧 ruler 之后绘制，会透传负值导致网格/GPU 一起右移）。
            *scroll = scroll.max(0.0);
            view.mark_dirty();
            ui.ctx().request_repaint();
        }
    }

    // ── 滚轮 / 触摸板上下滑动 → 沿主轴缩放 ──
    // 时间标尺专属：纯滚轮即可触发时间缩放（无需 Cmd 修饰键），
    // 与内容区的 Cmd+滚轮 缩放语义分离，避免冲突。
    // pinch（zoom_delta）也联动时间缩放。
    let pointer_in_ruler = crate::view_interaction::pointer_hits(ui, ruler_rect);
    if pointer_in_ruler {
        let hover = ui.input(|i| i.pointer.hover_pos().unwrap_or_default());
        let pointer_main = main_px(hover);

        // pinch → 时间缩放
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            view.zoom_main_around(pointer_main, zoom_delta, view_size);
            view.mark_dirty();
            ui.ctx().request_repaint();
        }

        // 滚轮 / 触摸板上下滑动 → 时间缩放（无需 Cmd）
        // 方向：上滚 = 放大，下滚 = 缩小（与内容区一致）
        let scroll = ui.input(|i| i.smooth_scroll_delta);
        if scroll.y.abs() > 0.5 {
            let factor = if scroll.y > 0.0 { 1.0 / 1.1 } else { 1.1 };
            view.zoom_main_around(pointer_main, factor, view_size);
            view.mark_dirty();
            ui.ctx().request_repaint();
        }
    }

    RulerOutcome {
        jumped,
        marker_edit,
    }
}

// ── 标签（marker）辅助 ──

/// 两带中心：返回 (标签带中心, 小节号带中心)。横向 = 上标签、下小节号；
/// 纵向 = 外标签、内小节号。
fn cross_bands(rect: egui::Rect, orientation: Orientation) -> (f32, f32) {
    match orientation {
        Orientation::Horizontal => (
            rect.min.y + rect.height() * 0.28,
            rect.min.y + rect.height() * 0.72,
        ),
        Orientation::Vertical => (
            rect.min.x + rect.width() * 0.28,
            rect.min.x + rect.width() * 0.72,
        ),
    }
}

fn marker_drag_preview(ui: &egui::Ui, id_salt: &str) -> Option<(u32, u32)> {
    ui.memory(|m| {
        m.data
            .get_temp(egui::Id::new((id_salt, "marker_drag_preview")))
    })
}

fn set_marker_drag_preview(ui: &egui::Ui, id_salt: &str, from: u32, to: u32) {
    ui.memory_mut(|m| {
        m.data
            .insert_temp(egui::Id::new((id_salt, "marker_drag_preview")), (from, to))
    });
}

fn take_marker_drag_preview(ui: &egui::Ui, id_salt: &str) -> Option<(u32, u32)> {
    let id = egui::Id::new((id_salt, "marker_drag_preview"));
    let v = ui.memory(|m| m.data.get_temp(id));
    ui.memory_mut(|m| {
        m.data.remove::<(u32, u32)>(id);
    });
    v
}

fn start_marker_rename(ui: &egui::Ui, id_salt: &str, tick: u32, markers: &[MarkerEvent]) {
    let text = markers
        .iter()
        .find(|m| m.tick == tick)
        .map(|m| m.text.clone())
        .unwrap_or_default();
    ui.memory_mut(|m| {
        m.data
            .insert_temp(egui::Id::new((id_salt, "marker_rename_tick")), tick);
        m.data
            .insert_temp(egui::Id::new((id_salt, "marker_rename_buf")), text);
        m.data
            .insert_temp(egui::Id::new((id_salt, "marker_rename_focus")), true);
    });
}

fn clear_marker_rename(ui: &egui::Ui, id_salt: &str) {
    ui.memory_mut(|m| {
        m.data
            .remove::<u32>(egui::Id::new((id_salt, "marker_rename_tick")));
        m.data
            .remove::<String>(egui::Id::new((id_salt, "marker_rename_buf")));
        m.data
            .remove::<bool>(egui::Id::new((id_salt, "marker_rename_focus")));
    });
}

/// 正在重命名的标签内联文本框；返回提交的改名动作。
fn marker_rename_ui(
    ui: &mut egui::Ui,
    id_salt: &str,
    labels: &[MarkerLabel],
    markers: &[MarkerEvent],
) -> Option<MarkerEdit> {
    let tick_id = egui::Id::new((id_salt, "marker_rename_tick"));
    let tick: u32 = ui.memory(|m| m.data.get_temp(tick_id))?;
    let Some(label) = labels.iter().find(|l| l.tick == tick) else {
        // 目标不在可视范围：取消。
        clear_marker_rename(ui, id_salt);
        return None;
    };
    let buf_id = egui::Id::new((id_salt, "marker_rename_buf"));
    let mut buf: String = ui.memory(|m| m.data.get_temp(buf_id).unwrap_or_default());
    let edit_id = egui::Id::new((id_salt, "marker_rename_edit"));
    let rect = label
        .rect
        .expand2(egui::vec2(2.0, 0.0))
        .intersect(ui.clip_rect());
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return None;
    }
    let resp = ui.put(
        rect,
        egui::TextEdit::singleline(&mut buf)
            .id(edit_id)
            .font(egui::FontId::new(
                crate::theme::SMALL_LABEL_FONT,
                egui::FontFamily::Proportional,
            ))
            .desired_width(rect.width()),
    );
    let focus_id = egui::Id::new((id_salt, "marker_rename_focus"));
    if ui.memory(|m| m.data.get_temp::<bool>(focus_id).unwrap_or(false)) {
        resp.request_focus();
        ui.memory_mut(|m| m.data.insert_temp(focus_id, false));
    }
    ui.memory_mut(|m| m.data.insert_temp(buf_id, buf.clone()));
    if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        clear_marker_rename(ui, id_salt);
        return None;
    }
    if resp.lost_focus() {
        clear_marker_rename(ui, id_salt);
        let old = markers
            .iter()
            .find(|m| m.tick == tick)
            .map(|m| m.text.clone())
            .unwrap_or_default();
        if buf != old {
            return Some(MarkerEdit::Rename { tick, text: buf });
        }
    }
    None
}

/// 点击标签跳转后，若标签起点已滚出主轴视口，则把它滚回可见。
fn scroll_tick_into_view(view: &mut impl TimeRulerView, tick: f64, view_size: f32) {
    let main = view.tick_to_main_px(tick);
    let desired = if main < 0.0 {
        4.0
    } else if main > view_size {
        view_size - 4.0
    } else {
        return;
    };
    let scroll = view.scroll_main_mut();
    *scroll = (*scroll + main - desired).max(0.0);
}

/// 绘制标签（marker）pill，返回命中矩形。
#[allow(clippy::too_many_arguments)]
fn paint_marker_labels(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &impl TimeRulerView,
    markers: &[MarkerEvent],
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    labels_center: f32,
    drag_preview: Option<(u32, u32)>,
) -> Vec<MarkerLabel> {
    let orientation = view.orientation();
    let ppu = view.pixels_per_tick();
    if ppu <= 0.001 {
        return Vec::new();
    }
    let main_size = match orientation {
        Orientation::Horizontal => rect.width(),
        Orientation::Vertical => rect.height(),
    };
    let font_id = egui::FontId::new(
        crate::theme::SMALL_LABEL_FONT,
        egui::FontFamily::Proportional,
    );
    let text_color = crate::theme::contrast_fg();
    // 不用强调色：亮色主题用更白的底、暗色主题用更黑的底（与 contrast_fg 文字色相反，保证可读）。
    let pill_color = if crate::theme::dark_mode() {
        egui::Color32::from_gray(24)
    } else {
        egui::Color32::from_gray(250)
    };

    let mut out: Vec<MarkerLabel> = Vec::new();
    for m in markers {
        let tick = drag_preview
            .and_then(|(from, to)| (m.tick == from).then_some(to))
            .unwrap_or(m.tick);
        let main_px = view.tick_to_main_px(tick as f64);
        if main_px < 0.0 || main_px > main_size {
            continue;
        }
        let text = if m.text.is_empty() {
            format_tick_bar_beat_with_time_sig(
                tick as f64,
                tpb,
                time_sig_events,
                default_num,
                default_den,
            )
        } else {
            m.text.clone()
        };
        let galley = painter.layout_no_wrap(text.clone(), font_id.clone(), text_color);
        let tw = galley.size().x;
        let half = MARKER_PILL_THICKNESS * 0.5;

        // pill 沿主轴从标签 tick 起，长度 = 文本宽 + 内边距。
        // 重叠时不跳过：全部绘制，后绘制的（tick 更大）盖在上层，两个标签都可见。
        let rect_start = main_px;

        let label_rect = match orientation {
            Orientation::Horizontal => egui::Rect::from_min_size(
                egui::pos2(rect.min.x + rect_start, labels_center - half),
                egui::vec2(tw + 8.0, MARKER_PILL_THICKNESS),
            ),
            Orientation::Vertical => egui::Rect::from_min_size(
                egui::pos2(labels_center - half, rect.min.y + rect_start),
                egui::vec2(MARKER_PILL_THICKNESS, tw + 8.0),
            ),
        };
        painter.rect_filled(label_rect, 2.0, pill_color);
        match orientation {
            Orientation::Horizontal => {
                painter.text(
                    egui::pos2(label_rect.min.x + 4.0, label_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    text,
                    font_id.clone(),
                    text_color,
                );
            }
            Orientation::Vertical => {
                let anchor = egui::Align2::CENTER_CENTER
                    .anchor_size(label_rect.center(), galley.size())
                    .min;
                painter.add(
                    egui::epaint::TextShape::new(anchor, galley, text_color).with_angle_and_anchor(
                        std::f32::consts::FRAC_PI_2,
                        egui::Align2::CENTER_CENTER,
                    ),
                );
            }
        }
        out.push(MarkerLabel {
            tick: m.tick,
            rect: label_rect,
        });
    }
    out
}

// ── Label painting ──

#[allow(clippy::too_many_arguments)]
fn paint_labels(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &impl TimeRulerView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    numbers_center: f32,
) {
    let orientation = view.orientation();
    let ppu = view.pixels_per_tick();
    if ppu <= 0.001 {
        return;
    }

    // 主轴参数按方向取：横向 = X（rect 宽）、纵向 = Y（rect 高）。
    // 标签位置统一换算为「相对 ruler 起点」的主轴像素 main_px：
    // - 横向：屏幕 x = rect.min.x + main_px（与原 offset_x + tick_to_x 严格等价：
    //   offset_x + tick_to_x = rect.min.x - content_left + tick_to_x = rect.min.x + tick_to_main_px）
    // - 纵向：屏幕 y = rect.min.y + main_px（main_px = tick*ppu - scroll_y）。
    let main_size = match orientation {
        Orientation::Horizontal => rect.width(),
        Orientation::Vertical => rect.height(),
    };
    // 纵向时间轴反转后 main_px_to_tick(0) 是最大 tick，故取 min/max 排序。
    let ta = view.main_px_to_tick(0.0);
    let tb = view.main_px_to_tick(main_size);
    let tick_start = ta.min(tb).max(0.0);
    let tick_end = ta.max(tb);
    // 小节号文字中线：落在「小节号带」（下行 / 内列），标签（marker）占另一带。
    let text_cross_center = numbers_center;

    let ticks_per_sub = (tpb / SUB_BEAT_DIV).max(1);

    let segments = build_time_sig_segments(time_sig_events, default_num, default_den);

    let bar_offsets = cumulative_bar_offsets(tpb, &segments);

    let pixels_per_beat = tpb as f32 * ppu;
    let pixels_per_sub = ticks_per_sub as f32 * ppu;
    let pixels_per_tick = ppu;

    let show_beat = pixels_per_beat >= MIN_LABEL_SPACING;
    let show_sub = pixels_per_sub >= MIN_LABEL_SPACING;
    let show_tick = pixels_per_tick >= MIN_LABEL_SPACING;

    let tick_step = if show_tick {
        (MIN_LABEL_SPACING / ppu).ceil() as u32
    } else {
        0
    };

    let font_id = egui::FontId::new(
        crate::theme::SMALL_LABEL_FONT,
        egui::FontFamily::Proportional,
    );

    for i in 0..segments.len() {
        let (seg_start, num, den) = segments[i];
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |&(t, _, _)| t);
        let seg_start_f = seg_start as f64;
        if seg_start_f > tick_end {
            break;
        }

        let ticks_per_measure = measure_ticks(tpb, num, den);
        let ticks_per_beat = ticks_per_measure / num as u32;
        let bar_offset = bar_offsets[i];

        // 多小节合并：缩很小时 measure label 太密，按 2/4/8… 小节合并，
        // 只在合并后的边界显示小节号（如 4 小节一条线时显示 1, 5, 9…）。
        let pixels_per_measure = ticks_per_measure as f32 * ppu;
        let measure_divisor = compute_measure_divisor(pixels_per_measure, MIN_LABEL_SPACING);
        let merged_measure_ticks = ticks_per_measure.saturating_mul(measure_divisor);

        // 主循环步长：显示 beat/sub 时仍用 ticks_per_sub 遍历，
        // 否则用合并步长以减少遍历量。
        let main_step = if show_beat || show_sub {
            ticks_per_sub
        } else {
            merged_measure_ticks.max(1)
        };

        // ── Main label loop ──
        // 对齐到段内网格：变拍子段的 seg_start 通常不在全局步长网格上，
        // 必须从 seg_start 起按 local 对齐，否则标签全落在错误的网格偏移。
        let first_tick_f = seg_start_f.max(tick_start);
        let step_f = main_step as f64;
        let first = seg_start.saturating_add(
            (((first_tick_f - seg_start_f) / step_f).floor() as u32).saturating_mul(main_step),
        );

        let mut tick = first;
        while (tick as f64) <= tick_end && tick < seg_end {
            let local = tick - seg_start;
            let main_px = view.tick_to_main_px(tick as f64);

            if main_px >= 0.0 && main_px <= main_size {
                let is_measure = local % merged_measure_ticks == 0;
                let is_beat = if !is_measure {
                    (local % ticks_per_measure).is_multiple_of(ticks_per_beat)
                } else {
                    false
                };

                let (label, color) = if is_measure {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    (format!("{}", bar), theme::measure_label())
                } else if is_beat && show_beat {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                    (format!("{}.{}", bar, beat), theme::text_label())
                } else if show_sub {
                    let bar = bar_offset + (local / ticks_per_measure) + 1;
                    let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                    if show_tick {
                        let tick_in_beat = (tick as f64 % tpb as f64) as u32;
                        (
                            format!("{}.{}.{:03}", bar, beat, tick_in_beat),
                            theme::tick_label(),
                        )
                    } else {
                        let sub = (local % ticks_per_beat) / ticks_per_sub;
                        (format!("{}.{}.{}", bar, beat, sub), theme::text_disabled())
                    }
                } else {
                    tick += main_step;
                    continue;
                };

                let label_pos = match orientation {
                    Orientation::Horizontal => egui::pos2(rect.min.x + main_px, text_cross_center),
                    Orientation::Vertical => egui::pos2(text_cross_center, rect.min.y + main_px),
                };
                draw_label(painter, &font_id, label_pos, &label, color, orientation);
            }

            tick += main_step;
        }

        // ── Fine-tick loop: label individual ticks between sub-beat lines ──
        if tick_step > 0 && tick_step < ticks_per_sub {
            let first_tick_u = seg_start.max(tick_start as u32);
            let first_aligned = seg_start.saturating_add(
                (first_tick_u - seg_start)
                    .div_ceil(tick_step)
                    .saturating_mul(tick_step),
            );

            let mut ft = first_aligned;
            while (ft as f64) <= tick_end && ft < seg_end {
                let local = ft - seg_start;

                let is_measure = local % ticks_per_measure == 0;
                let is_beat_line = if !is_measure {
                    (local % ticks_per_measure).is_multiple_of(ticks_per_beat)
                } else {
                    false
                };
                let is_sub_line = local % ticks_per_sub == 0;

                if !is_measure && !is_beat_line && !is_sub_line {
                    let main_px = view.tick_to_main_px(ft as f64);
                    if main_px >= 0.0 && main_px <= main_size {
                        let bar = bar_offset + (local / ticks_per_measure) + 1;
                        let beat = (local % ticks_per_measure) / ticks_per_beat + 1;
                        let tick_in_beat = (ft as f64 % tpb as f64) as u32;
                        let label = format!("{}.{}.{:03}", bar, beat, tick_in_beat);
                        let label_pos = match orientation {
                            Orientation::Horizontal => {
                                egui::pos2(rect.min.x + main_px, text_cross_center)
                            }
                            Orientation::Vertical => {
                                egui::pos2(text_cross_center, rect.min.y + main_px)
                            }
                        };
                        draw_label(
                            painter,
                            &font_id,
                            label_pos,
                            &label,
                            theme::tick_label(),
                            orientation,
                        );
                    }
                }

                ft += tick_step;
            }
        }
    }
}

// ── Label drawing ──

fn draw_label(
    painter: &egui::Painter,
    font_id: &egui::FontId,
    pos: egui::Pos2,
    text: &str,
    color: egui::Color32,
    orientation: Orientation,
) {
    match orientation {
        // 横向：横排标签（保持原逐像素行为：右偏 2px、LEFT_CENTER、无旋转）。
        Orientation::Horizontal => {
            painter.text(
                egui::pos2(pos.x + 2.0, pos.y),
                egui::Align2::LEFT_CENTER,
                text,
                font_id.clone(),
                color,
            );
        }
        // 纵向：竖排标签（顺时针 90°，时间向下递增时字形头朝上），
        // 与 hover.rs 同款的旋转写法。
        Orientation::Vertical => {
            let galley = painter.layout_no_wrap(text.to_owned(), font_id.clone(), color);
            let anchor_pos = egui::Align2::CENTER_CENTER
                .anchor_size(pos, galley.size())
                .min;
            painter.add(
                egui::epaint::TextShape::new(anchor_pos, galley, color).with_angle_and_anchor(
                    std::f32::consts::FRAC_PI_2,
                    egui::Align2::CENTER_CENTER,
                ),
            );
        }
    }
}

// ── Bar offset computation ──

/// Compute cumulative bar counts before each segment starts.
///
/// `offsets[i]` = total number of complete bars in segments 0..i.
/// Segment 0 always starts at offset 0.
fn cumulative_bar_offsets(tpb: u32, segments: &[(u32, u8, u8)]) -> Vec<u32> {
    let mut offsets = Vec::with_capacity(segments.len());
    let mut acc = 0u32;
    for i in 0..segments.len() {
        offsets.push(acc);
        if i + 1 < segments.len() {
            let (start, num, den) = segments[i];
            let end = segments[i + 1].0;
            let tm = measure_ticks(tpb, num, den);
            if tm > 0 && end > start {
                acc += (end - start) / tm;
            }
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::TimelineViewBase;

    struct FakeRuler {
        base: TimelineViewBase,
    }

    fn make_base(ppu: f32) -> TimelineViewBase {
        TimelineViewBase {
            pixels_per_tick: ppu,
            scroll_x: 0.0,
            scroll_y: 0.0,
            left_panel_width: 60.0,
            dirty: false,
            track_panel_row_height: 40.0,
            track_panel_scroll_y: 0.0,
            follow_target: None,
            follow_anim_start: 0.0,
            follow_anim_elapsed: 0.0,
        }
    }

    impl TimeRulerView for FakeRuler {
        fn tick_to_x(&self, tick: f64) -> f32 {
            self.base.tick_to_x(tick)
        }
        fn x_to_tick(&self, x: f32) -> f64 {
            self.base.x_to_tick(x)
        }
        fn pixels_per_tick(&self) -> f32 {
            self.base.pixels_per_tick
        }
        fn content_left(&self) -> f32 {
            self.base.left_panel_width
        }
        fn zoom_around_x(&mut self, _pointer_x: f32, _factor: f32) {}
        fn mark_dirty(&mut self) {}
        fn scroll_main_mut(&mut self) -> &mut f32 {
            &mut self.base.scroll_x
        }
    }

    /// 变拍子段（seg_start 不在主步长网格上）的标签必须正常显示：
    /// 段内对齐保证小节/拍标签不丢失。
    #[test]
    fn test_ruler_labels_align_within_segment() {
        // tpb=480，拍号事件在 tick 1000（非 480 倍数）变 3/4，屏幕显示 tick 8000..14000。
        // 3/4 段每小节 1440、每拍 480；seg_start=1000 不在 480/120 网格上。
        let ruler = FakeRuler {
            base: TimelineViewBase {
                pixels_per_tick: 0.1,
                scroll_x: 800.0,
                follow_anim_start: 0.0,
                follow_anim_elapsed: 0.0,
                ..make_base(0.1)
            },
        };
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(660.0, 30.0));
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default()); // 初始化字体系统（测试环境）
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("ruler_test"),
        ));
        let events = vec![TimeSigEvent {
            tick: 1000,
            numerator: 3,
            denominator: 2,
        }];
        paint_labels(
            &painter,
            rect,
            &ruler,
            480,
            4,
            2,
            &events,
            rect.min.y + rect.height() * 0.72,
        );

        let offset_x = rect.min.x - ruler.content_left();
        let mut label_ticks = Vec::new();
        painter.for_each_shape(|cs| {
            if let egui::Shape::Text(t) = &cs.shape {
                let x = t.pos.x - 2.0; // draw_label 在 x+2 处绘制
                label_ticks.push(ruler.x_to_tick(x - offset_x));
            }
        });
        label_ticks.sort_by(|a, b| a.total_cmp(b));

        // 4/4 段与 3/4 段都应有标签（修复前 3/4 段因全局对齐全部丢失）。
        assert!(label_ticks.len() >= 10, "标签数量过少: {label_ticks:?}");
        let in_34 = label_ticks.iter().filter(|&&t| t >= 10000.0 - 1.0).count();
        assert!(
            in_34 >= 3,
            "3/4 段应有标签，实际 {in_34} 个: {label_ticks:?}"
        );
        // 3/4 段所有标签都必须落在拍网格上（段内 local 是 480 的倍数）。
        let on_beat_grid = |t: f64| {
            let local = t - 1000.0;
            ((local / 480.0).round() * 480.0 - local).abs() < 1.0
        };
        assert!(
            label_ticks
                .iter()
                .all(|&t| t < 10000.0 - 1.0 || on_beat_grid(t)),
            "3/4 段标签应落在拍网格上: {label_ticks:?}"
        );
    }

    #[test]
    fn cumulative_bar_offsets_single_segment() {
        // 4/4 at 480tpb, one segment from tick 0
        let segs = vec![(0u32, 4u8, 2u8)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets, vec![0]);
    }

    #[test]
    fn cumulative_bar_offsets_two_segments() {
        // 4/4 from tick 0, then 3/4 from tick 1920
        let segs = vec![(0, 4, 2), (1920, 3, 2)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets.len(), 2);
        assert_eq!(offsets[0], 0);
        // 1920 ticks / (480*4=1920 ticks/bar) = 1 bar
        assert_eq!(offsets[1], 1);
    }

    #[test]
    fn cumulative_bar_offsets_empty() {
        let segs: Vec<(u32, u8, u8)> = vec![];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert!(offsets.is_empty());
    }

    #[test]
    fn cumulative_bar_offsets_starts_at_zero() {
        let segs = vec![(0, 4, 2), (960, 4, 2), (1920, 3, 2)];
        let offsets = cumulative_bar_offsets(480, &segs);
        assert_eq!(offsets[0], 0);
    }

    /// 标签（marker）绘制：返回命中矩形，位置随 tick 递增；重叠的标签全部绘制
    /// （后绘制者在上层，不再跳过）。
    #[test]
    fn marker_labels_hit_rects_and_overlap() {
        let ruler = FakeRuler {
            base: make_base(0.1),
        };
        let rect = egui::Rect::from_min_max(egui::pos2(60.0, 0.0), egui::pos2(660.0, 36.0));
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput::default());
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("ruler_marker_test"),
        ));
        let labels_center = rect.min.y + rect.height() * 0.28;

        let far = vec![
            MarkerEvent {
                tick: 0,
                text: "A".into(),
            },
            MarkerEvent {
                tick: 1000,
                text: "B".into(),
            },
        ];
        let hits = paint_marker_labels(
            &painter,
            rect,
            &ruler,
            &far,
            480,
            4,
            2,
            &[],
            labels_center,
            None,
        );
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].tick, 0);
        assert_eq!(hits[1].tick, 1000);
        // tick 越大，标签越靠右（主轴 = X）。
        assert!(hits[1].rect.min.x > hits[0].rect.min.x);

        // 两个标签主轴间距过小 → 两者都绘制（后者在上层，不跳过）。
        let near = vec![
            MarkerEvent {
                tick: 0,
                text: "A".into(),
            },
            MarkerEvent {
                tick: 10,
                text: "B".into(),
            },
        ];
        let hits = paint_marker_labels(
            &painter,
            rect,
            &ruler,
            &near,
            480,
            4,
            2,
            &[],
            labels_center,
            None,
        );
        assert_eq!(hits.len(), 2, "重叠标签也应全部绘制");
    }
}
