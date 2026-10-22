//! 自定义标题栏：文档标签（点击打开工程选择 popup / 拖动排序 / 拖出新建窗口）、
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
mod popup;

use eframe::egui;
use rust_i18n::t;

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
    /// 工程选择 popup：打开最近文件（追加为新文档）
    OpenRecent(String),
    /// 工程选择 popup：从最近文件列表移除记录
    RemoveRecent(String),
    /// 工程选择 popup：在新窗口（新进程）打开最近文件
    OpenRecentInNewWindow(String),
    /// 工程选择 popup：打开文件选择器（.yin / .mid 等）
    OpenFile,
    /// 应用名「Yinhe」点击：打开欢迎窗口
    OpenWelcome,
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
    recent: &[String],
    recent_times: &std::collections::HashMap<String, i64>,
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
            // macOS 原生全屏（新桌面）时系统红绿灯隐藏，收回为它预留的空间
            let fullscreen = ui.input(|i| i.viewport().fullscreen).unwrap_or(false);
            let m = BarMetrics::new(ui.max_rect(), fullscreen);
            let painter = ui.painter().clone();
            let n_docs = documents.len();

            // ── Precompute tab widths / rects for hit-testing and reorder ──
            // 标签宽度动态（按工程名实测，最长 MAX_TAB_W）
            let tab_widths = paint::tab_widths(&painter, documents);
            let tab_rects = paint::tab_rects(&m, *tab_scroll_offset, &tab_widths);
            // 应用名「Yinhe」可点击矩形（打开欢迎窗口）
            let title_rect = paint::title_rect(&painter, &m);

            // ── Handle mouse wheel / trackpad scroll for tab overflow ──
            let pointer_in_bar = ui.input(|i| {
                i.pointer
                    .hover_pos()
                    .is_some_and(|p| m.bar_rect.contains(p))
            });
            // 状态栏讲解行：悬停标题栏面板本身（控件提示由各控件以高优先级覆盖）
            if pointer_in_bar {
                crate::widgets::hint::set_region(ui.ctx(), t!("hint.panel.title_bar"));
            }
            // 拖拽中不响应滚轮（避免插入位置跳动）
            let drag_id = ui.scope_id().with("title_bar_drag");
            let press_id = ui.scope_id().with("title_bar_press");
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
                *tab_scroll_offset = tab_scroll_offset.clamp(0.0, m.max_scroll_offset(&tab_widths));

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
            let blank_left = m.blank_left(*tab_scroll_offset, &tab_widths);
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

            // 标签上按下后移动超过阈值则进入拖拽
            if drag.is_none()
                && let (Some(p), Some(cur)) = (press.clone(), pointer_pos)
                && pointer_down
                && (cur - p.pos).length() > max_click_dist
                && let Some(from) = p.idx
            {
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
                            (*tab_scroll_offset + SPEED).min(m.max_scroll_offset(&tab_widths));
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
                &tab_rects,
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

            // ── 工程选择 popup 状态与点击标签后的开合 ──
            let mut doc_popup: popup::PopupState = ui
                .data_mut(|d| d.get_temp(popup::state_id()))
                .unwrap_or_default();
            let mut popup_toggled = false;

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
                    // 标签点击：打开/关闭工程选择 popup（切换文档由 popup 行选择）
                    if let Some(i) = p.idx {
                        doc_popup.open = !doc_popup.open;
                        doc_popup.anchor_x = tab_rects
                            .get(i)
                            .map_or(m.bar_rect.min.x + m.left_padding, |r| r.min.x);
                        popup_toggled = true;
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
                        // 应用名「Yinhe」点击：打开欢迎窗口（像歌曲标题一样可点）
                        if title_rect.contains(p.pos) && title_rect.contains(release) {
                            action = Some(TitleBarAction::OpenWelcome);
                        }
                    }
                }
            }

            // ── 工程选择 popup：拖拽/点击外部/Esc 关闭，渲染在最后（浮在最上层）──
            if drag.is_some() {
                doc_popup.open = false;
            }
            if doc_popup.open && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                doc_popup.open = false;
            }
            if doc_popup.open {
                let out = popup::show(
                    ui,
                    &m,
                    documents,
                    active_doc,
                    recent,
                    recent_times,
                    doc_popup.anchor_x,
                );
                if out.close {
                    doc_popup.open = false;
                }
                if let Some(a) = out.action {
                    action = Some(a);
                }
                // 点击 popup 外部关闭（本帧刚由标签点击切换时跳过）
                if doc_popup.open
                    && !popup_toggled
                    && ui.input(|i| i.pointer.any_pressed())
                    && let Some(pos) = pointer_pos
                    && !out.rect.contains(pos)
                    && !tab_rects.iter().any(|r| r.contains(pos))
                {
                    doc_popup.open = false;
                }
            }
            ui.data_mut(|d| d.insert_temp(popup::state_id(), doc_popup));

            // ── Paint window buttons (non-macOS, visual only) ──
            #[cfg(not(target_os = "macos"))]
            {
                let maximized = ui.input(|i| i.viewport().maximized.unwrap_or(false));
                buttons::paint_window_buttons(ui, &win_btn_rects, maximized);
            }

            // Double-click blank area to toggle maximize/restore
            const DOUBLE_CLICK_MS: f64 = 400.0;
            let dbl_id = ui.scope_id().with("title_bar_dbl_click");
            if drag.is_none()
                && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
                && let Some(pos) = pointer_pos
                && in_blank_area(pos)
                && !title_rect.contains(pos)
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

            // ── 悬停讲解行（手动指针追踪；高优先级覆盖面板提示）──
            if let Some(pos) = pointer_pos {
                if let Some(d) = drag.as_ref() {
                    crate::widgets::hint::set(
                        ui.ctx(),
                        if d.detached {
                            t!("hint.title_bar.detach")
                        } else {
                            t!("hint.title_bar.reorder")
                        },
                    );
                } else if m.bar_rect.contains(pos) {
                    // 标签溢出时可滚轮横向滚动
                    if m.max_scroll_offset(&tab_widths) > 0.0 {
                        crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.scroll"));
                    }
                    if tab_rects.iter().any(|r| r.contains(pos)) {
                        crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.tab"));
                    } else if title_rect.contains(pos) {
                        // 应用名可点击（打开欢迎窗口）：手型光标 + 讲解
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.app_title"));
                    } else if in_blank_area(pos) {
                        let last: f64 = ui.data_mut(|d| d.get_persisted(dbl_id)).unwrap_or(0.0);
                        let now = ui.input(|i| i.time);
                        let pending = last > 0.0 && now - last < DOUBLE_CLICK_MS / 1000.0;
                        crate::widgets::hint::set(
                            ui.ctx(),
                            if pending {
                                t!("hint.title_bar.double_click")
                            } else {
                                t!("hint.title_bar.drag_window")
                            },
                        );
                    }
                }
            }
            // 非 macOS 窗口按钮（关闭/最大化/最小化）
            #[cfg(not(target_os = "macos"))]
            if let Some(pos) = pointer_pos {
                if win_btn_rects.0.contains(pos) {
                    crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.win_close"));
                } else if win_btn_rects.1.contains(pos) {
                    crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.win_max"));
                } else if win_btn_rects.2.contains(pos) {
                    crate::widgets::hint::set(ui.ctx(), t!("hint.title_bar.win_min"));
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
    use egui_kittest::kittest::Queryable as _;

    /// 测试状态：active_doc / 滚动 / 最近一次 title bar 动作。
    #[derive(Default)]
    struct TbState {
        action: Option<&'static str>,
        active_doc: Option<usize>,
        scroll: f32,
    }

    /// macOS left_padding=80、标签宽度动态（短名 "test.mid" 取最小宽度 120）
    /// → 首个标签 rect [80..200]×[4..28]；窗口取够高，保证标题栏下方的 popup
    /// 落在视口内可被点击。
    fn make_harness<'a>(documents: &'a [Document], recent: &'a [String]) -> Harness<'a, TbState> {
        let mut first_frame = true;
        Harness::builder()
            .allow_missing_glyphs()
            .with_accessibility_check(false)
            .with_size(egui::vec2(1200.0, 600.0))
            .build_ui_state(
                move |ui, st| {
                    if std::mem::take(&mut first_frame) {
                        ui.ctx().add_font(egui_material_icons::font_insert());
                        return;
                    }
                    let times = std::collections::HashMap::new();
                    if let Some(a) = show(
                        ui,
                        documents,
                        &mut st.active_doc,
                        &mut st.scroll,
                        recent,
                        &times,
                    ) {
                        st.action = Some(match a {
                            TitleBarAction::CloseDocument(_) => "close",
                            TitleBarAction::ReorderTab { .. } => "reorder",
                            TitleBarAction::DetachTab(_) => "detach",
                            TitleBarAction::OpenRecent(_) => "open_recent",
                            TitleBarAction::RemoveRecent(_) => "remove_recent",
                            TitleBarAction::OpenRecentInNewWindow(_) => "recent_new_window",
                            TitleBarAction::OpenFile => "open_file",
                            TitleBarAction::OpenWelcome => "open_welcome",
                        });
                    }
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

    fn click_at(h: &mut Harness<'_, TbState>, pos: egui::Pos2, time: f64) {
        press_at(h, pos, time);
        release_at(h, pos, time + 0.05);
    }

    fn has_command(h: &Harness<'_, TbState>, cmd: &egui::ViewportCommand) -> bool {
        h.output()
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|o| o.commands.iter().any(|c| c == cmd))
    }

    fn popup_open(h: &Harness<'_, TbState>) -> bool {
        h.ctx
            .data(|d| d.get_temp::<popup::PopupState>(popup::state_id()))
            .is_some_and(|s| s.open)
    }

    /// 回归测试：标签上按住并拖动不得移动窗口（也不得误触排序/detach）。
    #[test]
    fn drag_on_tab_does_not_start_window_drag() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs, &[]);
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
        let mut h = make_harness(&docs, &[]);
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

    /// 单击标签打开工程选择 popup；再点一次关闭（不直接切换文档）。
    #[test]
    fn click_tab_toggles_popup() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs, &[]);
        h.state_mut().active_doc = Some(0);
        let start = egui::pos2(140.0, 16.0);
        click_at(&mut h, start, 2.0);
        assert!(popup_open(&h), "单击标签应打开 popup");
        assert_eq!(h.state().active_doc, Some(0), "标签点击不直接切换文档");
        click_at(&mut h, start, 2.2);
        assert!(!popup_open(&h), "再点一次标签应关闭 popup");
    }

    /// Esc 关闭 popup。
    #[test]
    fn escape_closes_popup() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs, &[]);
        let start = egui::pos2(140.0, 16.0);
        click_at(&mut h, start, 2.0);
        assert!(popup_open(&h));
        event_at(
            &mut h,
            2.2,
            egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            },
        );
        assert!(!popup_open(&h), "Esc 应关闭 popup");
    }

    /// popup「当前窗口」行：点击切换活跃文档并关闭 popup。
    #[test]
    fn click_doc_row_switches_active_doc() {
        let mut d0 = yinhe_test_helpers::make_test_document();
        d0.file_name = "alpha.mid".to_string();
        let mut d1 = yinhe_test_helpers::make_test_document();
        d1.file_name = "beta.mid".to_string();
        let docs = vec![d0, d1];
        let mut h = make_harness(&docs, &[]);
        h.state_mut().active_doc = Some(0);
        click_at(&mut h, egui::pos2(140.0, 16.0), 2.0);
        h.run();
        h.get_by_label_contains("beta.mid").click();
        h.run();
        assert_eq!(h.state().active_doc, Some(1), "点击行应切换活跃文档");
        assert!(!popup_open(&h), "选中行后 popup 应关闭");
    }

    /// popup「最近文件」行：点击返回 OpenRecent 动作。
    #[test]
    fn click_recent_row_emits_open_recent() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let recent = vec!["/tmp/song.mid".to_string()];
        let mut h = make_harness(&docs, &recent);
        click_at(&mut h, egui::pos2(140.0, 16.0), 2.0);
        h.run();
        h.get_by_label_contains("song.mid").click();
        h.run();
        assert_eq!(h.state().action, Some("open_recent"));
        assert!(!popup_open(&h), "触发动作后 popup 应关闭");
    }

    /// popup 底部「打开文件」按钮：返回 OpenFile 动作。
    #[test]
    fn click_open_file_button_emits_action() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs, &[]);
        click_at(&mut h, egui::pos2(140.0, 16.0), 2.0);
        h.run();
        let label = t!("title_bar.open_file").to_string();
        h.get_by_label_contains(&label).click();
        h.run();
        assert_eq!(h.state().action, Some("open_file"));
        assert!(!popup_open(&h));
    }

    /// 最近文件行悬停后出现的 × 图标：返回 RemoveRecent 动作。
    #[test]
    fn click_remove_recent_icon_emits_action() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let recent = vec!["/tmp/song.mid".to_string()];
        let mut h = make_harness(&docs, &recent);
        click_at(&mut h, egui::pos2(140.0, 16.0), 2.0);
        h.run();
        h.get_by_label_contains("song.mid").hover();
        h.run();
        let label = t!("hint.title_bar.remove_recent").to_string();
        h.get_by_label_contains(&label).click();
        h.run();
        assert_eq!(h.state().action, Some("remove_recent"));
        assert!(!popup_open(&h));
    }

    /// 最近文件行悬停后出现的 ↗ 图标：返回 OpenRecentInNewWindow 动作。
    #[test]
    fn click_open_recent_in_new_window_icon_emits_action() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let recent = vec!["/tmp/song.mid".to_string()];
        let mut h = make_harness(&docs, &recent);
        click_at(&mut h, egui::pos2(140.0, 16.0), 2.0);
        h.run();
        h.get_by_label_contains("song.mid").hover();
        h.run();
        let label = t!("hint.title_bar.open_new").to_string();
        h.get_by_label_contains(&label).click();
        h.run();
        assert_eq!(h.state().action, Some("recent_new_window"));
        assert!(!popup_open(&h));
    }

    /// 点击标题栏中央的应用名「Yinhe」：返回 OpenWelcome（打开欢迎窗口）。
    #[test]
    fn click_app_title_opens_welcome() {
        let docs = vec![yinhe_test_helpers::make_test_document()];
        let mut h = make_harness(&docs, &[]);
        // 「Yinhe」居中于标题栏：1200 宽窗口 → 中心 x = 600
        click_at(&mut h, egui::pos2(600.0, 16.0), 2.0);
        assert_eq!(h.state().action, Some("open_welcome"));
    }
}
