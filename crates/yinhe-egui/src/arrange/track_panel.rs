use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD, ICON_KEYBOARD_ARROW_DOWN, ICON_KEYBOARD_ARROW_RIGHT};
use rust_i18n::t;

use yinhe_core::TrackInfo;
use yinhe_types::{ArRow, ArRowLayout, AutomationTarget};

use yinhe_editor_core::document::TrackOverride;

mod types;
pub(crate) use types::TrackAction;
use types::*;

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
    info_content: &mut Option<crate::right_panel::InfoContent>,
    // 行布局（含展开的 AM 子行；与 GPU 区共用同一 ArRowLayout）。
    row_layout: &ArRowLayout,
    // 音轨数据（AM 子行画 lane 名 / 右键菜单判断已有 lane）。
    tracks: &[Arc<yinhe_core::TrackData>],
    // 每轨 AM 展开状态（chevron 点击直接翻转，纯视图状态不进 undo）。
    arr_am_expanded: &mut [bool],
    // 被选中的 AM lane（(音轨索引, target)）；点主行清空。
    am_lane_selected: &mut HashSet<(u16, AutomationTarget)>,
    // AR 自动化 lane 的 M/S 试听状态（子行按钮切换，切后需重载音频）。
    am_ms: &mut std::collections::HashMap<(u16, AutomationTarget), yinhe_types::AmMsState>,
) -> (bool, bool, Vec<TrackAction>) {
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();
    let num_tracks = track_info.len();

    if num_tracks == 0 || panel_w < 1.0 || panel_h < 1.0 {
        return (false, false, Vec::new());
    }

    let mut actions = Vec::new();

    let show_details = *row_height >= 30.0;

    // Clamp scroll_y（总行数含展开的 AM 子行）
    let total_rows = row_layout.total_rows();
    let max_scroll = (total_rows as f32 * *row_height - panel_h).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // ── 滚轮垂直滚动 ──
    // 必须先于行绘制处理：否则本帧面板按旧 scroll_y 绘制、GPU 按写回的新值上传，
    // egui 面板与 GPU 区差 1 帧（与横向滚动"clamp 后再画标尺"同帧同步同理）。
    if crate::view_interaction::pointer_hits(ui, panel_rect) {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            *scroll_y = (*scroll_y - scroll_delta.y).clamp(0.0, max_scroll);
        }
    }

    // ── 拖拽排序跨帧状态（算法见 widgets::reorder） ──
    let drag_id = ui.id().with("track_panel_drag");
    let mut drag: Option<crate::widgets::reorder::DragReorder> =
        ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or_default();
    let dragging = drag.is_some();

    // Visible row range（含展开的 AM 子行；行高均匀 = 音轨行高）
    let lh = *row_height;
    let first_row = ((*scroll_y / lh).floor().max(0.0) as usize).min(total_rows);
    let last_row = (((*scroll_y + panel_h) / lh).ceil().max(0.0) as usize + 1).min(total_rows);

    let painter = ui.painter().clone();
    let mut audio_dirty = false;
    // AM lane M/S 试听状态变化 → 调用方重载音频（带 am_ms 旁通）。
    let mut am_ms_dirty = false;

    // 交替行条纹：着色行（偶数行号）与 GPU 区同源颜色，不透明；奇数行用 app_bg 打底。
    // 按全局行号奇偶（AM 子行也参与）：展开奇数条 lane 会错位后续音轨的斑纹。
    let lane_even = crate::theme::stripe_bg();

    let interact_id = egui::Id::new("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());

    let btn_size = egui::vec2(18.0, 18.0);

    // 主行矩形（含视口外/隐藏行，保证拖拽插入索引全局正确；AM 子行不参与排序）；
    // 仅可视行渲染。
    let mut item_rects: Vec<egui::Rect> = Vec::with_capacity(num_tracks);
    for idx in 0..num_tracks {
        let y = panel_rect.min.y + row_layout.track_y(idx, lh) - *scroll_y;
        item_rects.push(egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, lh),
        ));
    }

    // chevron 命中区：按下在 chevron 上不触发排序/选择（只翻转展开状态）。
    let mut chevron_rects: Vec<egui::Rect> = Vec::new();

    // 兄弟轨道联动：鼠标落在某音轨的主行/任意自动化行上，视为悬停整个
    // 「兄弟轨道」（该音轨及其全部自动化轨），它们的 chevron/加号一并显示。
    let hover_track = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|&pos| panel_rect.contains(pos))
        .and_then(|pos| row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh))
        .map(|h| h.track());

    for row in first_row..last_row {
        let y = panel_rect.min.y + row as f32 * lh - *scroll_y;
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(panel_rect.min.x, y), egui::vec2(panel_w, lh));
        let Some(row_hit) = row_layout.row_hit(row) else {
            continue;
        };
        let idx = row_hit.track();
        if !track_visible.get(idx).copied().unwrap_or(true) {
            continue;
        }
        if y > panel_rect.max.y || y + lh < panel_rect.min.y {
            continue;
        }

        // ── AM 行：复用普通轨的行样式（两条文本：第一行 = 自动化名，第二行 = 所属音轨）。
        // 色条同宽、M/S 按钮同位置；无 chevron（像 tempo 一样）。
        if let ArRow::Automation(track, sub) = row_hit {
            let Some(lane) = tracks.get(track).and_then(|t| t.automation_lanes.get(sub)) else {
                continue;
            };
            if row % 2 == 0 {
                painter.rect_filled(row_rect, 0.0, lane_even);
            }
            let lane_key = (track_info[track].index, lane.target.clone());
            if am_lane_selected.contains(&lane_key) {
                painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
            } else if row_rect.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or_default())) {
                painter.rect_filled(
                    row_rect,
                    0.0,
                    crate::theme::hover_color(crate::theme::app_bg()),
                );
            }
            let color = track_colors
                .get(track)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR);
            let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));
            let badge_w = 14.0_f32;
            let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, lh));
            painter.rect_filled(badge_rect, 0.0, color32);

            // 完全复刻普通轨的字号与几何：详情模式两行（×0.25，y 0.30/0.70，均 primary），
            // 紧凑模式单行（×0.45 居中）；唯一差异是文本内容（自动化名 / 所属音轨）。
            let text_x = badge_rect.max.x + 6.0;
            let label = super::am_lanes::lane_label(&lane.target);
            let owner = track_info[track].name.clone();
            if show_details {
                let font = egui::FontId::proportional((lh * 0.25).clamp(9.0, 13.0));
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
                let font = egui::FontId::proportional((lh * 0.45).clamp(8.0, 14.0));
                painter.text(
                    egui::pos2(text_x, badge_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    format!("{} · {}", label, owner),
                    font,
                    crate::theme::text_primary(),
                );
            }

            // M/S 试听按钮：位置与普通轨同款（18px 大按钮，右缘对齐）；
            // 与普通轨一致，紧凑模式（!show_details）不显示。
            if show_details {
                let key = (track_info[track].index, lane.target.clone());
                let st = am_ms.get(&key).copied().unwrap_or_default();
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
                    st.mute,
                    crate::theme::mute_active(),
                    egui::Id::new(("am_btn_m", track, sub)),
                );
                let s_resp = draw_inline_button(
                    ui,
                    &painter,
                    s_rect,
                    "S",
                    st.solo,
                    crate::theme::solo_active(),
                    egui::Id::new(("am_btn_s", track, sub)),
                );
                if m_resp.clicked() || s_resp.clicked() {
                    let mut next = st;
                    if m_resp.clicked() {
                        next.mute = !next.mute;
                    }
                    if s_resp.clicked() {
                        next.solo = !next.solo;
                    }
                    if next == yinhe_types::AmMsState::default() {
                        am_ms.remove(&key);
                    } else {
                        am_ms.insert(key, next);
                    }
                    am_ms_dirty = true;
                }
            }

            // 最后一条自动化行：在该行右侧（M/S 旁）显示加号，点击再次添加自动化。
            let is_last_lane = tracks
                .get(track)
                .is_some_and(|t| sub + 1 == t.automation_lanes.len());
            if is_last_lane {
                let add_open =
                    egui::Popup::is_id_open(ui.ctx(), egui::Id::new(("arr_add_pop", track)));
                if hover_track == Some(track) || add_open {
                    let lum = color[0] * 0.299 + color[1] * 0.587 + color[2] * 0.114;
                    let plus_color = if lum > 0.55 {
                        egui::Color32::BLACK
                    } else {
                        egui::Color32::WHITE
                    };
                    // 加号放在色带列底部（与主行 chevron 同位置 lh*0.62）。
                    let badge_center_x = row_rect.min.x + 14.0 * 0.5;
                    badge_icon_menu(
                        ui,
                        egui::pos2(badge_center_x, row_rect.min.y + lh * 0.62),
                        ICON_ADD.codepoint,
                        ICON_ADD.font_family(),
                        plus_color,
                        track,
                        |ui| create_automation_menu(ui, track, tracks, &mut actions),
                    );
                }
            }
            continue;
        }

        let ti = &track_info[idx];

        let is_conductor = Some(ti.index) == conductor_track_idx;
        let selected = track_selected.contains(&ti.index);
        // 着色行条纹（奇数行 = app_bg 普通行，不画；选中/悬停 tint 在条纹之上）
        if row % 2 == 0 {
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

        let color = if is_conductor {
            crate::theme::conductor_color_f32()
        } else {
            track_colors
                .get(idx)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR)
        };
        let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));

        // 色条统一为窄版（conductor 同宽，只是不放 chevron）。
        let badge_w = 14.0_f32;
        let badge_rect = egui::Rect::from_min_size(row_rect.min, egui::vec2(badge_w, lh));
        painter.rect_filled(badge_rect, 0.0, color32);

        if !is_conductor {
            let expanded = arr_am_expanded.get(idx).copied().unwrap_or(false);
            // 未展开且完全没有任何自动化 → 直接在色带上给「+」，点击弹创建自动化菜单
            //（无需先展开）；否则照常显示展开/收起 chevron。
            let has_any_lane = tracks
                .get(idx)
                .is_some_and(|t| !t.automation_lanes.is_empty());
            let lum = color[0] * 0.299 + color[1] * 0.587 + color[2] * 0.114;
            let icon_color = if lum > 0.55 {
                egui::Color32::BLACK
            } else {
                egui::Color32::WHITE
            };
            // 兄弟轨道联动：本轨任意行（主/自动化）被悬停时，图标一并显示。
            let family_hovered = hover_track == Some(idx);
            if !expanded && !has_any_lane {
                // 无自动化未展开：色带同一图标位给无边框「+」，点弹创建自动化菜单；
                // 与 chevron 一样仅悬浮显示；popup 打开期间持续渲染加号防漂移。
                let add_open =
                    egui::Popup::is_id_open(ui.ctx(), egui::Id::new(("arr_add_pop", idx)));
                if family_hovered || add_open {
                    badge_icon_menu(
                        ui,
                        egui::pos2(badge_rect.center().x, badge_rect.min.y + lh * 0.62),
                        ICON_ADD.codepoint,
                        ICON_ADD.font_family(),
                        icon_color,
                        idx,
                        |ui| create_automation_menu(ui, idx, tracks, &mut actions),
                    );
                }
            } else {
                let icon = if expanded {
                    ICON_KEYBOARD_ARROW_DOWN
                } else {
                    ICON_KEYBOARD_ARROW_RIGHT
                };
                // chevron 仅在该轨家族被悬浮时显示；颜色按音轨颜色亮度选黑/白保证对比度。
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(badge_rect.center().x, badge_rect.min.y + lh * 0.62),
                    egui::vec2(12.0, lh.min(16.0)),
                );
                let chev_resp = ui.interact(
                    icon_rect,
                    egui::Id::new(("am_chevron", idx)),
                    egui::Sense::click(),
                );
                if family_hovered {
                    chevron_rects.push(icon_rect);
                    painter.text(
                        icon_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon.codepoint,
                        egui::FontId::new(crate::theme::ICON_BTN_FONT, icon.font_family()),
                        icon_color,
                    );
                }
                if chev_resp.clicked()
                    && let Some(e) = arr_am_expanded.get_mut(idx)
                {
                    *e = !*e;
                }
            }
        }

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
                match tracks.get(idx) {
                    // 乐器轨显示乐器通道（与 MIDI 通道是两套独立命名空间）。
                    Some(t) if t.kind == yinhe_core::TrackKind::Instrument => t
                        .instrument_channel
                        // u32 转换避免 u16 上限（65535）+1 溢出 panic。
                        .map(|c| format!("I{:02}", u32::from(c) + 1))
                        .unwrap_or_else(|| "I--".to_string()),
                    // 音频轨（预留）显示 AU。
                    Some(t) if t.kind == yinhe_core::TrackKind::Audio => "AU".to_string(),
                    // MIDI 轨：port 字母（A..P）+ 通道号。
                    _ => format!("{}{:02}", (b'A' + ti.port.min(15)) as char, ti.channel + 1),
                }
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
    // 行命中 → 音轨（AM 子行归到所属音轨；双击/单击子行等效于主行）。
    let hit = |pos: egui::Pos2| -> Option<usize> {
        let rel_y = pos.y - panel_rect.min.y + *scroll_y;
        row_layout.hit_at_music_y(rel_y, lh).map(|h| h.track())
    };

    if resp.double_clicked() && !dragging {
        if let Some(pos) = resp.interact_pointer_pos()
            && let Some(idx) = hit(pos)
        {
            // 双击：选中该行（track_selected = {该行}，即成为主音轨）并打开 PR。
            // Conductor 双击同样选中（Tempo automation 编辑照旧，主音轨 = Conductor）。
            let track_idx = track_info[idx].index;
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
            *request_pianoroll = true;
            // 双击 = 编辑主轨：清除 AM lane 选择。
            am_lane_selected.clear();
        }
    } else if resp.clicked()
        && !dragging
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(row_hit) = row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
    {
        let shift = ui.input(|i| i.modifiers.shift);
        let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);

        // AM 子行点击：选择 lane 本身（选中高亮子行；PR 仍显示主轨音符）。
        if let ArRow::Automation(t, s) = row_hit {
            if let Some(lane) = tracks.get(t).and_then(|tr| tr.automation_lanes.get(s)) {
                let key = (track_info[t].index, lane.target.clone());
                if cmd {
                    // Toggle this lane.
                    if !am_lane_selected.remove(&key) {
                        am_lane_selected.insert(key);
                    }
                } else {
                    // Plain/shift：替换为唯一选中（shift 简化同 plain）。
                    am_lane_selected.clear();
                    am_lane_selected.insert(key);
                }
            }
            // 点子行：把主轨写入 track_selected（与主行点击互斥：
            // 点主行清 arr_am_selected，点子行选中主轨并保留 lane 选中）。
            // 选中的 AM lane 对应卷帘显示主轨音符（主音轨强制可见，不强制切换当前视图）。
            track_selected.clear();
            track_selected.insert(track_info[t].index);
            *selection_anchor = None;
        } else {
            let idx = row_hit.track();
            let track_idx = track_info[idx].index;
            // 选中主行：清除 AM lane 选择（互斥）。
            am_lane_selected.clear();
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
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
    }

    // On secondary click, select the track under the cursor and record its
    // index in egui temp data so the context_menu closure (which may run on
    // subsequent frames while the menu stays open) can recover it.
    let ctx_menu_idx_id = egui::Id::new("track_ctx_menu_idx");
    if resp.secondary_clicked()
        && let Some(pos) = resp.interact_pointer_pos()
        && let Some(row_hit) = row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh)
    {
        // AM 子行右键：额外记录 lane 下标，菜单只给「删除自动化」；并选中该 lane。
        // 加号行右键等同主行（含创建自动化）。
        let (idx, sub) = match row_hit {
            ArRow::Track(t) => (t, None),
            ArRow::Automation(t, s) => (t, Some(s)),
        };
        let track_idx = track_info[idx].index;
        if let Some(s) = sub {
            am_lane_selected.clear();
            if let Some(lane) = tracks.get(idx).and_then(|tr| tr.automation_lanes.get(s)) {
                let key = (track_idx, lane.target.clone());
                if !am_lane_selected.contains(&key) {
                    am_lane_selected.insert(key);
                }
            }
            track_selected.clear();
            *selection_anchor = None;
        } else if !track_selected.contains(&track_idx) {
            am_lane_selected.clear();
            track_selected.clear();
            track_selected.insert(track_idx);
            *selection_anchor = Some(track_idx);
        }
        *info_content = Some(crate::right_panel::InfoContent::Track);
        ui.ctx()
            .data_mut(|d| d.insert_temp(ctx_menu_idx_id, (idx, sub)));
    }

    resp.context_menu(|ui| {
        ui.set_min_width(160.0);
        ui.set_max_width(160.0);
        let (idx, sub) = ui
            .ctx()
            .data(|d| d.get_temp::<(usize, Option<usize>)>(ctx_menu_idx_id))
            .unwrap_or((0, None));
        let track_idx = track_info.get(idx).map(|t| t.index).unwrap_or(0);
        let is_conductor = conductor_track_idx == Some(track_idx);

        // 任意音轨（含 Conductor）顶部：「音轨属性」→ 选中并打开浮窗。
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("arrange.track_properties").as_ref(),
            ))
            .clicked()
        {
            actions.push(TrackAction::ShowProperties { idx });
            ui.close();
        }
        ui.separator();

        // AM 子行右键：只给「删除自动化」。
        if let Some(lane_idx) = sub {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.delete_automation"),
                ))
                .clicked()
            {
                actions.push(TrackAction::DeleteAutomation { idx, lane_idx });
                ui.close();
            }
            return;
        }

        if !is_conductor {
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.add_below"),
                ))
                .clicked()
            {
                actions.push(TrackAction::AddTrack {
                    after_idx: Some(idx),
                });
                ui.close();
            }
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.add_above"),
                ))
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
                    .add(crate::widgets::menu::menu_item_button(
                        ui,
                        false,
                        t!("arrange.move_up"),
                    ))
                    .clicked()
            {
                actions.push(TrackAction::MoveUp { idx });
                ui.close();
            }
            if idx < num_tracks - 1
                && ui
                    .add(crate::widgets::menu::menu_item_button(
                        ui,
                        false,
                        t!("arrange.move_down"),
                    ))
                    .clicked()
            {
                actions.push(TrackAction::MoveDown { idx });
                ui.close();
            }
            ui.separator();
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.delete_track"),
                ))
                .clicked()
            {
                actions.push(TrackAction::RemoveTrack { idx });
                ui.close();
            }
            ui.separator();
            create_automation_menu(ui, idx, tracks, &mut actions);
        } else {
            // Conductor track: only allow adding after
            if ui
                .add(crate::widgets::menu::menu_item_button(
                    ui,
                    false,
                    t!("arrange.add_below"),
                ))
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
        && !chevron_rects.iter().any(|r| r.contains(pos))
        // 只有主行能拖动排序，AM 子行不起排序。
        && matches!(
            row_layout.hit_at_music_y(pos.y - panel_rect.min.y + *scroll_y, lh),
            Some(ArRow::Track(_))
        )
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

    ui.data_mut(|d| d.insert_temp(drag_id, drag));

    (audio_dirty, am_ms_dirty, actions)
}

/// 「创建自动化」子菜单（主行右键 + 加号占位行共用）：复用 PR AM 面板的
/// target 列表（跳过 Tempo；已有 lane 的 target 不重复显示），自定义 CC 用
/// DragValue 选控制器号。选择后 push CreateAutomation 交给 arrange.rs 落模型。
fn create_automation_menu(
    ui: &mut egui::Ui,
    idx: usize,
    tracks: &[Arc<yinhe_core::TrackData>],
    actions: &mut Vec<TrackAction>,
) {
    // 面板轮廓由调用方提供（badge popup 用 Popup::new 的 Frame::menu；
    // 右键菜单自带 context_menu 面板），这里只渲染无边框等宽的菜单项。
    ui.set_min_width(160.0);
    ui.set_max_width(160.0);
    let existing: Vec<AutomationTarget> = tracks
        .get(idx)
        .map(|t| {
            t.automation_lanes
                .iter()
                .map(|l| l.target.clone())
                .collect()
        })
        .unwrap_or_default();
    for target in crate::piano_view::automation_panel::AUTOMATION_TARGETS {
        if matches!(target, AutomationTarget::Tempo) || existing.contains(target) {
            continue;
        }
        let label = super::am_lanes::lane_label(target);
        if ui
            .add(crate::widgets::menu::menu_item_button(ui, false, label))
            .clicked()
        {
            actions.push(TrackAction::CreateAutomation {
                idx,
                target: target.clone(),
            });
            ui.close();
        }
    }
    ui.separator();
    // 自定义 CC：菜单内 DragValue（0..=127）+ 无边框「创建」按钮。
    let cc_id = egui::Id::new(("arr_custom_cc", idx));
    let mut cc = ui.ctx().data_mut(|d| d.get_temp::<u8>(cc_id)).unwrap_or(7);
    ui.horizontal(|ui| {
        ui.label(t!("arrange.custom_cc"));
        if ui
            .add(egui::DragValue::new(&mut cc).range(0..=127))
            .changed()
        {
            ui.ctx().data_mut(|d| d.insert_temp(cc_id, cc));
        }
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                false,
                t!("arrange.create"),
            ))
            .clicked()
        {
            let target = AutomationTarget::CC { controller: cc };
            if !existing.contains(&target) {
                actions.push(TrackAction::CreateAutomation { idx, target });
            }
            ui.close();
        }
    });
}

/// 在色带图标位置（与 chevron 同坐标、同尺寸、同绘制方式）画一个图标，点击弹菜单。
///
/// 图标用 `painter.text` 严格 `CENTER_CENTER` 居中（同 chevron）。点击时用固定的
/// popup id（`arr_add_pop_{track}`）打开，并把加号中心的**屏幕坐标**存为锚点；
/// popup 用 `Popup::new(id, ?, Position(固定锚点), ...)` 渲染——锚点不与 hover 行绑定，
/// 只要该加号在 popup 打开期间持续渲染（调用方用 `add_open` 保证），popup 就会稳定
/// 落在加号旁固定位置，鼠标移向菜单不会漂移或消失。
fn badge_icon_menu(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    codepoint: &str,
    family: egui::FontFamily,
    color: egui::Color32,
    track: usize,
    body: impl FnOnce(&mut egui::Ui),
) {
    let size = egui::vec2(12.0, 16.0);
    let rect = egui::Rect::from_center_size(center, size);
    let popup_id = egui::Id::new(("arr_add_pop", track));
    let resp = ui.interact(
        rect,
        egui::Id::new(("badge_icon", track)),
        egui::Sense::click(),
    );
    // 图标严格水平+垂直居中对齐（同 chevron），不会因按钮 padding 右偏。
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        codepoint,
        egui::FontId::new(crate::theme::ICON_BTN_FONT, family),
        color,
    );
    // 点击：打开 popup 并在 memory 记下加号中心屏幕坐标作为固定锚点。
    if resp.clicked() {
        // rect 是 ui 局部坐标，转成全局屏幕坐标。
        let screen_center = ui
            .ctx()
            .layer_transform_to_global(resp.layer_id)
            .map(|t| t * rect.center())
            .unwrap_or(rect.center());
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                Anchor::key(),
                Anchor {
                    track,
                    pos: screen_center,
                },
            )
        });
        egui::Popup::open_id(ui.ctx(), popup_id);
    }
    // 打开状态下渲染 popup（open_memory(None)：不干预 memory 的开关）。
    if egui::Popup::is_id_open(ui.ctx(), popup_id)
        && let Some(anchor) = ui.ctx().data_mut(|d| d.get_temp::<Anchor>(Anchor::key()))
        && anchor.track == track
    {
        egui::Popup::new(
            popup_id,
            ui.ctx().clone(),
            anchor.pos, // impl From<Pos2> → PopupAnchor::Position
            egui::LayerId::new(egui::Order::Middle, egui::Id::new("arr_add_popup_layer")),
        )
        .frame(egui::Frame::menu(ui.style())) // 只一层菜单轮廓，不再由 body 内部再套
        .open_memory(None)
        .show(|ui| {
            ui.set_min_width(160.0);
            ui.set_max_width(160.0);
            body(ui);
        });
    }
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
