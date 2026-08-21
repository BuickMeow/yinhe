use eframe::egui;

#[allow(clippy::single_component_path_imports)]
use yinhe_types;

use crate::theme;
use crate::view_interaction;
use crate::widgets::scrollbar;

/// 绘制钢琴卷帘的滚动条（时间轴 + 音高轴）。
///
/// 纵向瀑布流与横向共用同一函数，内部按 `view.is_vertical()` 分支：
///
/// - 纵向：时间滚动条竖在右侧（绑 `scroll_y` / `ppt`），音高滚动条横在底部（绑 `scroll_x` / `key_height`）
/// - 横向：时间横条（绑 `scroll_x` / `ppt`）+ 音高竖条（绑 `scroll_y` / `key_height`）
///
/// 处理 `tick_sb_drag` / `key_sb_drag` 的 zoom 逻辑以及 `pointer_over_popup` 检查。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show_scrollbars(
    ui: &mut egui::Ui,
    view: &mut yinhe_types::PianoRollView,
    rect: egui::Rect,
    content_rect: egui::Rect,
    content_bottom: f32,
    content_y: f32,
    w: u32,
    h: u32,
    total_ticks: f64,
    pr_orientation: yinhe_types::Orientation,
) {
    let _ = h;
    let sb_y = rect.min.y + rect.height() - scrollbar::SCROLLBAR_H;
    let content_right_x = rect.max.x - scrollbar::SCROLLBAR_W;
    let content_h = (content_bottom - content_y).max(0.0);
    let kb_w = view.keyboard_width();

    // 右下角角落：横纵滚动条交叠区（SCROLLBAR_W × SCROLLBAR_H）
    let corner_rect = egui::Rect::from_min_max(
        egui::pos2(content_right_x, sb_y),
        egui::pos2(rect.max.x, rect.max.y),
    );
    ui.painter()
        .rect_filled(corner_rect, 0.0, theme::track_bg());

    if view.is_vertical() {
        // ── 纵向瀑布流：时间滚动条竖在右侧（绑 scroll_y / ppt），音高滚动条横在底部（绑 scroll_x / key_height）──
        // 时间竖条
        let tick_sb_rect = egui::Rect::from_min_max(
            egui::pos2(content_right_x, content_y),
            egui::pos2(rect.max.x, content_bottom),
        );
        let main_len = content_rect.height();
        let tick_sb_drag = ui
            .push_id("piano_scrollbar", |ui| {
                scrollbar::show(
                    ui,
                    tick_sb_rect,
                    main_len,
                    &mut view.base.scroll_y,
                    &mut view.base.pixels_per_tick,
                    total_ticks,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if tick_sb_drag != 0.0 {
            let factor = 1.0 - tick_sb_drag * 0.005;
            let anchor_y = tick_sb_rect.center().y - content_rect.min.y;
            view.zoom_around_y(anchor_y, factor, main_len);
            ui.ctx().request_repaint();
        }

        // 音高横条
        let key_sb_rect = egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, sb_y),
            egui::pos2(content_right_x, rect.max.y),
        );
        let cross_len = content_rect.width();
        let cell_min = cross_len / 128.0;
        let cell_max = cross_len / 12.0;
        let key_sb_drag = ui
            .push_id("piano_vscroll", |ui| {
                scrollbar::show_vertical(
                    ui,
                    key_sb_rect,
                    cross_len,
                    &mut view.base.scroll_x,
                    &mut view.key_height,
                    128,
                    cell_min,
                    cell_max,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if key_sb_drag != 0.0 {
            let factor = 1.0 - key_sb_drag * 0.005;
            let anchor_x = key_sb_rect.center().x - content_rect.min.x;
            view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：时间条上滚 = 时间缩放（沿 Y）；音高条上滚 = 音高缩放（沿 X）
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && !view_interaction::pointer_over_popup(ui.ctx())
        {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                if tick_sb_rect.contains(pos) {
                    let anchor_y = tick_sb_rect.center().y - content_rect.min.y;
                    view.zoom_around_y(anchor_y, factor, main_len);
                    ui.ctx().request_repaint();
                } else if key_sb_rect.contains(pos) {
                    let anchor_x = key_sb_rect.center().x - content_rect.min.x;
                    view.zoom_around_x(anchor_x, factor);
                    ui.ctx().request_repaint();
                }
            }
        }
    } else {
        // ── 横向（现状）：时间横条（绑 scroll_x / ppt）+ 音高竖条（绑 scroll_y / key_height）──
        let sb_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + kb_w, sb_y),
            egui::pos2(content_right_x, sb_y + scrollbar::SCROLLBAR_H),
        );
        let sb_drag_dy = ui
            .push_id("piano_scrollbar", |ui| {
                scrollbar::show(
                    ui,
                    sb_rect,
                    w as f32 - kb_w,
                    &mut view.base.scroll_x,
                    &mut view.base.pixels_per_tick,
                    total_ticks,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if sb_drag_dy != 0.0 {
            let factor = 1.0 - sb_drag_dy * 0.005;
            let anchor_x = sb_rect.center().x - content_rect.min.x;
            view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // 音高竖条
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(content_right_x, content_y),
            egui::pos2(rect.max.x, content_y + content_h),
        );
        let cell_min = content_rect.height() / 128.0;
        let cell_max = content_rect.height() / 12.0;
        let vsb_drag_dx = ui
            .push_id("piano_vscroll", |ui| {
                scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    content_rect.height(),
                    &mut view.base.scroll_y,
                    &mut view.key_height,
                    128,
                    cell_min,
                    cell_max,
                    &mut view.base.dirty,
                    pr_orientation,
                )
            })
            .inner;
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 - vsb_drag_dx * 0.005;
            let anchor_y = vsb_rect.center().y - content_rect.min.y;
            view.zoom_around_y(anchor_y, factor, content_rect.height());
            ui.ctx().request_repaint();
        }

        // 滚动条滚轮缩放：水平滚动条滚轮 = x 轴缩放；垂直滚动条滚轮 = y 轴缩放
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
            && !view_interaction::pointer_over_popup(ui.ctx())
        {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.0 / 1.1 } else { 1.1 };
                if vsb_rect.contains(pos) {
                    let anchor_y = vsb_rect.center().y - content_rect.min.y;
                    view.zoom_around_y(anchor_y, factor, content_rect.height());
                    ui.ctx().request_repaint();
                } else if sb_rect.contains(pos) {
                    let anchor_x = sb_rect.center().x - content_rect.min.x;
                    view.zoom_around_x(anchor_x, factor);
                    ui.ctx().request_repaint();
                }
            }
        }
    }
}
