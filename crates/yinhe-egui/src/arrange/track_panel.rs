use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD, ICON_KEYBOARD_ARROW_DOWN, ICON_KEYBOARD_ARROW_RIGHT};

use yinhe_core::TrackInfo;
use yinhe_types::{ArRow, ArRowLayout, AutomationTarget};

use yinhe_editor_core::document::TrackOverride;

mod badge;
mod draw;
mod hover;
mod interaction;
mod menu;
mod render;
mod types;
pub(crate) use types::TrackAction;

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

    // Visible row range（含展开的 AM 子行；行高均匀 = 音轨行高）
    let lh = *row_height;
    let (first_row, last_row) = render::visible_range(*scroll_y, panel_h, lh, total_rows);

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
    let item_rects =
        render::build_item_rects(row_layout, panel_rect, panel_w, lh, *scroll_y, num_tracks);

    // chevron 命中区：按下在 chevron 上不触发排序/选择（只翻转展开状态）。
    let mut chevron_rects: Vec<egui::Rect> = Vec::new();

    // 兄弟轨道联动：鼠标落在某音轨的主行/任意自动化行上，视为悬停整个
    // 「兄弟轨道」（该音轨及其全部自动化轨），它们的 chevron/加号一并显示。
    // 成熟实现：走 pointer_hits + pointer_over_popup，感知 Foreground popup 遮挡
    let hover_track = hover::hover_track(ui, panel_rect, row_layout, *scroll_y, lh);

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
            let lane_key = (track_info[track].index, lane.target.clone());
            render::draw_row_background(
                ui,
                &painter,
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
            let badge_rect = render::draw_badge(&painter, row_rect, lh, color32);

            // 完全复刻普通轨的字号与几何：详情模式两行（×0.25，y 0.30/0.70，均 primary），
            // 紧凑模式单行（×0.45 居中）；唯一差异是文本内容（自动化名 / 所属音轨）。
            let text_x = badge_rect.max.x + 6.0;
            let label = super::am_lanes::lane_label(&lane.target);
            let owner = track_info[track].name.clone();
            if show_details {
                let font = render::detail_font(lh);
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
                let font = render::compact_font(lh);
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
                let m_resp = draw::draw_inline_button(
                    ui,
                    &painter,
                    m_rect,
                    "M",
                    st.mute,
                    crate::theme::mute_active(),
                    egui::Id::new(("am_btn_m", track, sub)),
                );
                let s_resp = draw::draw_inline_button(
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
                    let plus_color = hover::icon_contrast_color(color);
                    // 加号放在色带列底部（与主行 chevron 同位置 lh*0.62）。
                    let badge_center_x = row_rect.min.x + 14.0 * 0.5;
                    badge::badge_icon_menu(
                        ui,
                        egui::pos2(badge_center_x, row_rect.min.y + lh * 0.62),
                        ICON_ADD.codepoint,
                        ICON_ADD.font_family(),
                        plus_color,
                        track,
                        |ui| menu::create_automation_menu(ui, track, tracks, &mut actions),
                    );
                }
            }
            continue;
        }

        let ti = &track_info[idx];

        let is_conductor = Some(ti.index) == conductor_track_idx;
        let selected = track_selected.contains(&ti.index);
        render::draw_row_background(ui, &painter, row_rect, row, selected, lane_even);

        let color = if is_conductor {
            crate::theme::conductor_color_f32()
        } else {
            track_colors
                .get(idx)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR)
        };
        let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));
        let badge_rect = render::draw_badge(&painter, row_rect, lh, color32);

        if !is_conductor {
            let expanded = arr_am_expanded.get(idx).copied().unwrap_or(false);
            // 未展开且完全没有任何自动化 → 直接在色带上给「+」，点击弹创建自动化菜单
            //（无需先展开）；否则照常显示展开/收起 chevron。
            let has_any_lane = tracks
                .get(idx)
                .is_some_and(|t| !t.automation_lanes.is_empty());
            let icon_color = hover::icon_contrast_color(color);
            // 兄弟轨道联动：本轨任意行（主/自动化）被悬停时，图标一并显示。
            let family_hovered = hover_track == Some(idx);
            if !expanded && !has_any_lane {
                // 无自动化未展开：色带同一图标位给无边框「+」，点弹创建自动化菜单；
                // 与 chevron 一样仅悬浮显示；popup 打开期间持续渲染加号防漂移。
                let add_open =
                    egui::Popup::is_id_open(ui.ctx(), egui::Id::new(("arr_add_pop", idx)));
                if family_hovered || add_open {
                    badge::badge_icon_menu(
                        ui,
                        egui::pos2(badge_rect.center().x, badge_rect.min.y + lh * 0.62),
                        ICON_ADD.codepoint,
                        ICON_ADD.font_family(),
                        icon_color,
                        idx,
                        |ui| menu::create_automation_menu(ui, idx, tracks, &mut actions),
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
                        egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
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
            let font = render::detail_font(*row_height);

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
            let name_font = render::detail_font(*row_height);
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

                let m_resp = draw::draw_inline_button(
                    ui,
                    &painter,
                    m_rect,
                    "M",
                    muted,
                    crate::theme::mute_active(),
                    egui::Id::new(("track_btn_m", idx)),
                );
                let s_resp = draw::draw_inline_button(
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
            let font = render::compact_font(*row_height);
            painter.text(
                egui::pos2(text_x, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &track_num_text,
                font,
                crate::theme::text_primary(),
            );

            let name = &ti.name;
            let name_font = render::compact_font(*row_height);
            painter.text(
                egui::pos2(text_x + 40.0, badge_rect.center().y),
                egui::Align2::LEFT_CENTER,
                name,
                name_font,
                crate::theme::text_primary(),
            );
        }
    }

    interaction::handle_interactions(
        ui,
        &painter,
        panel_rect,
        row_layout,
        lh,
        scroll_y,
        track_info,
        track_visible,
        track_selected,
        selection_anchor,
        conductor_track_idx,
        num_tracks,
        am_lane_selected,
        &resp,
        &chevron_rects,
        &item_rects,
        &mut drag,
        drag_id,
        info_content,
        request_pianoroll,
        tracks,
        &mut actions,
        max_scroll,
    );

    (audio_dirty, am_ms_dirty, actions)
}
