//! 标题栏绘制：标题文字、文档标签、拖拽覆盖层（插入线 / ghost / detach 提示）。
//! 交互状态机与动作判定在父模块 `title_bar`，这里只消费状态画外观。

use eframe::egui;

use yinhe_editor_core::document::Document;

use super::TitleDrag;

/// 标题栏几何常量与坐标换算（show 与绘制函数共用，避免公式散落多处）。
pub(super) struct BarMetrics {
    pub bar_rect: egui::Rect,
    /// macOS 左侧留给红绿灯的宽度。
    pub left_padding: f32,
    pub tab_w: f32,
    pub tab_h: f32,
    pub tab_y: f32,
    pub tab_gap: f32,
    pub close_w: f32,
}

impl BarMetrics {
    pub(super) fn new(bar_rect: egui::Rect) -> Self {
        let tab_h = 24.0;
        Self {
            bar_rect,
            left_padding: if cfg!(target_os = "macos") {
                80.0
            } else {
                10.0
            },
            tab_w: 120.0, // Uniform tab width: fixed 120px for compact tabs
            tab_h,
            tab_y: bar_rect.center().y - tab_h / 2.0,
            tab_gap: 2.0,
            close_w: 20.0,
        }
    }

    /// 第 i 个标签矩形（考虑滚动偏移；不裁剪可视性）。
    pub(super) fn tab_rect(&self, scroll_offset: f32, i: usize) -> egui::Rect {
        let x = self.bar_rect.min.x + self.left_padding - scroll_offset
            + i as f32 * (self.tab_w + self.tab_gap);
        egui::Rect::from_min_max(
            egui::pos2(x, self.tab_y),
            egui::pos2(x + self.tab_w, self.tab_y + self.tab_h),
        )
    }

    /// 第 i 个标签的关闭按钮矩形。
    pub(super) fn close_rect(&self, scroll_offset: f32, i: usize) -> egui::Rect {
        let tab_rect = self.tab_rect(scroll_offset, i);
        egui::Rect::from_min_size(
            egui::pos2(tab_rect.max.x - self.close_w, tab_rect.min.y),
            egui::vec2(self.close_w, self.tab_h),
        )
    }

    /// n 个标签的总宽度（含间距）。
    pub(super) fn tabs_width(&self, n: usize) -> f32 {
        n as f32 * (self.tab_w + self.tab_gap)
    }

    /// 滚动偏移上限（标签溢出量）。
    pub(super) fn max_scroll_offset(&self, n: usize) -> f32 {
        let available_w = self.bar_rect.width() - self.left_padding;
        (self.tabs_width(n) - available_w).max(0.0)
    }

    /// 空白区左界（最后一个标签布局右端，不小于 left_padding）。
    pub(super) fn blank_left(&self, scroll_offset: f32, n: usize) -> f32 {
        (self.bar_rect.min.x + self.left_padding - scroll_offset + self.tabs_width(n))
            .max(self.bar_rect.min.x + self.left_padding)
    }
}

/// 给颜色乘一个透明度系数（拖拽中的半透明标签/ghost 用）。
pub(super) fn tint(c: egui::Color32, mul: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * mul) as u8)
}

/// 绘制标题文字 + 全部文档标签（拖拽中的标签半透明 + accent 描边 + 阴影，
/// 关闭按钮在拖拽中隐藏防误触）。
pub(super) fn paint_tabs(
    painter: &egui::Painter,
    ui: &egui::Ui,
    m: &BarMetrics,
    documents: &[Document],
    active_doc: Option<usize>,
    scroll_offset: f32,
    drag: Option<&TitleDrag>,
) {
    // ── Draw title BEHIND tabs (lower z-order) ──
    painter.text(
        egui::pos2(m.bar_rect.center().x, m.bar_rect.center().y),
        egui::Align2::CENTER_CENTER,
        "Yinhe MIDI Editor",
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        crate::theme::text_secondary(),
    );

    let font_id = egui::FontId::proportional(crate::theme::BODY_FONT);
    let padding = 6.0;
    // 未保存圆点占位（直径 8 + 间距 6）
    let dirty_dot_w = 14.0;
    let text_max_w = m.tab_w - m.close_w - padding * 2.0;

    let hover_pos_val = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
    let interact_pos_val = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
    let pointer_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let dragging = drag.is_some();

    for (i, doc) in documents.iter().enumerate() {
        let is_active = active_doc == Some(i);
        let tab_rect = m.tab_rect(scroll_offset, i);

        let is_visible = tab_rect.max.x >= m.bar_rect.min.x + m.left_padding
            && tab_rect.min.x <= m.bar_rect.max.x;
        if !is_visible {
            continue;
        }

        // 拖拽中：被拖标签半透明 + 抬升阴影，其余标签正常
        let is_dragged = drag.is_some_and(|d| d.from == i);
        let detached = drag.is_some_and(|d| d.detached);
        let alpha_mul = if is_dragged && dragging {
            if detached { 0.35 } else { 0.45 }
        } else {
            1.0
        };

        // Tab background — active / pressed / hover / inactive
        let is_hovered = tab_rect.contains(hover_pos_val) && !is_active && !is_dragged;
        let pointer_down_on_tab =
            pointer_down && tab_rect.contains(interact_pos_val) && !is_dragged;
        let base_bg = if is_active {
            crate::theme::control_selected_bg()
        } else if pointer_down_on_tab && is_hovered {
            crate::theme::pressed_color(crate::theme::control_bg())
        } else if is_hovered {
            crate::theme::hover_color(crate::theme::control_bg())
        } else {
            crate::theme::control_bg()
        };
        let bg = if is_dragged && dragging {
            tint(base_bg, alpha_mul)
        } else {
            base_bg
        };
        // 被拖标签画阴影边框，突出悬浮感
        if is_dragged && dragging && !detached {
            painter.rect_filled(
                tab_rect.expand(1.0),
                4.0,
                egui::Color32::from_black_alpha(40),
            );
        }
        painter.rect_filled(tab_rect, 4.0, bg);
        // 拖拽中被拖标签加 accent 描边
        if is_dragged && dragging && !detached {
            painter.rect_stroke(
                tab_rect,
                4.0,
                egui::Stroke::new(1.5, crate::theme::accent_active()),
                egui::StrokeKind::Middle,
            );
        }

        // Build display name with dirty indicator
        let file_name = doc.file_name.as_str();
        let dirty_dot = doc.is_dirty();
        let display_name = file_name.to_string();
        // 未保存时文字可用宽度减去圆点占位
        let text_max_w_cur = text_max_w - if dirty_dot { dirty_dot_w } else { 0.0 };

        // Tab text with ellipsis truncation
        let text_color = if is_active {
            crate::theme::text_primary()
        } else {
            crate::theme::text_secondary()
        };
        let text_color_draw = if is_dragged && dragging {
            tint(text_color, alpha_mul)
        } else {
            text_color
        };
        let text_to_draw = {
            let full_w = painter
                .layout_no_wrap(display_name.clone(), font_id.clone(), text_color)
                .size()
                .x;
            if full_w <= text_max_w_cur {
                display_name
            } else {
                let ellipsis = "\u{2026}";
                let mut truncated = String::new();
                for c in display_name.chars() {
                    let test_w = painter
                        .layout_no_wrap(
                            format!("{}{}{}", truncated, c, ellipsis),
                            font_id.clone(),
                            text_color,
                        )
                        .size()
                        .x;
                    if test_w > text_max_w_cur {
                        break;
                    }
                    truncated.push(c);
                }
                format!("{}{}", truncated, ellipsis)
            }
        };
        let mut text_x = tab_rect.min.x + padding;
        if dirty_dot {
            // 未保存指示：Material 风格圆点（比文字深一点的灰色）
            let dot_color = if is_dragged && dragging {
                tint(crate::theme::tab_dirty_dot(), alpha_mul)
            } else {
                crate::theme::tab_dirty_dot()
            };
            painter.circle_filled(
                egui::pos2(text_x + 4.0, tab_rect.center().y),
                4.0,
                dot_color,
            );
            text_x += dirty_dot_w;
        }
        let text_pos = egui::pos2(text_x, tab_rect.center().y);
        painter.text(
            text_pos,
            egui::Align2::LEFT_CENTER,
            text_to_draw,
            font_id.clone(),
            text_color_draw,
        );

        // Close button（拖拽中隐藏，避免误触）
        if !dragging {
            let tab_close_rect = m.close_rect(scroll_offset, i);
            let close_hover = tab_close_rect.contains(hover_pos_val);
            let close_pressed =
                close_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            crate::chrome::dialog::paint_close_button(
                painter,
                tab_close_rect,
                close_hover,
                close_pressed,
            );
        }
    }
}

/// 拖拽覆盖层：插入竖线 + 跟随 ghost（排序态），或 detach 幽灵标签 + 提示气泡。
pub(super) fn paint_drag_overlay(
    painter: &egui::Painter,
    ui: &egui::Ui,
    m: &BarMetrics,
    documents: &[Document],
    drag: &TitleDrag,
    tab_rects: &[egui::Rect],
) {
    let font_id = egui::FontId::proportional(crate::theme::BODY_FONT);
    let pointer_pos = ui.input(|i| i.pointer.interact_pos());

    if drag.detached {
        let Some(cur) = pointer_pos else { return };
        // Detach 幽灵标签跟随指针
        let ghost_rect = egui::Rect::from_center_size(cur, egui::vec2(m.tab_w, m.tab_h));
        painter.rect_filled(
            ghost_rect,
            4.0,
            crate::theme::hover_color(crate::theme::control_selected_bg()),
        );
        painter.rect_stroke(
            ghost_rect,
            4.0,
            egui::Stroke::new(1.5, crate::theme::accent_active()),
            egui::StrokeKind::Middle,
        );
        // 文字
        let name = documents[drag.from].file_name.as_str();
        painter.text(
            ghost_rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            font_id.clone(),
            crate::theme::text_primary(),
        );

        // 提示气泡「松开以在新窗口打开」
        let tip = rust_i18n::t!("title_bar.detach_hint").to_string();
        let tip_font = egui::FontId::proportional(crate::theme::SMALL_FONT);
        let galley =
            painter.layout_no_wrap(tip.clone(), tip_font.clone(), crate::theme::text_primary());
        let tip_pad = egui::vec2(8.0, 4.0);
        let tip_size = galley.size() + tip_pad * 2.0;
        let mut tip_pos = egui::pos2(cur.x + 14.0, cur.y + 18.0);
        // 防止超出窗口
        tip_pos.x = tip_pos.x.min(m.bar_rect.max.x - tip_size.x - 4.0);
        let tip_rect = egui::Rect::from_min_size(tip_pos, tip_size);
        painter.rect_filled(
            tip_rect,
            4.0,
            crate::theme::pressed_color(crate::theme::control_selected_bg()),
        );
        painter.text(
            tip_rect.center(),
            egui::Align2::CENTER_CENTER,
            tip,
            tip_font,
            crate::theme::text_bright(),
        );
        return;
    }

    // ── 排序态：插入竖线 + 跟随 ghost ──
    let tmp = crate::widgets::reorder::DragReorder {
        indices: vec![drag.from],
        insert_idx: drag.insert_idx,
    };
    if let Some(x) = tmp.insert_line_x(tab_rects) {
        // clamp 到标题栏可视区
        let x_clamped = x.clamp(m.bar_rect.min.x + m.left_padding, m.bar_rect.max.x - 1.0);
        let y1 = m.bar_rect.min.y + 4.0;
        let y2 = m.bar_rect.max.y - 4.0;
        painter.line_segment(
            [egui::pos2(x_clamped, y1), egui::pos2(x_clamped, y2)],
            egui::Stroke::new(2.5, crate::theme::accent_active()),
        );
        // 顶部小圆点装饰
        painter.circle_filled(
            egui::pos2(x_clamped, y1),
            3.0,
            crate::theme::accent_active(),
        );
        painter.circle_filled(
            egui::pos2(x_clamped, y2),
            3.0,
            crate::theme::accent_active(),
        );
    }
    // 跟随指针的半透明 ghost（轻量）
    if let Some(cur) = pointer_pos {
        let ghost_rect = egui::Rect::from_center_size(cur, egui::vec2(m.tab_w * 0.92, m.tab_h));
        let name = documents[drag.from].file_name.as_str();
        painter.rect_filled(
            ghost_rect,
            4.0,
            tint(crate::theme::control_selected_bg(), 0.55),
        );
        painter.text(
            ghost_rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            font_id.clone(),
            tint(crate::theme::text_primary(), 0.55),
        );
    }
}
