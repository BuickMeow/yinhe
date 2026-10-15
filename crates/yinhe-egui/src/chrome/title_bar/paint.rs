//! 标题栏绘制：标题文字、文档标签、拖拽覆盖层（插入线 / ghost / detach 提示）。
//! 交互状态机与动作判定在父模块 `title_bar`，这里只消费状态画外观。

use eframe::egui;

use yinhe_editor_core::document::Document;

use super::TitleDrag;

/// 标题栏几何常量与坐标换算（show 与绘制函数共用，避免公式散落多处）。
pub(super) struct BarMetrics {
    pub bar_rect: egui::Rect,
    /// macOS 左侧留给红绿灯的宽度（原生全屏时红绿灯隐藏，收回为普通内边距）。
    pub left_padding: f32,
    pub tab_h: f32,
    pub tab_y: f32,
    pub tab_gap: f32,
}

/// 标签最小宽度（短名不至于太窄，保证点击目标）。
const MIN_TAB_W: f32 = 120.0;
/// 标签最大宽度（超长工程名截断，悬停跑马灯展示）。
const MAX_TAB_W: f32 = 400.0;
/// 标签文字左右内边距。
const TAB_PADDING: f32 = 6.0;
/// 未保存圆点占位（直径 8 + 间距 6）。
const DIRTY_DOT_W: f32 = 14.0;

impl BarMetrics {
    /// `fullscreen`：窗口是否处于 macOS 原生全屏（全屏到新桌面时系统红绿灯不显示，
    /// 无需为它留空）。
    pub(super) fn new(bar_rect: egui::Rect, fullscreen: bool) -> Self {
        let tab_h = 24.0;
        let reserve_traffic_lights = cfg!(target_os = "macos") && !fullscreen;
        Self {
            bar_rect,
            left_padding: if reserve_traffic_lights { 80.0 } else { 10.0 },
            tab_h,
            tab_y: bar_rect.center().y - tab_h / 2.0,
            tab_gap: 2.0,
        }
    }

    /// 标签总宽度（含尾部间距，与原固定宽度公式一致）。
    pub(super) fn tabs_width(&self, widths: &[f32]) -> f32 {
        widths.iter().sum::<f32>() + widths.len() as f32 * self.tab_gap
    }

    /// 滚动偏移上限（标签溢出量）。
    pub(super) fn max_scroll_offset(&self, widths: &[f32]) -> f32 {
        let available_w = self.bar_rect.width() - self.left_padding;
        (self.tabs_width(widths) - available_w).max(0.0)
    }

    /// 空白区左界（最后一个标签布局右端，不小于 left_padding）。
    pub(super) fn blank_left(&self, scroll_offset: f32, widths: &[f32]) -> f32 {
        (self.bar_rect.min.x + self.left_padding - scroll_offset + self.tabs_width(widths))
            .max(self.bar_rect.min.x + self.left_padding)
    }
}

/// 每个文档的标签宽度：按工程名实测（未保存圆点占位 + 左右内边距），
/// clamp 到 `[MIN_TAB_W, MAX_TAB_W]`——动态长度，最长 400。
pub(super) fn tab_widths(painter: &egui::Painter, documents: &[Document]) -> Vec<f32> {
    let font = egui::FontId::proportional(crate::theme::BODY_FONT);
    documents
        .iter()
        .map(|doc| {
            let text_w = painter
                .layout_no_wrap(doc.file_name.clone(), font.clone(), egui::Color32::WHITE)
                .size()
                .x;
            let dot_w = if doc.is_dirty() { DIRTY_DOT_W } else { 0.0 };
            (text_w + dot_w + TAB_PADDING * 2.0).clamp(MIN_TAB_W, MAX_TAB_W)
        })
        .collect()
}

/// 全部标签矩形（按各标签动态宽度排布，考虑滚动偏移；不裁剪可视性）。
pub(super) fn tab_rects(m: &BarMetrics, scroll_offset: f32, widths: &[f32]) -> Vec<egui::Rect> {
    let mut x = m.bar_rect.min.x + m.left_padding - scroll_offset;
    widths
        .iter()
        .map(|&w| {
            let rect = egui::Rect::from_min_max(
                egui::pos2(x, m.tab_y),
                egui::pos2(x + w, m.tab_y + m.tab_h),
            );
            x += w + m.tab_gap;
            rect
        })
        .collect()
}

/// 给颜色乘一个透明度系数（拖拽中的半透明标签/ghost 用）。
pub(super) fn tint(c: egui::Color32, mul: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * mul) as u8)
}

/// 标题栏中央的应用名（可点击：打开欢迎窗口）。
pub(super) const APP_TITLE: &str = "Yinhe";

/// 应用名「Yinhe」的可点击矩形（文字实测 + 少量内边距，居中于标题栏）。
pub(super) fn title_rect(painter: &egui::Painter, m: &BarMetrics) -> egui::Rect {
    let galley = painter.layout_no_wrap(
        APP_TITLE.to_string(),
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        egui::Color32::WHITE,
    );
    egui::Rect::from_center_size(m.bar_rect.center(), galley.size() + egui::vec2(12.0, 2.0))
}

/// 绘制标题文字 + 全部文档标签（拖拽中的标签半透明 + accent 描边 + 阴影）。
/// 常态无按钮背景（只显示未保存圆点 + 工程名），悬停/按下才有底色。
pub(super) fn paint_tabs(
    painter: &egui::Painter,
    ui: &egui::Ui,
    m: &BarMetrics,
    documents: &[Document],
    active_doc: Option<usize>,
    tab_rects: &[egui::Rect],
    drag: Option<&TitleDrag>,
) {
    // ── Draw title BEHIND tabs (lower z-order)；悬停有底色，可点击 ──
    let title = title_rect(painter, m);
    let title_hovered = drag.is_none() && ui.rect_contains_pointer(title);
    if title_hovered {
        painter.rect_filled(
            title,
            4.0,
            crate::theme::hover_color(crate::theme::app_bg()),
        );
    }
    painter.text(
        title.center(),
        egui::Align2::CENTER_CENTER,
        APP_TITLE,
        egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
        if title_hovered {
            crate::theme::text_primary()
        } else {
            crate::theme::text_secondary()
        },
    );

    let font_id = egui::FontId::proportional(crate::theme::BODY_FONT);
    let padding = TAB_PADDING;
    let dirty_dot_w = DIRTY_DOT_W;

    let hover_pos_val = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();
    let interact_pos_val = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
    let pointer_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let dragging = drag.is_some();

    for (i, doc) in documents.iter().enumerate() {
        let is_active = active_doc == Some(i);
        let tab_rect = tab_rects[i];

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

        // Tab background — 常态透明（极简），仅悬停/按下给底色
        let is_hovered = tab_rect.contains(hover_pos_val) && !is_dragged;
        let pointer_down_on_tab =
            pointer_down && tab_rect.contains(interact_pos_val) && !is_dragged;
        let base_bg = crate::theme::btn_bg();
        let bg = if is_dragged && dragging {
            // 拖拽中保持可见：用按钮底色并叠透明度
            tint(base_bg, alpha_mul)
        } else if pointer_down_on_tab && is_hovered {
            crate::theme::pressed_color(crate::theme::app_bg())
        } else if is_hovered {
            crate::theme::hover_color(crate::theme::app_bg())
        } else {
            egui::Color32::TRANSPARENT
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
        // 拖拽中被拖标签加 accent 描边（选中/未选中的区分改为文字色，不再描边）
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
        // 本标签动态宽度内、扣掉左右内边距与圆点占位后的文字可用宽度
        let text_max_w_cur =
            tab_rect.width() - padding * 2.0 - if dirty_dot { dirty_dot_w } else { 0.0 };

        // Tab text with ellipsis truncation
        let text_color = if is_active {
            crate::theme::text_primary()
        } else {
            crate::theme::text_disabled()
        };
        let text_color_draw = if is_dragged && dragging {
            tint(text_color, alpha_mul)
        } else {
            text_color
        };
        // 完整名称宽度（跑马灯判定与截断逻辑共用一次测量）
        let full_w = painter
            .layout_no_wrap(display_name.clone(), font_id.clone(), text_color)
            .size()
            .x;

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

        // 悬停且名称超宽（静态显示会被截断）→ 跑马灯横向滚动完整名称：
        // 文字尾部留 MARQUEE_GAP 间隔后接开头，无限循环。被拖拽的标签不滚动。
        let marquee_id = ui.id().with(("tab_marquee_start", i));
        if is_hovered && full_w > text_max_w_cur {
            // 先取 time 再进 data_mut（在 data_mut 闭包里调用 ui.input 会死锁）
            let time = ui.input(|i| i.time);
            let start = ui.data_mut(|d| *d.get_temp_mut_or_insert_with(marquee_id, || time));
            const MARQUEE_GAP: f32 = 48.0;
            const MARQUEE_SPEED: f32 = 30.0;
            let elapsed = (time - start) as f32;
            let cycle = full_w + MARQUEE_GAP;
            let offset = (elapsed * MARQUEE_SPEED) % cycle;
            // 只裁剪文字区 [text_x, 标签右缘-padding]，未保存圆点不受影响
            let clip = egui::Rect::from_min_max(
                egui::pos2(text_x, tab_rect.min.y),
                egui::pos2(tab_rect.max.x - padding, tab_rect.max.y),
            );
            let clipped = painter.with_clip_rect(clip);
            let x1 = text_x - offset;
            for x in [x1, x1 + cycle] {
                clipped.text(
                    egui::pos2(x, tab_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    display_name.clone(),
                    font_id.clone(),
                    text_color_draw,
                );
            }
            ui.ctx().request_repaint();
        } else {
            // 未悬停/未被截断：清掉计时，下次悬停重新从文字开头滚动
            ui.data_mut(|d| d.remove::<f64>(marquee_id));
            // Tab text with ellipsis truncation
            let text_to_draw = if full_w <= text_max_w_cur {
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
            };
            painter.text(
                egui::pos2(text_x, tab_rect.center().y),
                egui::Align2::LEFT_CENTER,
                text_to_draw,
                font_id.clone(),
                text_color_draw,
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
        // Detach 幽灵标签跟随指针（宽度取被拖标签的动态宽度）
        let ghost_w = tab_rects.get(drag.from).map_or(MIN_TAB_W, |r| r.width());
        let ghost_rect = egui::Rect::from_center_size(cur, egui::vec2(ghost_w, m.tab_h));
        painter.rect_filled(
            ghost_rect,
            4.0,
            crate::theme::hover_color(crate::theme::btn_bg()),
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
            crate::theme::pressed_color(crate::theme::btn_bg()),
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

    // ── 排序态：插入竖线 ──
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// macOS 窗口模式为红绿灯留 80px；原生全屏（新桌面，红绿灯隐藏）收回为 10px。
    #[test]
    fn left_padding_follows_fullscreen() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1200.0, 32.0));
        let windowed = BarMetrics::new(rect, false);
        let fullscreen = BarMetrics::new(rect, true);
        if cfg!(target_os = "macos") {
            assert_eq!(windowed.left_padding, 80.0);
        } else {
            assert_eq!(windowed.left_padding, 10.0);
        }
        assert_eq!(fullscreen.left_padding, 10.0);
    }
}
