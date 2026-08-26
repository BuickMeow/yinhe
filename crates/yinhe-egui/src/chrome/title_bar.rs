//! 自定义标题栏：文档标签（点击切换 / 拖动排序 / 拖出新建窗口）、
//! 窗口拖动、双击最大化、非 macOS 窗口按钮。
//!
//! 交互全部用手动指针追踪（TitlePress / TitleDrag 跨帧状态），不注册
//! egui interact widget——ui.interact 的 hit-test 兜底会在标签附近把
//! 拖拽归属抢走，导致误触发窗口移动或吞掉点击（transport_bar 同款问题）。

/// 非 macOS 平台的窗口控制按钮（关闭/最大化/最小化）绘制模块；
/// macOS 用系统红绿灯，整个模块不参与编译。
#[cfg(not(target_os = "macos"))]
mod buttons;
mod paint;

use eframe::egui;

use paint::BarMetrics;
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
    /// 空白区按下且已发送 StartDrag（每按只发一次，防系统拖拽中重发）。
    win_drag_sent: bool,
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
            let m = BarMetrics::new(ui.max_rect());
            let painter = ui.painter().clone();
            let n_docs = documents.len();

            // ── Precompute tab/close rects for hit-testing and reorder ──
            let tab_rects: Vec<egui::Rect> = (0..n_docs)
                .map(|i| m.tab_rect(*tab_scroll_offset, i))
                .collect();
            let close_rects: Vec<egui::Rect> = (0..n_docs)
                .map(|i| m.close_rect(*tab_scroll_offset, i))
                .collect();

            // ── Handle mouse wheel / trackpad scroll for tab overflow ──
            let pointer_in_bar = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .is_some_and(|p| m.bar_rect.contains(p))
            });
            // 状态栏讲解行：鼠标在标题栏上时清空（标题栏不属于任何可讲解区域）
            if pointer_in_bar {
                *status_hint = None;
            }
            // 拖拽中不响应滚轮（避免插入位置跳动）
            let drag_id = ui.id().with("title_bar_drag");
            let press_id = ui.id().with("title_bar_press");
            let mut drag: Option<TitleDrag> = ui.data_mut(|d| d.get_temp(drag_id));
            let mut press: Option<TitlePress> = ui.data_mut(|d| d.get_temp(press_id));
            if pointer_in_bar && drag.is_none() {
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
                *tab_scroll_offset = tab_scroll_offset.clamp(0.0, m.max_scroll_offset(n_docs));

                if scroll_delta != egui::Vec2::ZERO || (zoom_delta - 1.0).abs() > 0.001 {
                    ui.ctx().request_repaint();
                }
            }

            // ── Press / drag detection ──
            let pointer_pos = ui
                .input(|i| i.pointer.interact_pos())
                .or_else(|| ui.input(|i| i.pointer.hover_pos()));
            let pointer_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
            let button_pressed =
                ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
            let button_released =
                ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
            let any_released = ui.input(|i| i.pointer.any_released());

            // 点击/拖拽分界线（与 egui click 判定同阈值，transport bar 同款）
            let max_click_dist = egui::InputOptions::default().max_click_dist;

            // 空白区（标签右侧、窗口按钮左侧）：窗口拖动 + 双击最大化区域
            let blank_left = m.blank_left(*tab_scroll_offset, n_docs);
            let blank_right = if cfg!(target_os = "macos") {
                m.bar_rect.max.x
            } else {
                m.bar_rect.max.x - 138.0
            };
            let in_blank_area = |pos: egui::Pos2| {
                pos.x >= blank_left && pos.x <= blank_right && m.bar_rect.y_range().contains(pos.y)
            };

            // 记录按下起点（标题栏任意位置；标签上额外记下索引）
            if drag.is_none()
                && button_pressed
                && let Some(pos) = pointer_pos
                && m.bar_rect.contains(pos)
            {
                let idx = tab_rects.iter().position(|r| r.contains(pos));
                press = Some(TitlePress {
                    idx,
                    pos,
                    win_drag_sent: false,
                });
            }

            // 标签上按下后移动超过阈值则进入拖拽（关闭按钮区域不触发）
            if drag.is_none()
                && let (Some(p), Some(cur)) = (press.clone(), pointer_pos)
                && pointer_down
                && (cur - p.pos).length() > max_click_dist
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

            // 空白区按下后移动超过点击阈值 → 启动系统窗口拖动（每按只发一次）。
            // 手动指针追踪，不用 ui.interact(Sense::drag)：interact 的 hit-test
            // 兜底（find_closest / interact_radius）会在标签附近把拖拽归属抢走，
            // 导致在标签上按下拖动误移动窗口（transport_bar 同款问题的同款解法）。
            if let Some(p) = press.as_mut()
                && p.idx.is_none()
                && !p.win_drag_sent
                && pointer_down
                && in_blank_area(p.pos)
                && let Some(cur) = pointer_pos
                && (cur - p.pos).length() >= max_click_dist
            {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
                p.win_drag_sent = true;
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
                d.detached = cur.y < m.bar_rect.min.y - 18.0 || cur.y > m.bar_rect.max.y + 30.0;
                if !d.detached {
                    // 边缘自动滚动（仅标签溢出时有效）
                    const MARGIN: f32 = 40.0;
                    const SPEED: f32 = 20.0;
                    if cur.x < m.bar_rect.min.x + m.left_padding + MARGIN {
                        *tab_scroll_offset = (*tab_scroll_offset - SPEED).max(0.0);
                    } else if cur.x > m.bar_rect.max.x - MARGIN {
                        *tab_scroll_offset =
                            (*tab_scroll_offset + SPEED).min(m.max_scroll_offset(n_docs));
                    }
                }
                ui.ctx().request_repaint();
            }

            // ── Paint ──
            paint::paint_tabs(
                &painter,
                ui,
                &m,
                documents,
                *active_doc,
                *tab_scroll_offset,
                drag.as_ref(),
            );
            if let Some(d) = drag.as_ref() {
                paint::paint_drag_overlay(&painter, ui, &m, documents, d, &tab_rects);
            }

            // ── Window button rects (non-macOS) ──
            #[cfg(not(target_os = "macos"))]
            let win_btn_rects = {
                let btn_w = 46.0;
                let btn_h = TITLE_BAR_HEIGHT;
                let btn_y = m.bar_rect.min.y;

                let c = egui::Rect::from_min_size(
                    egui::pos2(m.bar_rect.max.x - btn_w, btn_y),
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
                        let order =
                            crate::widgets::reorder::plan_order(n_docs, &[d.from], d.insert_idx);
                        let cur_order: Vec<usize> = (0..n_docs).collect();
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
                    && (release - p.pos).length() < max_click_dist
                {
                    // 标签 / 关闭按钮点击（按下与松开都在同一矩形内）
                    if let Some(i) = p.idx {
                        if close_rects[i].contains(p.pos) && close_rects[i].contains(release) {
                            action = Some(TitleBarAction::CloseDocument(i));
                        } else if tab_rects[i].contains(p.pos) && tab_rects[i].contains(release) {
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
                buttons::paint_window_buttons(ui, &win_btn_rects, maximized);
            }

            // Double-click blank area to toggle maximize/restore
            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.id().with("title_bar_dbl_click");
            if drag.is_none()
                && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = pointer_pos
                && in_blank_area(pos)
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
                if let Some(dg) = drag {
                    d.insert_temp(drag_id, dg);
                } else {
                    d.remove::<TitleDrag>(drag_id);
                }
                if let Some(p) = press {
                    d.insert_temp(press_id, p);
                } else {
                    d.remove::<TitlePress>(press_id);
                }
            });

            // Reserve space for title bar height
            ui.allocate_space(egui::vec2(0.0, TITLE_BAR_HEIGHT));
        });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    /// 测试状态：active_doc / 滚动 / 最近一次 title bar 动作。
    #[derive(Default)]
    struct TbState {
        action: Option<&'static str>,
        active_doc: Option<usize>,
        scroll: f32,
    }

    /// macOS left_padding=80、tab_w=120 → 首个标签 rect [80..200]×[4..28]。
    fn make_harness<'a>(documents: &'a [Document]) -> Harness<'a, TbState> {
        let mut first_frame = true;
        Harness::builder()
            .with_size(egui::vec2(1200.0, 32.0))
            .build_ui_state(
                move |ui, st| {
                    if std::mem::take(&mut first_frame) {
                        ui.ctx().add_font(egui_material_icons::font_insert());
                        return;
                    }
                    let mut hint = None;
                    st.action =
                        match show(ui, documents, &mut st.active_doc, &mut st.scroll, &mut hint) {
                            Some(TitleBarAction::CloseDocument(_)) => Some("close"),
                            Some(TitleBarAction::ReorderTab { .. }) => Some("reorder"),
                            Some(TitleBarAction::DetachTab(_)) => Some("detach"),
                            None => None,
                        };
                },
                TbState::default(),
            )
    }

    fn event_at(h: &mut Harness<'_, TbState>, time: f64, event: egui::Event) {
        h.input_mut().time = Some(time);
        h.event(event);
        h.step();
    }

    fn press_at(h: &mut Harness<'_, TbState>, pos: egui::Pos2, time: f64) {
        event_at(h, time, egui::Event::PointerMoved(pos));
        event_at(
            h,
            time + 0.001,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn release_at(h: &mut Harness<'_, TbState>, pos: egui::Pos2, time: f64) {
        event_at(
            h,
            time,
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            },
        );
    }

    fn has_command(h: &Harness<'_, TbState>, cmd: &egui::ViewportCommand) -> bool {
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
    }

    /// 回归测试：标签上按住并拖动不得移动窗口（也不得误触排序/detach）。
    #[test]
    fn drag_on_tab_does_not_start_window_drag() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs);
        let start = egui::pos2(140.0, 16.0); // 第一个标签中心
        press_at(&mut h, start, 2.0);
        for i in 1..=5 {
            event_at(
                &mut h,
                2.0 + i as f64 * 0.05,
                egui::Event::PointerMoved(start + egui::vec2(10.0 * i as f32, 3.0 * i as f32)),
            );
            assert!(
                !has_command(&h, &egui::ViewportCommand::StartDrag),
                "标签上拖动不应发送 StartDrag（帧 {i}）"
            );
        }
        release_at(&mut h, start + egui::vec2(50.0, 15.0), 2.4);
    }

    /// 空白区按住拖动仍应启动窗口拖动。
    #[test]
    fn drag_blank_area_starts_window_drag() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs);
        let start = egui::pos2(600.0, 16.0);
        press_at(&mut h, start, 2.0);
        event_at(
            &mut h,
            2.1,
            egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)),
        );
        assert!(
            has_command(&h, &egui::ViewportCommand::StartDrag),
            "空白区拖动应发送 StartDrag"
        );
        release_at(&mut h, start + egui::vec2(10.0, 0.0), 2.15);
    }

    /// 单击标签切换活跃文档；拖拽阈值内的小幅移动不产生排序动作。
    #[test]
    fn click_tab_switches_active_doc() {
        let docs = vec![
            yinhe_test_helpers::make_test_document(),
            yinhe_test_helpers::make_test_document(),
        ];
        let mut h = make_harness(&docs);
        h.state_mut().active_doc = Some(0);
        let start = egui::pos2(262.0, 16.0); // 第二个标签中心（80+122+60）
        press_at(&mut h, start, 2.0);
        release_at(&mut h, start, 2.05);
        assert_eq!(h.state().active_doc, Some(1), "单击第二个标签应切换活跃");
        assert_eq!(h.state().action, None);
    }

    /// 点击关闭按钮应返回 CloseDocument 动作。
    #[test]
    fn click_close_button_emits_close_action() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs);
        // 关闭按钮在标签右缘内 20px：rect [180..200]×[4..28]
        let start = egui::pos2(190.0, 16.0);
        press_at(&mut h, start, 2.0);
        release_at(&mut h, start, 2.05);
        assert_eq!(
            h.state().action,
            Some("close"),
            "点关闭按钮应触发 CloseDocument"
        );
    }
}
