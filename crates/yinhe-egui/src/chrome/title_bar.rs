use eframe::egui;

use yinhe_editor_core::document::Document;

/// Height of the custom title bar.
pub(crate) const TITLE_BAR_HEIGHT: f32 = crate::theme::TITLE_BAR_H;

/// Action to be performed by the caller after title bar rendering.
pub(crate) enum TitleBarAction {
    CloseDocument(usize),
    /// 拖动单个标签排序：把 from 移动到剩余列表的 insert_at 位置
    ReorderTab {
        from: usize,
        insert_at: usize,
    },
    /// 拖出到新窗口：把标签 idx 的工程在新进程打开
    DetachTab(usize),
}

/// 标签拖拽跨帧状态（标题栏横向，单标签）
#[derive(Clone)]
struct TitleDrag {
    from: usize,
    insert_idx: usize,
    detached: bool,
}

/// 按下但尚未达到拖拽阈值的待定状态（整个标题栏任意位置）。
#[derive(Clone)]
struct TitlePress {
    /// 按下的标签索引；不在任何标签上时为 None（空白/窗口按钮区域）。
    idx: Option<usize>,
    pos: egui::Pos2,
}

/// 给颜色乘一个透明度系数（拖拽中的半透明标签/ghost 用）。
fn tint(c: egui::Color32, mul: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * mul) as u8)
}

/// Draw the custom title bar at the top of the window.
/// Returns an optional action for the caller to perform (e.g. close a document).
pub(crate) fn show(
    ui: &mut egui::Ui,
    documents: &[Document],
    active_doc: &mut Option<usize>,
    tab_scroll_offset: &mut f32,
    status_hint: &mut Option<String>,
) -> Option<TitleBarAction> {
    let mut action = None;
    egui::Panel::top("title_bar")
        .show_separator_line(false)
        .frame(egui::Frame {
            fill: crate::theme::app_bg(),
            inner_margin: egui::Margin::ZERO,
            outer_margin: egui::Margin::ZERO,
            ..Default::default()
        })
        .show(ui, |ui| {
            let bar_rect = ui.max_rect();
            let painter = ui.painter().clone();

            // macOS: leave ~80px on the left for traffic lights
            let left_padding = if cfg!(target_os = "macos") {
                80.0
            } else {
                10.0
            };

            // ── Draw tabs (left side) ──
            let tab_h = 24.0;
            let tab_y = bar_rect.center().y - tab_h / 2.0;

            let tmp_docs: Vec<(&Document, bool)> = documents
                .iter()
                .enumerate()
                .map(|(i, d)| (d, *active_doc == Some(i)))
                .collect();

            let font_id = egui::FontId::proportional(crate::theme::BODY_FONT);
            let close_w = 20.0;
            let padding = 6.0;
            let tab_gap = 2.0;

            // Uniform tab width: fixed 120px for compact tabs
            let tab_w = 120.0;
            // 未保存圆点占位（直径 8 + 间距 6）
            let dirty_dot_w = 14.0;
            let text_max_w = tab_w - close_w - padding * 2.0;

            // ── Precompute tab/close rects for hit-testing and reorder ──
            let mut tab_rects: Vec<egui::Rect> = Vec::with_capacity(tmp_docs.len());
            let mut close_rects: Vec<egui::Rect> = Vec::with_capacity(tmp_docs.len());
            let mut tab_x_for_rects = bar_rect.min.x + left_padding - *tab_scroll_offset;
            for _ in 0..tmp_docs.len() {
                let tab_rect = egui::Rect::from_min_max(
                    egui::pos2(tab_x_for_rects, tab_y),
                    egui::pos2(tab_x_for_rects + tab_w, tab_y + tab_h),
                );
                let close_rect = egui::Rect::from_min_size(
                    egui::pos2(tab_rect.max.x - close_w, tab_rect.min.y),
                    egui::vec2(close_w, tab_h),
                );
                tab_rects.push(tab_rect);
                close_rects.push(close_rect);
                tab_x_for_rects += tab_w + tab_gap;
            }

            // ── Handle mouse wheel / trackpad scroll for tab overflow ──
            let pointer_in_bar =
                ui.input(|i| i.pointer.hover_pos().is_some_and(|p| bar_rect.contains(p)));
            // 状态栏讲解行：鼠标在标题栏上时清空（标题栏不属于任何可讲解区域）
            if pointer_in_bar {
                *status_hint = None;
            }
            // 拖拽中不响应滚轮（避免插入位置跳动）
            let drag_id = ui.id().with("title_bar_drag");
            let press_id = ui.id().with("title_bar_press");
            let mut drag: Option<TitleDrag> = ui.data_mut(|d| d.get_temp(drag_id));
            let mut press: Option<TitlePress> = ui.data_mut(|d| d.get_temp(press_id));
            let is_dragging = drag.is_some();
            if pointer_in_bar && !is_dragging {
                let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
                let zoom_delta = ui.input(|i| i.zoom_delta());

                // Mouse wheel horizontal scroll: Cmd+scroll or plain horizontal scroll
                let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
                if cmd && scroll_delta.y.abs() > 0.5 {
                    // Cmd+vertical scroll → tab horizontal scroll
                    *tab_scroll_offset -= scroll_delta.y * 2.0;
                } else if scroll_delta.x.abs() > 0.5 {
                    // Trackpad horizontal swipe → tab scroll
                    *tab_scroll_offset -= scroll_delta.x;
                } else if (zoom_delta - 1.0).abs() > 0.001 {
                    // Trackpad pinch → tab scroll (zoom gesture repurposed for tab scroll)
                    *tab_scroll_offset -= (zoom_delta - 1.0) * 100.0;
                } else if !cmd && scroll_delta.y.abs() > 0.5 && cfg!(target_os = "macos") {
                    // Plain vertical scroll on macOS → also scroll tabs if overflow
                    *tab_scroll_offset -= scroll_delta.y * 2.0;
                }

                // Clamp scroll offset
                let total_tab_w = tmp_docs.len() as f32 * (tab_w + tab_gap);
                let available_w = bar_rect.width() - left_padding;
                let max_offset = (total_tab_w - available_w).max(0.0);
                *tab_scroll_offset = tab_scroll_offset.clamp(0.0, max_offset);

                if scroll_delta != egui::Vec2::ZERO || (zoom_delta - 1.0).abs() > 0.001 {
                    ui.ctx().request_repaint();
                }
            }

            // ── Press / drag detection ──
            let hover_pos = ui.input(|i| i.pointer.hover_pos());
            let interact_pos = ui.input(|i| i.pointer.interact_pos());
            let pointer_pos = interact_pos.or(hover_pos);
            let pointer_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let button_pressed =
                ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let button_released =
                ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            let any_released = ui.input(|i| i.pointer.any_released());

            // 记录按下起点（标题栏任意位置；标签上额外记下索引）
            if drag.is_none()
                && button_pressed
                && let Some(pos) = pointer_pos
                && bar_rect.contains(pos)
            {
                let idx = tab_rects.iter().position(|r| r.contains(pos));
                press = Some(TitlePress { idx, pos });
            }

            // 标签上按下后移动超过阈值则进入拖拽（关闭按钮区域不触发）
            if drag.is_none()
                && let (Some(p), Some(cur)) = (press.clone(), pointer_pos)
                && pointer_down
                && (cur - p.pos).length() > 6.0
                && let Some(from) = p.idx
            {
                let press_on_close = close_rects.get(from).is_some_and(|c| c.contains(p.pos));
                if !press_on_close {
                    let mut tmp = crate::widgets::reorder::DragReorder {
                        indices: vec![from],
                        insert_idx: 0,
                    };
                    tmp.update_insert_idx_horizontal(cur.x, &tab_rects);
                    drag = Some(TitleDrag {
                        from,
                        insert_idx: tmp.insert_idx,
                        detached: false,
                    });
                    press = None;
                    ui.ctx().request_repaint();
                }
            }

            // 拖拽中：更新插入位置、detach 标记与自动滚动
            if let Some(d) = drag.as_mut()
                && let Some(cur) = pointer_pos
            {
                let mut tmp = crate::widgets::reorder::DragReorder {
                    indices: vec![d.from],
                    insert_idx: d.insert_idx,
                };
                tmp.update_insert_idx_horizontal(cur.x, &tab_rects);
                d.insert_idx = tmp.insert_idx;
                d.detached = cur.y < bar_rect.min.y - 18.0 || cur.y > bar_rect.max.y + 30.0;
                if !d.detached {
                    // 边缘自动滚动（仅标签溢出时有效）
                    const MARGIN: f32 = 40.0;
                    const SPEED: f32 = 20.0;
                    if cur.x < bar_rect.min.x + left_padding + MARGIN {
                        *tab_scroll_offset = (*tab_scroll_offset - SPEED).max(0.0);
                    } else if cur.x > bar_rect.max.x - MARGIN {
                        let total_tab_w = tmp_docs.len() as f32 * (tab_w + tab_gap);
                        let available_w = bar_rect.width() - left_padding;
                        let max_offset = (total_tab_w - available_w).max(0.0);
                        *tab_scroll_offset = (*tab_scroll_offset + SPEED).min(max_offset);
                    }
                }
                ui.ctx().request_repaint();
            }

            // ── Draw title BEHIND tabs (lower z-order) ──
            painter.text(
                egui::pos2(bar_rect.center().x, bar_rect.center().y),
                egui::Align2::CENTER_CENTER,
                "Yinhe MIDI Editor",
                egui::FontId::proportional(crate::theme::SUB_TITLE_FONT),
                crate::theme::text_secondary(),
            );

            let hover_pos_val = hover_pos.unwrap_or_default();
            let interact_pos_val = interact_pos.unwrap_or_default();

            // 重算带当前滚动偏移的 tab_x（滚动可能在拖拽中自动改变）
            let mut tab_x = bar_rect.min.x + left_padding - *tab_scroll_offset;

            for (i, (doc, is_active)) in tmp_docs.iter().enumerate() {
                let tab_rect = egui::Rect::from_min_max(
                    egui::pos2(tab_x, tab_y),
                    egui::pos2(tab_x + tab_w, tab_y + tab_h),
                );

                let is_visible = tab_rect.max.x >= bar_rect.min.x + left_padding
                    && tab_rect.min.x <= bar_rect.max.x;
                if !is_visible {
                    tab_x += tab_w + tab_gap;
                    continue;
                }

                // 拖拽中：被拖标签半透明 + 抬升阴影，其余标签正常
                let is_dragged = drag.as_ref().is_some_and(|d| d.from == i);
                let dragging = drag.is_some();
                let detached = drag.as_ref().is_some_and(|d| d.detached);
                let alpha_mul = if is_dragged && dragging {
                    if detached { 0.35 } else { 0.45 }
                } else {
                    1.0
                };

                // Tab background — active / pressed / hover / inactive
                let is_hovered = tab_rect.contains(hover_pos_val) && !*is_active && !is_dragged;
                let pointer_down_on_tab =
                    pointer_down && tab_rect.contains(interact_pos_val) && !is_dragged;
                let base_bg = if *is_active {
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
                let text_color = if *is_active {
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
                let tab_close_rect = egui::Rect::from_min_size(
                    egui::pos2(tab_rect.max.x - close_w, tab_rect.min.y),
                    egui::vec2(close_w, tab_h),
                );
                if !dragging {
                    let close_hover = tab_close_rect.contains(hover_pos_val);
                    let close_pressed = close_hover
                        && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
                    crate::chrome::dialog::paint_close_button(
                        &painter,
                        tab_close_rect,
                        close_hover,
                        close_pressed,
                    );
                }

                tab_x += tab_w + tab_gap;
            }

            // ── 拖拽插入线 / ghost / detach 提示 ──
            if let Some(d) = drag.as_ref() {
                if d.detached {
                    // Detach 幽灵标签跟随指针
                    if let Some(cur) = pointer_pos {
                        let ghost_w = tab_w;
                        let ghost_h = tab_h;
                        let ghost_rect =
                            egui::Rect::from_center_size(cur, egui::vec2(ghost_w, ghost_h));
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
                        let name = documents
                            .get(d.from)
                            .map(|doc| doc.file_name.as_str())
                            .unwrap_or("Untitled");
                        painter.text(
                            ghost_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            name,
                            font_id.clone(),
                            crate::theme::text_primary(),
                        );
                        // 提示气泡
                        let tip = rust_i18n::t!("title_bar.detach_hint").to_string();
                        let tip_font = egui::FontId::proportional(crate::theme::SMALL_FONT);
                        let galley = painter.layout_no_wrap(
                            tip.to_string(),
                            tip_font.clone(),
                            crate::theme::text_primary(),
                        );
                        let tip_pad = egui::vec2(8.0, 4.0);
                        let tip_size = galley.size() + tip_pad * 2.0;
                        let mut tip_pos = egui::pos2(cur.x + 14.0, cur.y + 18.0);
                        // 防止超出窗口
                        tip_pos.x = tip_pos.x.min(bar_rect.max.x - tip_size.x - 4.0);
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
                    }
                } else {
                    // 插入竖线（仅在非 detach 时）
                    let tmp = crate::widgets::reorder::DragReorder {
                        indices: vec![d.from],
                        insert_idx: d.insert_idx,
                    };
                    if let Some(x) = tmp.insert_line_x(&tab_rects) {
                        // clamp 到标题栏可视区
                        let x_clamped =
                            x.clamp(bar_rect.min.x + left_padding, bar_rect.max.x - 1.0);
                        let y1 = bar_rect.min.y + 4.0;
                        let y2 = bar_rect.max.y - 4.0;
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
                    // 非 detach 时也画跟随的半透明 ghost（轻量）
                    if let Some(cur) = pointer_pos {
                        let ghost_rect =
                            egui::Rect::from_center_size(cur, egui::vec2(tab_w * 0.92, tab_h));
                        let name = documents
                            .get(d.from)
                            .map(|doc| doc.file_name.as_str())
                            .unwrap_or("Untitled");
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
            }

            // ── Window button rects (non-macOS) ──
            #[cfg(not(target_os = "macos"))]
            let win_btn_rects = {
                let btn_w = 46.0;
                let btn_h = TITLE_BAR_HEIGHT;
                let btn_y = bar_rect.min.y;

                let c = egui::Rect::from_min_size(
                    egui::pos2(bar_rect.max.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                let mx = egui::Rect::from_min_size(
                    egui::pos2(c.min.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                let mn = egui::Rect::from_min_size(
                    egui::pos2(mx.min.x - btn_w, btn_y),
                    egui::vec2(btn_w, btn_h),
                );
                (c, mx, mn)
            };

            // ── Press/drag 结束后的点击 / 排序 / detach 判定 ──
            // 拖拽中主键已抬起但没收到 release 事件（指针离开视口被吞）→ 视为释放
            let drag_stuck_release = drag.is_some() && !ui.input(|i| i.pointer.any_down());
            if any_released || drag_stuck_release {
                if let Some(d) = drag.take() {
                    if d.detached {
                        action = Some(TitleBarAction::DetachTab(d.from));
                    } else {
                        let order = crate::widgets::reorder::plan_order(
                            documents.len(),
                            &[d.from],
                            d.insert_idx,
                        );
                        let cur_order: Vec<usize> = (0..documents.len()).collect();
                        if order != cur_order {
                            action = Some(TitleBarAction::ReorderTab {
                                from: d.from,
                                insert_at: d.insert_idx,
                            });
                        } else {
                            *active_doc = Some(d.from);
                        }
                    }
                    press = None;
                } else if button_released
                    && let Some(p) = press.take()
                    && let Some(release) = pointer_pos
                    && (release - p.pos).length() < 8.0
                {
                    // 标签 / 关闭按钮点击（按下与松开都在同一矩形内）
                    if let Some(i) = p.idx {
                        let cr = close_rects[i];
                        let tr = tab_rects[i];
                        if cr.contains(p.pos) && cr.contains(release) {
                            action = Some(TitleBarAction::CloseDocument(i));
                        } else if tr.contains(p.pos) && tr.contains(release) {
                            *active_doc = Some(i);
                        }
                    } else {
                        // 窗口按钮（仅非 macOS；macOS 用系统红绿灯）
                        #[cfg(not(target_os = "macos"))]
                        {
                            if win_btn_rects.0.contains(p.pos) && win_btn_rects.0.contains(release)
                            {
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                            } else if win_btn_rects.1.contains(p.pos)
                                && win_btn_rects.1.contains(release)
                            {
                                let maximized =
                                    ui.input(|i| i.viewport().maximized.unwrap_or(false));
                                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(
                                    !maximized,
                                ));
                            } else if win_btn_rects.2.contains(p.pos)
                                && win_btn_rects.2.contains(release)
                            {
                                ui.ctx()
                                    .send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                    }
                }
            }

            // ── Paint window buttons (non-macOS, visual only) ──
            #[cfg(not(target_os = "macos"))]
            {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                paint_window_buttons(ui, &win_btn_rects, maximized);
            }

            // ── Register interact for each tab to prevent drag from claiming clicks on tabs ──
            // 拖拽中不注册，避免干扰
            if drag.is_none() {
                for (i, tr) in tab_rects.iter().enumerate() {
                    // 仅对可见标签注册
                    if tr.max.x < bar_rect.min.x + left_padding || tr.min.x > bar_rect.max.x {
                        continue;
                    }
                    ui.interact(*tr, ui.id().with("tab_block").with(i), egui::Sense::click());
                }
            }

            // ── Window drag region (after the tabs, excluding window buttons) ──
            let drag_right = if cfg!(target_os = "macos") {
                bar_rect.max.x
            } else {
                bar_rect.max.x - 138.0
            };
            // 重新计算 tab 区域最右端（考虑当前滚动）
            let cur_tab_x = bar_rect.min.x + left_padding - *tab_scroll_offset
                + tmp_docs.len() as f32 * (tab_w + tab_gap);
            let drag_rect_left = cur_tab_x.max(bar_rect.min.x + left_padding);
            let drag_rect = egui::Rect::from_min_max(
                egui::pos2(drag_rect_left, bar_rect.min.y),
                egui::pos2(drag_right, bar_rect.max.y),
            );

            // 标签拖拽中禁用窗口拖动（空白区的窗口拖动走下方 drag_rect，
            // 与标签矩形不重叠，按下待定状态不影响它）
            let can_window_drag = drag.is_none();
            if can_window_drag {
                let drag_resp = ui.interact(drag_rect, ui.next_auto_id(), egui::Sense::drag());
                if drag_resp.dragged_by(egui::PointerButton::Primary) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
            }

            // Double-click title bar drag area to toggle maximize/restore
            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.id().with("title_bar_dbl_click");
            if can_window_drag
                && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = pointer_pos
                && drag_rect.contains(pos)
            {
                let now = ui.input(|i| i.time);
                let last_click: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
                if now - last_click < DOUBLE_CLICK_MS / 1000.0 {
                    let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
                    ui.data_mut(|d| d.insert_persisted(dbl_id, 0.0)); // reset
                } else {
                    ui.data_mut(|d| d.insert_persisted(dbl_id, now));
                }
            }

            // Persist drag/press state
            ui.data_mut(|d| {
                if drag.is_some() {
                    d.insert_temp(drag_id, drag.clone().unwrap());
                } else {
                    d.remove::<TitleDrag>(drag_id);
                }
                if press.is_some() {
                    d.insert_temp(press_id, press.clone().unwrap());
                } else {
                    d.remove::<TitlePress>(press_id);
                }
            });

            // Reserve space for title bar height
            ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
        });
    action
}

/// Paint the three window control buttons (close, maximize, minimize) for non-macOS platforms.
#[cfg(not(target_os = "macos"))]
fn paint_window_buttons(
    ui: &egui::Ui,
    rects: &(egui::Rect, egui::Rect, egui::Rect),
    maximized: bool,
) {
    let painter = ui.painter();
    let (close_rect, maximize_rect, minimize_rect) = rects;

    // ── Close button (red on hover) ──
    let close_hover = close_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let close_pressed =
        close_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let close_bg = if close_pressed {
        crate::theme::pressed_color(crate::theme::danger())
    } else if close_hover {
        crate::theme::danger()
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*close_rect, 0.0, close_bg);
    // X icon
    let x_color = if close_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let cx = close_rect.center();
    let x_size = 8.0;
    let x_half = x_size / 2.0;
    let x1 = egui::pos2(cx.x - x_half, cx.y - x_half);
    let x2 = egui::pos2(cx.x + x_half, cx.y + x_half);
    let x3 = egui::pos2(cx.x + x_half, cx.y - x_half);
    let x4 = egui::pos2(cx.x - x_half, cx.y + x_half);
    painter.line_segment([x1, x2], (1.5, x_color));
    painter.line_segment([x3, x4], (1.5, x_color));

    // ── Maximize button ──
    let max_hover = maximize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let max_pressed =
        max_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let max_bg = if max_pressed {
        crate::theme::pressed_color(crate::theme::app_bg())
    } else if max_hover {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*maximize_rect, 0.0, max_bg);
    let max_color = if max_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let mcx = maximize_rect.center();
    let m_size = 9.0;
    if maximized {
        let r1 = egui::Rect::from_center_size(
            egui::pos2(mcx.x - 2.0, mcx.y - 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        let r2 = egui::Rect::from_center_size(
            egui::pos2(mcx.x + 2.0, mcx.y + 2.0),
            egui::vec2(m_size - 2.0, m_size - 2.0),
        );
        painter.rect_stroke(r1, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
        painter.rect_stroke(r2, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    } else {
        let r = egui::Rect::from_center_size(mcx, egui::vec2(m_size, m_size));
        painter.rect_stroke(r, 1.0, (1.5, max_color), egui::StrokeKind::Middle);
    }

    // ── Minimize button ──
    let min_hover = minimize_rect.contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default());
    let min_pressed =
        min_hover && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let min_bg = if min_pressed {
        crate::theme::pressed_color(crate::theme::app_bg())
    } else if min_hover {
        crate::theme::hover_color(crate::theme::app_bg())
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(*minimize_rect, 0.0, min_bg);
    let min_color = if min_hover {
        crate::theme::contrast_fg()
    } else {
        crate::theme::text_label()
    };
    let mn_cx = minimize_rect.center();
    let line_y = mn_cx.y;
    painter.line_segment(
        [
            egui::pos2(mn_cx.x - 5.0, line_y),
            egui::pos2(mn_cx.x + 5.0, line_y),
        ],
        (1.5, min_color),
    );
}
