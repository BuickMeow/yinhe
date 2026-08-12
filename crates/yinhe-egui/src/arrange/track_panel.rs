use std::collections::HashSet;

use eframe::egui;
use egui_material_icons::icons::ICON_EDIT;
use rust_i18n::t;

use yinhe_core::TrackInfo;

use yinhe_editor_core::document::TrackOverride;

/// Actions requested by the track panel that need Document access.
#[derive(Clone, Debug)]
pub(crate) enum TrackAction {
    /// Add a new track after the given index (or at end if None)
    AddTrack { after_idx: Option<usize> },
    /// Remove the track at the given index
    RemoveTrack { idx: usize },
    /// Move a track up (swap with previous)
    MoveUp { idx: usize },
    /// Move a track down (swap with next)
    MoveDown { idx: usize },
    /// 拖拽排序：把 `indices`（升序，保持相对顺序）整体移动到
    /// 删除它们后的列表中的 `insert_at` 位置。
    MoveTracks {
        indices: Vec<usize>,
        insert_at: usize,
    },
}

/// Render the track list using a painter (unified component for both
/// pianoroll and transport contexts).
///
/// Returns `(audio_dirty, actions)` where `audio_dirty` is `true` if the user
/// toggled a Mute or Solo button this frame, and `actions` is a list of
/// track-management actions (add/remove/move) for the caller to apply.
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
#[must_use]
pub(crate) fn show(
    ui: &mut egui::Ui,
    track_info: &[TrackInfo],
    track_visible: &[bool],
    track_overrides: &mut [TrackOverride],
    track_selected: &mut HashSet<u16>,
    selection_anchor: &mut Option<u16>,
    conductor_track_idx: Option<u16>,
    track_colors: &[[f32; 4]],
    row_height: &mut f32,
    scroll_y: &mut f32,
    request_pianoroll: &mut bool,
    editing_track: &mut Option<u16>,
    info_content: &mut Option<crate::right_panel::InfoContent>,
) -> (bool, Vec<TrackAction>) {
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();
    let num_tracks = track_info.len();

    if num_tracks == 0 || panel_w < 1.0 || panel_h < 1.0 {
        return (false, Vec::new());
    }

    let mut actions = Vec::new();

    let show_details = *row_height >= 30.0;

    // Clamp scroll_y
    let max_scroll = (num_tracks as f32 * *row_height - panel_h).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // ── 拖拽排序跨帧状态（算法见 widgets::reorder） ──
    let drag_id = ui.id().with("track_panel_drag");
    let mut drag: Option<crate::widgets::reorder::DragReorder> =
        ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or_default();
    let dragging = drag.is_some();

    // Visible track range
    let first = (*scroll_y / *row_height).floor() as usize;
    let visible_count = (panel_h / *row_height).ceil() as usize + 2;
    let last = (first + visible_count).min(num_tracks);

    let painter = ui.painter().clone();
    let mut audio_dirty = false;

    // 交替行条纹：着色行（偶数行）与 GPU 区同源颜色，不透明；奇数行用 app_bg 打底
    let lane_even = crate::theme::stripe_bg();

    let interact_id = egui::Id::new("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());

    let btn_size = egui::vec2(18.0, 18.0);

    // 全部行的矩形（含视口外/隐藏行，保证拖拽插入索引全局正确）；仅可视行渲染。
    let mut item_rects: Vec<egui::Rect> = Vec::with_capacity(num_tracks);

    for (idx, ti) in track_info.iter().enumerate() {
        let y = panel_rect.min.y + idx as f32 * *row_height - *scroll_y;
        let row_rect = egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, *row_height),
        );
        item_rects.push(row_rect);
        if idx < first || idx >= last {
            continue;
        }
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }
        if y > panel_rect.max.y || y + *row_height < panel_rect.min.y {
            continue;
        }

        let is_conductor = Some(ti.index) == conductor_track_idx;
        let selected = track_selected.contains(&ti.index);
        // 着色行条纹（奇数行 = app_bg 普通行，不画；选中/悬停 tint 在条纹之上）
        if idx % 2 == 0 {
            painter.rect_filled(row_rect, 0.0, lane_even);
        }
        if selected {
            painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
        } else if row_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
            painter.rect_filled(
                row_rect,
                0.0,
                crate::theme::hover_color(crate::theme::app_bg()),
            );
        }

        let color = track_colors
            .get(idx)
            .copied()
            .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
        let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));

        let badge_w = 8.0_f32;
        let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, *row_height));
        painter.rect_filled(badge_rect, 0.0, color32);

        let text_x = badge_rect.max.x + 6.0;
        let track_num_text = format!("{:03}", ti.index);

        if show_details {
            // 详情模式行号/名称字号下限统一为 9（原行号误写 8）
            let font = egui::FontId::proportional((*row_height * 0.25).clamp(9.0, 13.0));

            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font.clone(),
                crate::theme::text_primary(),
            );
            let badge_text = if is_conductor {
                "Master".to_string()
            } else {
                let port_letter = match ti.port {
                    0 => 'A',
                    1 => 'B',
                    2 => 'C',
                    3 => 'D',
                    4 => 'E',
                    5 => 'F',
                    6 => 'G',
                    7 => 'H',
                    _ => '?',
                };
                format!("{}{:02}", port_letter, ti.channel + 1)
            };
            painter.text(
                egui::pos2(text_x + 32.0, badge_rect.min.y + *row_height * 0.30),
                egui::Align2::LEFT_CENTER,
                &badge_text,
                font.clone(),
                crate::theme::text_primary(),
            );

            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.25).clamp(9.0, 13.0));
            painter.text(
                egui::pos2(text_x, badge_rect.min.y + *row_height * 0.70),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                crate::theme::text_primary(),
            );

            if !is_conductor {
                let muted = track_overrides.get(idx).map(|o| o.muted).unwrap_or(false);
                let soloed = track_overrides.get(idx).map(|o| o.soloed).unwrap_or(false);

                let gap = 2.0;
                let total_btn_w = 2.0 * btn_size.x + gap;
                let btn_x_start = row_rect.max.x - total_btn_w - 6.0;
                let btn_y = badge_rect.center().y - btn_size.y * 0.5;

                let m_rect = egui::Rect::from_min_size(egui::pos2(btn_x_start, btn_y), btn_size);
                let s_rect = egui::Rect::from_min_size(
                    egui::pos2(btn_x_start + btn_size.x + gap, btn_y),
                    btn_size,
                );

                let m_resp = draw_inline_button(
                    ui,
                    &painter,
                    m_rect,
                    "M",
                    muted,
                    crate::theme::mute_active(),
                    egui::Id::new(("track_btn_m", idx)),
                );
                let s_resp = draw_inline_button(
                    ui,
                    &painter,
                    s_rect,
                    "S",
                    soloed,
                    crate::theme::solo_active(),
                    egui::Id::new(("track_btn_s", idx)),
                );

                if m_resp.clicked()
                    && let Some(ov) = track_overrides.get_mut(idx)
                {
                    ov.muted = !ov.muted;
                    audio_dirty = true;
                }
                if s_resp.clicked()
                    && let Some(ov) = track_overrides.get_mut(idx)
                {
                    ov.soloed = !ov.soloed;
                    audio_dirty = true;
                }
            }

            // 铅笔 ICON：双击 track 后显示，表示该 track 是 pencil/automation 的编辑目标。
            // 非 conductor：在 M/S 按钮左侧；conductor：不出铅笔图标
            // （Tempo 编辑不依赖编辑目标，conductor 仅作 PR 打开/定位用）。
            if *editing_track == Some(ti.index) && !is_conductor {
                let gap = 2.0;
                let total_btn_w = 2.0 * btn_size.x + gap;
                let icon_x = row_rect.max.x - total_btn_w - 6.0 - gap - btn_size.x;
                let icon_y = badge_rect.center().y - btn_size.y * 0.5;
                let icon_rect = egui::Rect::from_min_size(egui::pos2(icon_x, icon_y), btn_size);
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    ICON_EDIT.codepoint,
                    egui::FontId::new(crate::theme::ICON_FONT, ICON_EDIT.font_family()),
                    crate::theme::text_bright(),
                );
            }
        } else {
            let font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font,
                crate::theme::text_primary(),
            );

            let name = &ti.name;
            let name_font = egui::FontId::proportional((*row_height * 0.45).clamp(8.0, 14.0));
            painter.text(
                egui::pos2(text_x + 40.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                crate::theme::text_primary(),
            );
        }
    }

    // ── Click handling ──
    let hit = |pos: egui::Pos2| -> Option<usize> {
        let rel_y = pos.y - panel_rect.min.y + *scroll_y;
        let idx = (rel_y / *row_height).floor() as usize;
        if idx >= num_tracks { None } else { Some(idx) }
    };

    if resp.double_clicked() && !dragging {
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(idx) = hit(pos)
        {
            // 双击 toggle：已经是 editing_track 则清除（关闭编辑），
            // 否则设为新 editing_track（打开 PR 并切换编辑目标）。
            // Conductor 也可作为编辑目标（仅用于 Tempo automation，不出铅笔图标）。
            let track_idx = track_info[idx].index;
            if *editing_track == Some(track_idx) {
                *editing_track = None;
            } else {
                *editing_track = Some(track_idx);
                *request_pianoroll = true;
            }
        }
    } else if resp.clicked()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(idx) = hit(pos)
    {
        let track_idx = track_info[idx].index;
        let shift = ui.input(|i| i.modifiers.shift);
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

        if shift {
            // Range-select from anchor to this track.
            if let Some(anchor) = *selection_anchor {
                let a = anchor as usize;
                let b = track_idx as usize;
                let lo = a.min(b);
                let hi = a.max(b);
                for i in lo..=hi {
                    track_selected.insert(i as u16);
                }
            } else {
                track_selected.clear();
                track_selected.insert(track_idx);
                *selection_anchor = Some(track_idx);
            }
        } else if cmd {
            // Toggle this track.
            if track_selected.contains(&track_idx) {
                track_selected.remove(&track_idx);
            } else {
                track_selected.insert(track_idx);
            }
            *selection_anchor = Some(track_idx);
        } else {
            // Plain click: 如果点击的音轨已是唯一选中的，则取消选择；
            // 否则替换选择（清除旧选择，选中此音轨）。
            if track_selected.len() == 1 && track_selected.contains(&track_idx) {
                track_selected.clear();
            } else {
                track_selected.clear();
                track_selected.insert(track_idx);
            }
            *selection_anchor = Some(track_idx);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }

    // On secondary click, select the track under the cursor and record its
    // index in egui temp data so the context_menu closure (which may run on
    // subsequent frames while the menu stays open) can recover it.
    let ctx_menu_idx_id = egui::Id::new("track_ctx_menu_idx");
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(idx) = hit(pos)
    {
        let track_idx = track_info[idx].index;
        if !track_selected.contains(&track_idx) {
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
        ui.ctx().data_mut(|d| d.insert_temp(ctx_menu_idx_id, idx));
    }

    resp.context_menu(|ui| {
        ui.set_min_width(160.0);
        ui.set_max_width(160.0);
        let idx = ui
            .ctx()
            .data(|d| d.get_temp::<usize>(ctx_menu_idx_id))
            .unwrap_or(0);
        let track_idx = track_info.get(idx).map(|t| t.index).unwrap_or(0);
        let is_conductor = conductor_track_idx == Some(track_idx);

        if !is_conductor {
            if ui
                .add(
                    egui::Button::new(t!("arrange.add_below").as_ref())
                        .min_size(egui::vec2(ui.available_width(), 0.0))
                        .stroke(egui::Stroke::NONE),
                )
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx),
                });
                ui.close();
            }
            if ui
                .add(
                    egui::Button::new(t!("arrange.add_above").as_ref())
                        .min_size(egui::vec2(ui.available_width(), 0.0))
                        .stroke(egui::Stroke::NONE),
                )
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx.saturating_sub(1)),
                });
                ui.close();
            }
            ui.separator();
            if idx > 0
                && conductor_track_idx != Some((idx - 1) as u16)
                && ui
                    .add(
                        egui::Button::new(t!("arrange.move_up").as_ref())
                            .min_size(egui::vec2(ui.available_width(), 0.0))
                            .stroke(egui::Stroke::NONE),
                    )
                    .clicked()
            {
                actions.push(TrackAction::MoveUp { idx });
                ui.close();
            }
            if idx < num_tracks - 1
                && ui
                    .add(
                        egui::Button::new(t!("arrange.move_down").as_ref())
                            .min_size(egui::vec2(ui.available_width(), 0.0))
                            .stroke(egui::Stroke::NONE),
                    )
                    .clicked()
            {
                actions.push(TrackAction::MoveDown { idx });
                ui.close();
            }
            ui.separator();
            if ui
                .add(
                    egui::Button::new(t!("arrange.delete_track").as_ref())
                        .min_size(egui::vec2(ui.available_width(), 0.0))
                        .stroke(egui::Stroke::NONE),
                )
                .clicked()
            {
                actions.push(TrackAction::RemoveTrack { idx });
                ui.close();
            }
        } else {
            // Conductor track: only allow adding after
            if ui
                .add(
                    egui::Button::new(t!("arrange.add_below").as_ref())
                        .min_size(egui::vec2(ui.available_width(), 0.0))
                        .stroke(egui::Stroke::NONE),
                )
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx),
                });
                ui.close();
            }
        }
    });

    // ── 拖拽排序 ──
    // 拖拽开始：未选中的行先单选，然后拖起整个选中集合（排除 conductor）。
    if resp.drag_started()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(idx) = hit(pos)
        && Some(track_info[idx].index) != conductor_track_idx
    {
        let track_idx = track_info[idx].index;
        if !track_selected.contains(&track_idx) {
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
        }
        let mut indices: Vec<usize> = track_selected.iter().map(|&t| t as usize).collect();
        indices.sort_unstable();
        indices.retain(|&i| track_info.get(i).map(|t| Some(t.index)) != Some(conductor_track_idx));
        if !indices.is_empty() {
            drag = Some(crate::widgets::reorder::DragReorder {
                indices,
                insert_idx: idx,
            });
        }
    }

    // 拖拽进行中：插入位置 + 插入线 + 边缘自动滚动；释放时提交排序。
    if let Some(drag_state) = &mut drag {
        if let Some(p) = ui.input(|i| i.pointer.interact_pos()) {
            drag_state.update_insert_idx(p.y, &item_rects);
            // conductor 固定在最前（索引 0），被拖行不能插到它前面
            drag_state.insert_idx = drag_state.insert_idx.max(1);

            // 自动滚动：指针贴近面板上下边缘
            const AUTO_SCROLL_MARGIN: f32 = 20.0;
            const AUTO_SCROLL_SPEED: f32 = 32.0;
            if p.y < panel_rect.top() + AUTO_SCROLL_MARGIN {
                *scroll_y = (*scroll_y - AUTO_SCROLL_SPEED).max(0.0);
            } else if p.y > panel_rect.bottom() - AUTO_SCROLL_MARGIN {
                *scroll_y = (*scroll_y + AUTO_SCROLL_SPEED).min(max_scroll);
            }
        }

        if let Some(y) = drag_state.insert_line_y(&item_rects) {
            let x1 = panel_rect.min.x + 4.0;
            let x2 = panel_rect.max.x - 4.0;
            painter.line_segment(
                [egui::pos2(x1, y), egui::pos2(x2, y)],
                egui::Stroke::new(3.0, crate::theme::accent_active()),
            );
        }

        if ui.input(|i| i.pointer.any_released()) {
            actions.push(TrackAction::MoveTracks {
                indices: drag_state.indices.clone(),
                insert_at: drag_state.insert_idx,
            });
            drag = None;
        }
    }

    // ── Up/Down arrow key navigation ──
    if ui.input(|i| i.key_pressed(egui::Key::ArrowUp)) {
        if let Some(&current) = track_selected.iter().next() {
            let new_idx = current.saturating_sub(1);
            let mut found = None;
            for i in (0..=new_idx as usize).rev() {
                if track_visible.get(i).copied().unwrap_or(true) {
                    found = Some(i as u16);
                    break;
                }
            }
            if let Some(target) = found {
                track_selected.clear();
                track_selected.insert(target);
                *selection_anchor = Some(target);
            }
        } else if !track_info.is_empty() {
            let last = track_info.len() - 1;
            track_selected.clear();
            track_selected.insert(last as u16);
            *selection_anchor = Some(last as u16);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }
    if ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        if let Some(&current) = track_selected.iter().next() {
            let new_idx = (current as usize + 1).min(num_tracks - 1);
            let mut found = None;
            for i in new_idx..num_tracks {
                if track_visible.get(i).copied().unwrap_or(true) {
                    found = Some(i as u16);
                    break;
                }
            }
            if let Some(target) = found {
                track_selected.clear();
                track_selected.insert(target);
                *selection_anchor = Some(target);
            }
        } else if !track_info.is_empty() {
            track_selected.clear();
            track_selected.insert(0);
            *selection_anchor = Some(0);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }

    if resp.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            *scroll_y = (*scroll_y - scroll_delta.y).max(0.0);
        }
    }

    ui.data_mut(|d| d.insert_temp(drag_id, drag));

    (audio_dirty, actions)
}

/// Paint an 18x18 inline button with a one-letter label and click handling.
fn draw_inline_button(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    rect: egui::Rect,
    label: &str,
    active: bool,
    active_color: egui::Color32,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let hovered = resp.hovered();
    let pressed = resp.is_pointer_button_down_on();

    let (fill, text_col) = if active {
        let f = if pressed {
            crate::theme::pressed_color(active_color)
        } else if hovered {
            crate::theme::hover_color(active_color)
        } else {
            active_color
        };
        (f, egui::Color32::BLACK)
    } else {
        let base = crate::theme::btn_bg();
        let f = if pressed {
            crate::theme::pressed_color(base)
        } else if hovered {
            crate::theme::hover_color(base)
        } else {
            base
        };
        (f, crate::theme::text_secondary())
    };

    painter.rect_filled(rect, 3.0, fill);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        text_col,
    );

    resp
}
