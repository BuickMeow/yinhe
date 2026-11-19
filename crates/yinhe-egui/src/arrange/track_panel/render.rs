use eframe::egui;
use rust_i18n::t;

use yinhe_types::ArRowLayout;

use egui_material_icons::icons::{ICON_ADD, ICON_KEYBOARD_ARROW_DOWN, ICON_KEYBOARD_ARROW_RIGHT};

use super::types::TrackAction;
use super::{BADGE_ICON_BOTTOM_INSET, LEVEL_METER_MARGIN, LEVEL_METER_W, badge, hover};

/// 详情/紧凑模式的文字字号，统一 clamp，避免散落魔法数字
pub fn detail_font(lh: f32) -> egui::FontId {
    egui::FontId::proportional((lh * 0.25).clamp(9.0, 13.0))
}
pub fn compact_font(lh: f32) -> egui::FontId {
    egui::FontId::proportional((lh * 0.45).clamp(8.0, 14.0))
}

/// 行背景：条纹 → 选中 → 悬停，三层叠加，复用 `hover::is_row_hovered` 的成熟实现
pub fn draw_row_background(
    ui: &egui::Ui,
    painter: &egui::Painter,
    row_rect: egui::Rect,
    row_idx: usize,
    is_selected: bool,
    stripe_even: egui::Color32,
) {
    if row_idx.is_multiple_of(2) {
        painter.rect_filled(row_rect, 0.0, stripe_even);
    }
    if is_selected {
        painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
    } else if super::hover::is_row_hovered(ui, row_rect) {
        painter.rect_filled(
            row_rect,
            0.0,
            crate::theme::hover_color(crate::theme::app_bg()),
        );
    }
}

/// 色带（窄条）绘制，高 14px，色值由调用方传入
pub fn draw_badge(
    painter: &egui::Painter,
    row_rect: egui::Rect,
    lh: f32,
    color32: egui::Color32,
) -> egui::Rect {
    let badge_w = 14.0_f32;
    let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, lh));
    painter.rect_filled(badge_rect, 0.0, color32);
    badge_rect
}

/// 可视范围与总行数计算，抽自 `show` 顶部，避免在 `show` 中散落 clamp 逻辑
pub fn visible_range(scroll_y: f32, panel_h: f32, lh: f32, total_rows: usize) -> (usize, usize) {
    let first = (scroll_y / lh).floor().max(0.0) as usize;
    let last = ((scroll_y + panel_h) / lh).ceil().max(0.0) as usize + 1;
    (first.min(total_rows), last.min(total_rows))
}

/// 根据 `row_layout` 计算每轨主行矩形，用于拖拽插入线
pub fn build_item_rects(
    row_layout: &ArRowLayout,
    panel_rect: egui::Rect,
    panel_w: f32,
    lh: f32,
    scroll_y: f32,
    num_tracks: usize,
) -> Vec<egui::Rect> {
    let mut rects = Vec::with_capacity(num_tracks);
    for idx in 0..num_tracks {
        let y = panel_rect.min.y + row_layout.track_y(idx, lh) - scroll_y;
        rects.push(egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, lh),
        ));
    }
    rects
}

/// 自动化子行（AM lane）的完整绘制与听感圆点交互。
///
/// 与普通轨行共用色带/条纹/按钮几何，差异只在文本内容与自动化表。
/// 返回 `(am_ms_dirty, actions)`。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_automation_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    row_rect: egui::Rect,
    row: usize,
    lh: f32,
    track: usize,
    sub: usize,
    lane: &yinhe_types::AutomationLane,
    track_info: &[yinhe_core::TrackInfo],
    track_colors: &[[f32; 4]],
    tracks: &[std::sync::Arc<yinhe_core::TrackData>],
    am_lane_selected: &std::collections::HashSet<(u16, yinhe_types::AutomationTarget)>,
    am_ms: &mut std::collections::HashMap<
        (u16, yinhe_types::AutomationTarget),
        yinhe_types::AmMsState,
    >,
    show_details: bool,
    hover_track: Option<usize>,
    pointer_pos: Option<egui::Pos2>,
    cursor_tick: Option<f64>,
    no_drag_rects: &mut Vec<egui::Rect>,
    lane_even: egui::Color32,
) -> (bool, Vec<super::types::TrackAction>) {
    let mut am_ms_dirty = false;
    let mut actions = Vec::new();
    let lane_key = (track_info[track].index, lane.target.clone());
    draw_row_background(
        ui,
        painter,
        row_rect,
        row,
        am_lane_selected.contains(&lane_key),
        lane_even,
    );
    let color = track_colors
        .get(track)
        .copied()
        .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
    let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));
    let depth = tracks.get(track).map(|t| t.folder_depth).unwrap_or(0);
    let indent = f32::from(depth) * 14.0;
    let content_rect = egui::Rect::from_min_size(
        row_rect.min + egui::vec2(indent, 0.0),
        egui::vec2((row_rect.width() - indent).max(80.0), row_rect.height()),
    );
    let badge_rect = draw_badge(painter, content_rect, lh, color32);

    // 完全复刻普通轨的字号与几何：详情模式两行（×0.25，y 0.30/0.70，均 primary），
    // 紧凑模式单行（×0.45 居中）；唯一差异是文本内容（自动化名 / 所属音轨）。
    let text_x = badge_rect.max.x + 6.0;
    let label = super::super::am_lanes::lane_label(&lane.target);
    let owner = track_info[track].name.clone();
    if show_details {
        let font = detail_font(lh);
        painter.text(
            egui::pos2(text_x, badge_rect.min.y + lh * 0.30),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            crate::theme::text_primary(),
        );
        painter.text(
            egui::pos2(text_x, badge_rect.min.y + lh * 0.70),
            egui::Align2::LEFT_CENTER,
            owner,
            font.clone(),
            crate::theme::text_primary(),
        );
    } else {
        // 紧凑模式：自动化名 + 所属音轨拼成单行（与普通轨单行对齐）。
        let font = compact_font(lh);
        painter.text(
            egui::pos2(text_x, badge_rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{} · {}", label, owner),
            font,
            crate::theme::text_primary(),
        );
    }

    // 听感圆点：右下角，亮 = 可听，灭 = 静音。
    // 与普通轨一致，紧凑模式（!show_details）不显示。
    if show_details {
        let key = (track_info[track].index, lane.target.clone());
        let st = am_ms.get(&key).copied().unwrap_or_default();
        let meter_right = row_rect.max.x - LEVEL_METER_MARGIN;
        let meter_left = meter_right - LEVEL_METER_W;
        // 自动化表：编辑/播放光标处该 lane 的值（基线依种类）。
        let meter_rect = egui::Rect::from_min_max(
            egui::pos2(meter_left, row_rect.min.y + LEVEL_METER_MARGIN),
            egui::pos2(meter_right, row_rect.max.y - LEVEL_METER_MARGIN),
        );
        crate::widgets::meter::automation_meter(
            painter,
            meter_rect,
            crate::arrange::am_lanes::lane_cursor_value(lane, cursor_tick),
            lane.target.bar_baseline(),
            color32,
        );
        let dot_rect = egui::Rect::from_center_size(
            egui::pos2(row_rect.max.x - 7.0, row_rect.max.y - 7.0),
            egui::vec2(
                crate::widgets::audibility::HIT,
                crate::widgets::audibility::HIT,
            ),
        );
        no_drag_rects.push(dot_rect);
        let resp = crate::widgets::audibility::dot(
            ui,
            dot_rect,
            !st.mute,
            egui::Id::unique(("am_audible", track, sub)),
        );
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.arrange.am_mute"));
        if resp.clicked() {
            let mut state = st;
            state.set_audibility(if st.mute {
                yinhe_types::Audibility::Normal
            } else {
                yinhe_types::Audibility::Muted
            });
            if state == yinhe_types::AmMsState::default() {
                am_ms.remove(&key);
            } else {
                am_ms.insert(key, state);
            }
            am_ms_dirty = true;
        }
    }

    // 最后一条自动化行：在该行右侧（圆点旁）显示加号，点击再次添加自动化。
    let is_last_lane = tracks
        .get(track)
        .is_some_and(|t| sub + 1 == t.automation_lanes.len());
    if is_last_lane && hover_track == Some(track) {
        let plus_color = hover::icon_contrast_color(color);
        let plus_center = egui::pos2(
            badge_rect.center().x,
            badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
        );
        let plus_rect = egui::Rect::from_center_size(plus_center, egui::vec2(12.0, 16.0));
        no_drag_rects.push(plus_rect);
        if pointer_pos.is_some_and(|p| plus_rect.contains(p)) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.add_automation"));
        }
        // 加号放在色带列底部（与主行 chevron 同位置：底边距行底 1px）。
        // 点击加号：直接打开「添加自动化」窗口（设备维度，不再弹菜单）。
        if badge::badge_icon_button(
            ui,
            plus_center,
            ICON_ADD.codepoint,
            ICON_ADD.font_family(),
            plus_color,
            track,
        ) {
            actions.push(TrackAction::OpenAutomationPicker { idx: track });
        }
    }

    (am_ms_dirty, actions)
}

/// 普通轨行（含 Conductor / 文件夹 / 音频轨）的文本、听感圆点与电平表。
///
/// 色带与 chevron 由调用方绘制（依赖行布局与展开状态）；本函数只处理
/// 行内内容，返回 `(audio_dirty, actions)`。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_track_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    row_rect: egui::Rect,
    badge_rect: egui::Rect,
    lh: f32,
    idx: usize,
    ti: &yinhe_core::TrackInfo,
    tracks: &[std::sync::Arc<yinhe_core::TrackData>],
    is_conductor: bool,
    is_folder: bool,
    show_details: bool,
    track_overrides: &mut [yinhe_editor_core::document::TrackOverride],
    track_meters: &[crate::widgets::meter::StereoMeter],
    tempo_lane: &yinhe_types::AutomationLane,
    cursor_tick: Option<f64>,
    color32: egui::Color32,
    no_drag_rects: &mut Vec<egui::Rect>,
) -> (bool, Vec<super::types::TrackAction>) {
    let mut audio_dirty = false;
    let mut actions = Vec::new();
    let text_x = badge_rect.max.x + 6.0;
    let track_num_text = format!("{:03}", ti.index);

    if show_details {
        let font = detail_font(lh);

        painter.text(
            egui::pos2(text_x, badge_rect.min.y + lh * 0.30),
            egui::Align2::LEFT_CENTER,
            &track_num_text,
            font.clone(),
            crate::theme::text_primary(),
        );
        let badge_text = if is_conductor {
            "Master".to_string()
        } else {
            match tracks.get(idx) {
                Some(t) if t.kind == yinhe_core::TrackKind::Folder => "Folder".to_string(),
                // 音频轨显示音频通道。
                Some(t) if t.kind == yinhe_core::TrackKind::Audio => t
                    .audio_channel
                    .map(crate::mix::audio_label)
                    .unwrap_or_else(|| "Audio--".to_string()),
                // MIDI 轨：OwnLc 无徽标（一轨一通道，普通 DAW 轨道），
                // AP 显示端口+通道。
                Some(t) => {
                    crate::mix::track_target_label(t.effective_audio_target(), ti.port, ti.channel)
                }
                None => crate::mix::midi_label(ti.port, ti.channel),
            }
        };
        painter.text(
            egui::pos2(text_x + 32.0, badge_rect.min.y + lh * 0.30),
            egui::Align2::LEFT_CENTER,
            &badge_text,
            font.clone(),
            crate::theme::text_primary(),
        );

        let name = &ti.name;
        let name_font = detail_font(lh);
        let name_y = badge_rect.min.y + lh * 0.70;
        if crate::widgets::track_rename::active(ui, idx) {
            let name_rect = egui::Rect::from_min_size(
                egui::pos2(text_x, name_y - name_font.size * 0.75),
                egui::vec2(
                    (row_rect.max.x - text_x - 8.0).max(60.0),
                    name_font.size * 1.5,
                ),
            );
            if let Some(new_name) =
                crate::widgets::track_rename::show(ui, idx, name_rect, name_font.size)
            {
                actions.push(TrackAction::RenameTrack {
                    idx,
                    name: new_name,
                });
            }
        } else {
            painter.text(
                egui::pos2(text_x, name_y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                crate::theme::text_primary(),
            );
        }

        if !is_conductor && !is_folder {
            // 听感圆点：右下角，亮 = 可听，灭 = 静音。
            let audibility = track_overrides
                .get(idx)
                .map(|o| o.audibility())
                .unwrap_or_default();

            let meter_right = row_rect.max.x - LEVEL_METER_MARGIN;
            let meter_left = meter_right - LEVEL_METER_W;

            // 每轨电平表（L/R 双条、向上、dB 刻度；仅播放时非零）。
            let meter_rect = egui::Rect::from_min_max(
                egui::pos2(meter_left, row_rect.min.y + LEVEL_METER_MARGIN),
                egui::pos2(meter_right, row_rect.max.y - LEVEL_METER_MARGIN),
            );
            let (level, hold) = track_meters
                .get(idx)
                .copied()
                .unwrap_or(((0.0, 0.0), (0.0, 0.0)));
            crate::widgets::meter::compact_meter(painter, meter_rect, level, hold);

            let dot_rect = egui::Rect::from_center_size(
                egui::pos2(row_rect.max.x - 7.0, row_rect.max.y - 7.0),
                egui::vec2(
                    crate::widgets::audibility::HIT,
                    crate::widgets::audibility::HIT,
                ),
            );
            no_drag_rects.push(dot_rect);
            let resp = crate::widgets::audibility::dot(
                ui,
                dot_rect,
                audibility != yinhe_types::Audibility::Muted,
                egui::Id::unique(("track_audible", idx)),
            );
            crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.arrange.track_mute"));
            if resp.clicked() {
                super::ensure_audibility_undo_snapshot(ui, track_overrides);
                if let Some(ov) = track_overrides.get_mut(idx) {
                    ov.set_audibility(if audibility == yinhe_types::Audibility::Muted {
                        yinhe_types::Audibility::Normal
                    } else {
                        yinhe_types::Audibility::Muted
                    });
                    audio_dirty = true;
                }
            }
        } else if is_conductor {
            // Conductor 行：Tempo 自动化表（贴右缘；光标处 BPM 按动态上限归一化）。
            let meter_right = row_rect.max.x - LEVEL_METER_MARGIN;
            let meter_left = meter_right - LEVEL_METER_W;
            let meter_rect = egui::Rect::from_min_max(
                egui::pos2(meter_left, row_rect.min.y + LEVEL_METER_MARGIN),
                egui::pos2(meter_right, row_rect.max.y - LEVEL_METER_MARGIN),
            );
            crate::widgets::meter::automation_meter(
                painter,
                meter_rect,
                crate::arrange::am_lanes::lane_cursor_value(tempo_lane, cursor_tick),
                tempo_lane.target.bar_baseline(),
                color32,
            );
        }
    } else {
        let font = compact_font(lh);
        painter.text(
            egui::pos2(text_x, badge_rect.center().y),
            egui::Align2::LEFT_CENTER,
            &track_num_text,
            font,
            crate::theme::text_primary(),
        );

        let name = &ti.name;
        let name_font = compact_font(lh);
        if crate::widgets::track_rename::active(ui, idx) {
            let name_rect = egui::Rect::from_min_size(
                egui::pos2(text_x + 40.0, badge_rect.center().y - name_font.size * 0.75),
                egui::vec2(
                    (row_rect.max.x - text_x - 48.0).max(60.0),
                    name_font.size * 1.5,
                ),
            );
            if let Some(new_name) =
                crate::widgets::track_rename::show(ui, idx, name_rect, name_font.size)
            {
                actions.push(TrackAction::RenameTrack {
                    idx,
                    name: new_name,
                });
            }
        } else {
            painter.text(
                egui::pos2(text_x + 40.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                crate::theme::text_primary(),
            );
        }
    }
    (audio_dirty, actions)
}

/// 末尾「添加音轨」占位行：整行可点，点击打开新建音轨。
pub(super) fn draw_add_track_row(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    row_rect: egui::Rect,
    row: usize,
) {
    let color = crate::theme::stripe_bg();
    draw_row_background(ui, painter, row_rect, row, false, color);
    let add_id = egui::Id::unique("arr_add_track_row");
    let add_resp = ui.interact(row_rect, add_id, egui::Sense::click());
    crate::widgets::hint::hover(ui.ctx(), &add_resp, t!("hint.arrange.add_track"));
    let plus_center = egui::pos2(row_rect.min.x + 14.0, row_rect.center().y);
    let icon_color = if add_resp.hovered() {
        crate::theme::text_primary()
    } else {
        crate::theme::text_muted()
    };
    painter.text(
        plus_center,
        egui::Align2::CENTER_CENTER,
        ICON_ADD.codepoint,
        egui::FontId::new(crate::theme::ICON_FONT, ICON_ADD.font_family()),
        icon_color,
    );
    painter.text(
        egui::pos2(plus_center.x + 14.0, row_rect.center().y),
        egui::Align2::LEFT_CENTER,
        t!("hint.arrange.add_track"),
        egui::FontId::proportional(crate::theme::SMALL_FONT),
        icon_color,
    );
    if add_resp.clicked() {
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::unique(crate::dialogs::new_track::OPEN_REQUEST_ID),
                true,
            )
        });
    }
}

/// 色带底部图标：文件夹折叠箭头 / 自动化 chevron / 无条件时的「+」。
#[allow(clippy::too_many_arguments)] // 行上下文透传
pub(super) fn draw_badge_icons(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    badge_rect: egui::Rect,
    lh: f32,
    idx: usize,
    color: [f32; 4],
    is_folder: bool,
    collapsed: bool,
    expanded: bool,
    has_any_lane: bool,
    family_hovered: bool,
    pointer_pos: Option<egui::Pos2>,
    no_drag_rects: &mut Vec<egui::Rect>,
    actions: &mut Vec<TrackAction>,
    arr_am_expanded: &mut [bool],
) {
    if is_folder {
        let icon = if collapsed {
            ICON_KEYBOARD_ARROW_RIGHT
        } else {
            ICON_KEYBOARD_ARROW_DOWN
        };
        let icon_color = hover::icon_contrast_color(color);
        // 文件夹图标放色带中上部，折叠箭头固定在底部（与普通轨 chevron 同位）。
        if lh >= 28.0 {
            painter.text(
                egui::pos2(badge_rect.center().x, badge_rect.center().y - 4.0),
                egui::Align2::CENTER_CENTER,
                egui_material_icons::icons::ICON_FOLDER_OPEN.codepoint,
                egui::FontId::new(
                    crate::theme::ICON_FONT_SM,
                    egui_material_icons::icons::ICON_FOLDER_OPEN.font_family(),
                ),
                icon_color,
            );
        }
        let toggle_rect = egui::Rect::from_center_size(
            egui::pos2(
                badge_rect.center().x,
                badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
            ),
            egui::vec2(12.0, lh.min(16.0)),
        );
        let resp = ui.interact(
            toggle_rect,
            egui::Id::unique(("folder_toggle", idx)),
            egui::Sense::click(),
        );
        no_drag_rects.push(toggle_rect);
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.arrange.folder_toggle"));
        painter.text(
            toggle_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
            icon_color,
        );
        if resp.clicked() {
            actions.push(TrackAction::ToggleFolder { idx });
        }
    } else {
        // 未展开且完全没有任何自动化 → 直接在色带上给「+」，点击弹创建自动化菜单
        //（无需先展开）；否则照常显示展开/收起 chevron。
        let icon_color = hover::icon_contrast_color(color);
        if !expanded && !has_any_lane {
            // 无自动化未展开：色带同一图标位给无边框「+」，点击直接打开
            // 「添加自动化」窗口（设备维度）；仅悬浮显示。
            let plus_center = egui::pos2(
                badge_rect.center().x,
                badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
            );
            let plus_rect = egui::Rect::from_center_size(plus_center, egui::vec2(12.0, 16.0));
            if family_hovered && pointer_pos.is_some_and(|p| plus_rect.contains(p)) {
                crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.add_automation"));
            }
            if family_hovered {
                no_drag_rects.push(plus_rect);
                if badge::badge_icon_button(
                    ui,
                    plus_center,
                    ICON_ADD.codepoint,
                    ICON_ADD.font_family(),
                    icon_color,
                    idx,
                ) {
                    actions.push(TrackAction::OpenAutomationPicker { idx });
                }
            }
        } else {
            let icon = if expanded {
                ICON_KEYBOARD_ARROW_DOWN
            } else {
                ICON_KEYBOARD_ARROW_RIGHT
            };
            // chevron 仅在该轨家族被悬浮时显示；颜色按音轨颜色亮度选黑/白保证对比度。
            let icon_rect = egui::Rect::from_center_size(
                egui::pos2(
                    badge_rect.center().x,
                    badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
                ),
                egui::vec2(12.0, lh.min(16.0)),
            );
            let chev_resp = ui.interact(
                icon_rect,
                egui::Id::unique(("am_chevron", idx)),
                egui::Sense::click(),
            );
            crate::widgets::hint::hover(ui.ctx(), &chev_resp, t!("hint.arrange.am_expand"));
            if family_hovered {
                no_drag_rects.push(icon_rect);
                painter.text(
                    icon_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon.codepoint,
                    egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
                    icon_color,
                );
            } else if has_any_lane {
                // Bitwig 风格占位：不显示 chevron 时用淡点提示该轨有自动化。
                painter.circle_filled(icon_rect.center(), 2.0, icon_color.gamma_multiply(0.3));
            }
            if chev_resp.clicked()
                && let Some(e) = arr_am_expanded.get_mut(idx)
            {
                *e = !*e;
            }
        }
    }
}
