//! 钢琴卷帘布局计算（方向感知：横向/纵向瀑布流）。

use eframe::egui;

use yinhe_types::PianoRollView;

/// 钢琴卷帘主布局结果。
#[allow(dead_code)]
pub(crate) struct Layout {
    pub content_rect: egui::Rect,
    pub music_rect: egui::Rect,
    pub keyboard_rect: egui::Rect,
    pub ruler_rect: egui::Rect,
    pub content_y: f32,
    pub content_bottom: f32,
    pub w: u32,
    pub h: u32,
    pub pw: u32,
    pub ph: u32,
    pub total_ticks: f64,
    pub panels_total_h: f32,
}

/// 计算钢琴卷帘布局（含横竖分支、面板高度、视口尺寸与滚动 clamp）。
///
/// `panels_natural_h` 为自动化面板自然总高度（调用方根据 `auto_ctx` 预先计算）；
/// `ppp` 为 `ui.ctx().pixels_per_point()`；
/// `total_ticks` 为工程总 tick（含 padding）。
/// 返回 `None` 表示音乐区像素尺寸为 0，无需后续渲染。
pub(crate) fn compute_layout(
    view: &mut PianoRollView,
    rect: egui::Rect,
    panels_natural_h: f32,
    ppp: f32,
    total_ticks: f64,
) -> Option<Layout> {
    let vertical = view.is_vertical();
    let kb_w = view.keyboard_width();
    let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;

    // AM 面板可用高度（横向：content 之下；纵向：content 之下、键盘之上）
    let avail_h = if vertical {
        (rect.height() - crate::theme::PR_BAR_H - crate::widgets::scrollbar::SCROLLBAR_H - kb_w)
            .max(0.0)
    } else {
        (rect.height()
            - super::types::RULER_H
            - crate::theme::PR_BAR_H
            - crate::widgets::scrollbar::SCROLLBAR_H)
            .max(0.0)
    };
    let panels_max_h = (avail_h * 0.65).max(0.0);
    let panels_total_h = panels_natural_h.min(panels_max_h);

    // 音乐区位置：横向顶部从 control_bar+ruler 之下开始（control 在上、ruler 贴内容更易操作）；纵向顶部从 control_bar 之下。
    let ruler_band_y = rect.min.y;
    let (content_y, content_bottom, content_left_x, music_left_x) = if vertical {
        let top = rect.min.y + crate::theme::PR_BAR_H;
        // 底部：key 滚动条 + 键盘条（高 kb_w）
        let keyboard_top = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H - kb_w;
        let bottom = keyboard_top - panels_total_h;
        let left = rect.min.x + super::types::RULER_H; // 竖 ruler 列
        (top, bottom.max(top), left, left)
    } else {
        let top = rect.min.y + crate::theme::PR_BAR_H + super::types::RULER_H;
        let bottom = top + (avail_h - panels_total_h).max(0.0);
        (top, bottom, rect.min.x, rect.min.x + kb_w)
    };
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(content_left_x, content_y),
        egui::pos2(content_right_x, content_bottom),
    );
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(music_left_x, content_y),
        egui::pos2(content_right_x, content_bottom),
    );
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return None;
    }

    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // 键盘条与标尺占位（与 show 中绘制时的计算一致，避免重复）。
    let keyboard_rect = if vertical {
        let kb_bottom = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H;
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, kb_bottom - kb_w),
            egui::pos2(content_right_x, kb_bottom),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, content_rect.min.y),
            egui::pos2(content_rect.min.x + kb_w, content_rect.max.y),
        )
    };
    let ruler_rect = if vertical {
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x, content_y),
            egui::pos2(rect.min.x + super::types::RULER_H, content_bottom),
        )
    } else {
        // 横向：control_bar 在最上（ruler_band_y .. +PR_BAR_H），ruler 在其下贴内容
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x + kb_w, ruler_band_y + crate::theme::PR_BAR_H),
            egui::pos2(
                content_right_x,
                ruler_band_y + crate::theme::PR_BAR_H + super::types::RULER_H,
            ),
        )
    };

    Some(Layout {
        content_rect,
        music_rect,
        keyboard_rect,
        ruler_rect,
        content_y,
        content_bottom,
        w,
        h,
        pw,
        ph,
        total_ticks,
        panels_total_h,
    })
}
