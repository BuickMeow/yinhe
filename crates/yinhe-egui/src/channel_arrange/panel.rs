//! 通道 AR 左侧通道面板：Conductor 行、通道行（含展开 chevron）、
//! 轨道音符行（M/S + 力度表）、自动化子行（lane 名 + 试听 M/S + 添加加号）。
//!
//! 复用轨道 AR 轨道面板的绘制原语（行背景 / 色带 / 内联按钮）。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use eframe::egui;
use egui_material_icons::icons::{ICON_ADD, ICON_KEYBOARD_ARROW_DOWN, ICON_KEYBOARD_ARROW_RIGHT};
use rust_i18n::t;

use yinhe_types::{AmMsState, AutomationTarget};

use super::groups::{CaChannel, CaGroups};
use super::rows::{CaRow, CaRowLayout};

use crate::arrange::track_panel::draw::draw_inline_button;
use crate::arrange::track_panel::hover::icon_contrast_color;
use crate::arrange::track_panel::render::{
    compact_font, detail_font, draw_badge, draw_row_background,
};

/// 色带图标（chevron / 加号）命中区高 16px，其底边距行底（色带底）1px，
/// 因此中心距行底 16 / 2 + 1 = 9px（与轨道面板一致）。
const BADGE_ICON_BOTTOM_INSET: f32 = 9.0;
/// 每轨力度表：条宽 / 贴边距 / 与 M/S 的间隙（px）。
const VEL_METER_W: f32 = 5.0;
const VEL_METER_MARGIN: f32 = 3.0;
const VEL_METER_GAP: f32 = 5.0;
/// 面板内联按钮尺寸。
const BTN_SIZE: f32 = 18.0;
const BTN_GAP: f32 = 2.0;

/// 面板一帧的可变输入（借用聚合）。
pub(crate) struct CaPanelIo<'a> {
    pub track_info: &'a [yinhe_core::TrackInfo],
    /// 每轨显示色（= 通道 AR 统一后的通道色；Conductor 为主题色）。
    pub track_colors: &'a [[f32; 4]],
    pub track_visible: &'a [bool],
    pub track_meters: &'a [(f32, f32)],
    pub track_overrides: &'a mut [yinhe_editor_core::document::TrackOverride],
    pub track_selected: &'a mut HashSet<u16>,
    pub selection_anchor: &'a mut Option<u16>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    pub expanded_midi: &'a mut Vec<bool>,
    pub expanded_audio: &'a mut Vec<bool>,
    pub tracks: &'a [Arc<yinhe_core::TrackData>],
    pub arr_am_ms: &'a mut HashMap<(u16, AutomationTarget), AmMsState>,
    /// 双击轨道行 → 请求打开钢琴卷帘。
    pub request_pianoroll: &'a mut bool,
    /// 混音台 MIDI 通道 strip（索引 = 源通道 0..256；通道行 M/S）。
    pub midi_strips: &'a [yinhe_mixer::StripParams],
    /// 混音台音频通道 strip（索引 = 音频通道；通道行 M/S）。
    pub audio_strips: &'a [yinhe_mixer::StripParams],
    /// MIDI 通道电平显示值（索引 = 源通道；(L,R) level/hold）。
    pub midi_levels: &'a [crate::widgets::meter::StereoMeter],
    /// 音频通道电平显示值（索引 = 音频通道）。
    pub audio_levels: &'a [crate::widgets::meter::StereoMeter],
}

/// 面板一帧输出。
#[derive(Default)]
pub(crate) struct CaPanelOut {
    /// 轨道 M/S 变化 → 重发 skip 掩码。
    pub audio_dirty: bool,
    /// AM 试听 M/S 变化 → 重发 am_ms。
    pub am_ms_dirty: bool,
    /// 「添加自动化」请求（轨道索引）。
    pub automation_picker: Option<u16>,
    /// 通道行 M/S 变更（MIDI 源通道 → 新 strip 参数）。
    pub set_strips: Vec<(u8, yinhe_mixer::StripParams)>,
    /// 通道行 M/S 变更（音频通道 → 新 strip 参数）。
    pub set_audio_strips: Vec<(u16, yinhe_mixer::StripParams)>,
}

/// 通道是否展开。
fn is_expanded(ch: CaChannel, io: &CaPanelIo<'_>) -> bool {
    match ch {
        CaChannel::Midi(c) => io.expanded_midi.get(c as usize).copied().unwrap_or(false),
        CaChannel::Audio(c) => io.expanded_audio.get(c as usize).copied().unwrap_or(false),
    }
}

/// 切换展开状态。
fn toggle_expanded(ch: CaChannel, io: &mut CaPanelIo<'_>) {
    match ch {
        CaChannel::Midi(c) => {
            let i = c as usize;
            if io.expanded_midi.len() <= i {
                io.expanded_midi.resize(i + 1, false);
            }
            io.expanded_midi[i] = !io.expanded_midi[i];
        }
        CaChannel::Audio(c) => {
            let i = c as usize;
            if io.expanded_audio.len() <= i {
                io.expanded_audio.resize(i + 1, false);
            }
            io.expanded_audio[i] = !io.expanded_audio[i];
        }
    }
}

/// 通道显示名（"MIDI-A01" / "Audio-01"）。
fn channel_name(ch: CaChannel) -> String {
    match ch {
        CaChannel::Midi(c) => crate::mix::channel_label(c),
        CaChannel::Audio(c) => crate::mix::audio_label(c),
    }
}

/// 每帧绘制通道面板。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    view: &mut yinhe_types::ArrangementView,
    layout: &CaRowLayout,
    groups: &CaGroups,
    conductor_color: [f32; 4],
    io: &mut CaPanelIo<'_>,
) -> CaPanelOut {
    let mut out = CaPanelOut::default();
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();
    if panel_w < 1.0 || panel_h < 1.0 || layout.is_empty() {
        return out;
    }

    ui.painter()
        .rect_filled(panel_rect, 0.0, crate::theme::app_bg());

    // ── 滚轮垂直滚动（clamp 到内容总高）──
    let max_scroll = (layout.total_height() - panel_h).max(0.0);
    if crate::view_interaction::pointer_hits(ui, panel_rect) {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            view.base.scroll_y = (view.base.scroll_y - scroll_delta.y).clamp(0.0, max_scroll);
        }
        // ctrl/cmd+滚轮（pinch 同理）：行高离散缩放，锚定指针 y。
        let zoom_delta = ui.input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001
            && let Some(hover) = ui.input(|i| i.pointer.hover_pos())
        {
            view.zoom_lane_height(hover.y - panel_rect.min.y, zoom_delta);
        }
    }
    let scroll_y = view.base.scroll_y;
    let visible = layout.visible_rows(scroll_y, panel_h);
    let stripe = crate::theme::stripe_bg();
    let painter = ui.painter().clone();
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());
    let over_popup = crate::view_interaction::pointer_over_popup(ui.ctx());

    for row in visible {
        let y = panel_rect.min.y + layout.y_top(row) - scroll_y;
        let h = layout.height(row);
        if y > panel_rect.max.y || y + h < panel_rect.min.y {
            continue;
        }
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(panel_rect.min.x, y), egui::vec2(panel_w, h));
        let Some(ca_row) = layout.row(row) else {
            continue;
        };
        // 不可见轨道不绘制行内容（与轨道 AR 面板一致）。
        if let Some(t) = ca_row.track()
            && !io.track_visible.get(t as usize).copied().unwrap_or(true)
        {
            continue;
        }
        let selected = match ca_row {
            CaRow::Track(t) | CaRow::Automation { track: t, .. } => io.track_selected.contains(&t),
            CaRow::Channel(ch) => groups.group(ch).is_some_and(|g| {
                !g.tracks.is_empty() && g.tracks.iter().all(|t| io.track_selected.contains(t))
            }),
            CaRow::Conductor => false,
        };
        draw_row_background(ui, &painter, row_rect, row, selected, stripe);

        let resp = ui.interact(
            row_rect,
            egui::Id::new(("ca_panel_row", row)),
            egui::Sense::click(),
        );
        let hovered = pointer_pos.is_some_and(|p| row_rect.contains(p)) && !over_popup;
        let color = match ca_row {
            CaRow::Conductor => conductor_color,
            CaRow::Channel(ch) => groups
                .group(ch)
                .map(|g| g.color(io.track_colors))
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR),
            CaRow::Track(t) | CaRow::Automation { track: t, .. } => io
                .track_colors
                .get(t as usize)
                .copied()
                .unwrap_or(yinhe_core::DEFAULT_TRACK_COLOR),
        };
        let color32 = crate::theme::rgba_to_color32((color[0], color[1], color[2], color[3]));
        let badge_rect = draw_badge(&painter, row_rect, h, color32);
        let text_x = badge_rect.max.x + 6.0;
        let show_details = h >= 30.0;

        match ca_row {
            CaRow::Conductor => {
                let font = if show_details {
                    detail_font(h)
                } else {
                    compact_font(h)
                };
                if show_details {
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.30),
                        egui::Align2::LEFT_CENTER,
                        "Master",
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.70),
                        egui::Align2::LEFT_CENTER,
                        "Conductor",
                        font,
                        crate::theme::text_primary(),
                    );
                } else {
                    painter.text(
                        egui::pos2(text_x, badge_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "Master",
                        font,
                        crate::theme::text_primary(),
                    );
                }
            }
            CaRow::Channel(ch) => {
                let name = channel_name(ch);
                let count = groups.group(ch).map(|g| g.tracks.len()).unwrap_or(0);
                let expanded = is_expanded(ch, io);
                let font = if show_details {
                    detail_font(h)
                } else {
                    compact_font(h)
                };
                if show_details {
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.70),
                        egui::Align2::LEFT_CENTER,
                        t!("ca.tracks", n = count),
                        font,
                        crate::theme::text_secondary(),
                    );
                } else {
                    painter.text(
                        egui::pos2(text_x, badge_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &name,
                        font,
                        crate::theme::text_primary(),
                    );
                }

                // 通道行 = 混音台通道条：M/S（详情模式）+ 双声道电平表（贴右缘）。
                let (muted, soloed, strip) = match ch {
                    CaChannel::Midi(c) => {
                        let p = io.midi_strips.get(c as usize).copied().unwrap_or_default();
                        (p.mute, p.solo, p)
                    }
                    CaChannel::Audio(c) => {
                        let p = io.audio_strips.get(c as usize).copied().unwrap_or_default();
                        (p.mute, p.solo, p)
                    }
                };
                let (level, hold) = match ch {
                    CaChannel::Midi(c) => io
                        .midi_levels
                        .get(c as usize)
                        .copied()
                        .unwrap_or(((0.0, 0.0), (0.0, 0.0))),
                    CaChannel::Audio(c) => io
                        .audio_levels
                        .get(c as usize)
                        .copied()
                        .unwrap_or(((0.0, 0.0), (0.0, 0.0))),
                };
                let bar_w = crate::widgets::meter::METER_W * 2.0 + crate::widgets::meter::METER_GAP;
                let meter_right = row_rect.max.x - 4.0;
                let meter_rect = egui::Rect::from_min_max(
                    egui::pos2(meter_right - bar_w, row_rect.min.y + 4.0),
                    egui::pos2(meter_right, row_rect.max.y - 4.0),
                );
                crate::widgets::meter::compact_meter(&painter, meter_rect, level, hold);
                if show_details {
                    let total_btn_w = 2.0 * BTN_SIZE + BTN_GAP;
                    let chan_btn_x = meter_rect.min.x - 6.0 - total_btn_w;
                    let chan_btn_y = badge_rect.center().y - BTN_SIZE * 0.5;
                    let m_rect = egui::Rect::from_min_size(
                        egui::pos2(chan_btn_x, chan_btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let s_rect = egui::Rect::from_min_size(
                        egui::pos2(chan_btn_x + BTN_SIZE + BTN_GAP, chan_btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let m_resp = draw_inline_button(
                        ui,
                        &painter,
                        m_rect,
                        "M",
                        muted,
                        crate::theme::mute_active(),
                        egui::Id::new(("ca_chan_m", ch)),
                    );
                    let s_resp = draw_inline_button(
                        ui,
                        &painter,
                        s_rect,
                        "S",
                        soloed,
                        crate::theme::solo_active(),
                        egui::Id::new(("ca_chan_s", ch)),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &m_resp, t!("hint.mix.mute"));
                    crate::widgets::hint::hover(ui.ctx(), &s_resp, t!("hint.mix.solo"));
                    if m_resp.clicked() || s_resp.clicked() {
                        let mut next = strip;
                        if m_resp.clicked() {
                            next.mute = !next.mute;
                        }
                        if s_resp.clicked() {
                            next.solo = !next.solo;
                        }
                        match ch {
                            CaChannel::Midi(c) => out.set_strips.push((c, next)),
                            CaChannel::Audio(c) => out.set_audio_strips.push((c, next)),
                        }
                    }
                }

                // chevron（悬停显示；窄行也放色带底部）。
                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(
                        badge_rect.center().x,
                        badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
                    ),
                    egui::vec2(12.0, h.min(16.0)),
                );
                if hovered {
                    let icon = if expanded {
                        ICON_KEYBOARD_ARROW_DOWN
                    } else {
                        ICON_KEYBOARD_ARROW_RIGHT
                    };
                    painter.text(
                        icon_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        icon.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT, icon.font_family()),
                        icon_contrast_color(color),
                    );
                }
                if hovered && pointer_pos.is_some_and(|p| icon_rect.contains(p)) {
                    crate::widgets::hint::set(ui.ctx(), t!("hint.ca.channel_row"));
                }

                if resp.double_clicked() {
                    // 双击通道行：展开/收起（与单击 chevron 等效，else 分支防第二击重复处理）。
                    toggle_expanded(ch, io);
                } else if resp.clicked() {
                    let press_on_icon = resp
                        .interact_pointer_pos()
                        .is_some_and(|p| icon_rect.contains(p));
                    if press_on_icon {
                        toggle_expanded(ch, io);
                    } else if let Some(g) = groups.group(ch) {
                        io.track_selected.clear();
                        for &t in &g.tracks {
                            io.track_selected.insert(t);
                        }
                        *io.selection_anchor = g.tracks.first().copied();
                        *io.info_content = Some(crate::right_panel::InfoContent::Track);
                    }
                }
            }
            CaRow::Track(t) => {
                // 轨道行：编号 + 通道标签 + 轨名 + M/S + 力度表（对齐轨道 AR）。
                let ti = io.track_info.get(t as usize);
                let num_text = format!("{:03}", ti.map(|i| i.index).unwrap_or(t));
                let chan_text = match ti {
                    Some(_)
                        if io
                            .tracks
                            .get(t as usize)
                            .is_some_and(|tr| tr.kind == yinhe_core::TrackKind::Audio) =>
                    {
                        io.tracks
                            .get(t as usize)
                            .and_then(|tr| tr.audio_channel)
                            .map(crate::mix::audio_label)
                            .unwrap_or_else(|| "Audio--".to_string())
                    }
                    Some(i) => format!(
                        "MIDI-{}{:02}",
                        (b'A' + i.port.min(15)) as char,
                        i.channel.saturating_add(1)
                    ),
                    None => String::new(),
                };
                let font = if show_details {
                    detail_font(h)
                } else {
                    compact_font(h)
                };
                if show_details {
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &num_text,
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    painter.text(
                        egui::pos2(text_x + 32.0, badge_rect.min.y + h * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &chan_text,
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    if let Some(i) = ti {
                        painter.text(
                            egui::pos2(text_x, badge_rect.min.y + h * 0.70),
                            egui::Align2::LEFT_CENTER,
                            &i.name,
                            font.clone(),
                            crate::theme::text_primary(),
                        );
                    }

                    let muted = io
                        .track_overrides
                        .get(t as usize)
                        .map(|o| o.muted)
                        .unwrap_or(false);
                    let soloed = io
                        .track_overrides
                        .get(t as usize)
                        .map(|o| o.soloed)
                        .unwrap_or(false);
                    let total_btn_w = 2.0 * BTN_SIZE + BTN_GAP;
                    let meter_right = row_rect.max.x - VEL_METER_MARGIN;
                    let meter_left = meter_right - VEL_METER_W;
                    let btn_x_start = meter_left - VEL_METER_GAP - total_btn_w;
                    let btn_y = badge_rect.center().y - BTN_SIZE * 0.5;
                    // 两个 18px 按钮（与原 AR 相同尺寸/间距）。
                    let m_rect = egui::Rect::from_min_size(
                        egui::pos2(btn_x_start, btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let s_rect = egui::Rect::from_min_size(
                        egui::pos2(btn_x_start + BTN_SIZE + BTN_GAP, btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let m_resp = draw_inline_button(
                        ui,
                        &painter,
                        m_rect,
                        "M",
                        muted,
                        crate::theme::mute_active(),
                        egui::Id::new(("ca_track_m", t)),
                    );
                    let s_resp = draw_inline_button(
                        ui,
                        &painter,
                        s_rect,
                        "S",
                        soloed,
                        crate::theme::solo_active(),
                        egui::Id::new(("ca_track_s", t)),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &m_resp, t!("hint.arrange.track_mute"));
                    crate::widgets::hint::hover(ui.ctx(), &s_resp, t!("hint.arrange.track_solo"));
                    if m_resp.clicked()
                        && let Some(ov) = io.track_overrides.get_mut(t as usize)
                    {
                        ov.muted = !ov.muted;
                        out.audio_dirty = true;
                    }
                    if s_resp.clicked()
                        && let Some(ov) = io.track_overrides.get_mut(t as usize)
                    {
                        ov.soloed = !ov.soloed;
                        out.audio_dirty = true;
                    }

                    let meter_rect = egui::Rect::from_min_max(
                        egui::pos2(meter_left, row_rect.min.y + VEL_METER_MARGIN),
                        egui::pos2(meter_right, row_rect.max.y - VEL_METER_MARGIN),
                    );
                    let (level, hold) = io
                        .track_meters
                        .get(t as usize)
                        .copied()
                        .unwrap_or((0.0, 0.0));
                    crate::widgets::meter::velocity_meter(&painter, meter_rect, level, hold);
                } else {
                    painter.text(
                        egui::pos2(text_x, badge_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &num_text,
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    if let Some(i) = ti {
                        painter.text(
                            egui::pos2(text_x + 40.0, badge_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            &i.name,
                            font,
                            crate::theme::text_primary(),
                        );
                    }
                }

                if hovered {
                    crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.track_row"));
                }
                if resp.double_clicked() && !over_popup {
                    // 双击轨道行：选中并打开钢琴卷帘（与轨道 AR 一致）。
                    io.track_selected.clear();
                    io.track_selected.insert(t);
                    *io.selection_anchor = Some(t);
                    *io.info_content = Some(crate::right_panel::InfoContent::Track);
                    *io.request_pianoroll = true;
                } else if resp.clicked() && !over_popup {
                    io.track_selected.clear();
                    io.track_selected.insert(t);
                    *io.selection_anchor = Some(t);
                    *io.info_content = Some(crate::right_panel::InfoContent::Track);
                }
            }
            CaRow::Automation { track, sub } => {
                let Some(lane) = io
                    .tracks
                    .get(track as usize)
                    .and_then(|tr| tr.automation_lanes.get(sub))
                else {
                    continue;
                };
                let label = crate::arrange::lane_label(&lane.target);
                let owner = io
                    .track_info
                    .get(track as usize)
                    .map(|i| i.name.clone())
                    .unwrap_or_default();
                let font = if show_details {
                    detail_font(h)
                } else {
                    compact_font(h)
                };
                if show_details {
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &label,
                        font.clone(),
                        crate::theme::text_primary(),
                    );
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + h * 0.70),
                        egui::Align2::LEFT_CENTER,
                        &owner,
                        font,
                        crate::theme::text_primary(),
                    );
                    // AM 试听 M/S 按钮（右缘对齐，与轨道行的 M/S 同位置）。
                    let key = (track, lane.target.clone());
                    let st = io.arr_am_ms.get(&key).copied().unwrap_or_default();
                    let total_btn_w = 2.0 * BTN_SIZE + BTN_GAP;
                    let btn_x_start = row_rect.max.x - total_btn_w - 6.0;
                    let btn_y = badge_rect.center().y - BTN_SIZE * 0.5;
                    let m_rect = egui::Rect::from_min_size(
                        egui::pos2(btn_x_start, btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let s_rect = egui::Rect::from_min_size(
                        egui::pos2(btn_x_start + BTN_SIZE + BTN_GAP, btn_y),
                        egui::vec2(BTN_SIZE, BTN_SIZE),
                    );
                    let m_resp = draw_inline_button(
                        ui,
                        &painter,
                        m_rect,
                        "M",
                        st.mute,
                        crate::theme::mute_active(),
                        egui::Id::new(("ca_am_m", track, sub)),
                    );
                    let s_resp = draw_inline_button(
                        ui,
                        &painter,
                        s_rect,
                        "S",
                        st.solo,
                        crate::theme::solo_active(),
                        egui::Id::new(("ca_am_s", track, sub)),
                    );
                    crate::widgets::hint::hover(ui.ctx(), &m_resp, t!("hint.arrange.am_mute"));
                    crate::widgets::hint::hover(ui.ctx(), &s_resp, t!("hint.arrange.am_solo"));
                    if m_resp.clicked() || s_resp.clicked() {
                        let mut next = st;
                        if m_resp.clicked() {
                            next.mute = !next.mute;
                        }
                        if s_resp.clicked() {
                            next.solo = !next.solo;
                        }
                        if next == AmMsState::default() {
                            io.arr_am_ms.remove(&key);
                        } else {
                            io.arr_am_ms.insert(key, next);
                        }
                        out.am_ms_dirty = true;
                    }
                }

                // 最后一条自动化行：悬停显示加号（再次添加自动化）。
                let is_last_lane = io
                    .tracks
                    .get(track as usize)
                    .is_some_and(|tr| sub + 1 == tr.automation_lanes.len());
                if is_last_lane && hovered {
                    let plus_center = egui::pos2(
                        badge_rect.center().x,
                        badge_rect.max.y - BADGE_ICON_BOTTOM_INSET,
                    );
                    let plus_rect =
                        egui::Rect::from_center_size(plus_center, egui::vec2(12.0, 16.0));
                    painter.text(
                        plus_center,
                        egui::Align2::CENTER_CENTER,
                        ICON_ADD.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT, ICON_ADD.font_family()),
                        icon_contrast_color(color),
                    );
                    if pointer_pos.is_some_and(|p| plus_rect.contains(p)) {
                        crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.add_automation"));
                    }
                    if resp.clicked()
                        && resp
                            .interact_pointer_pos()
                            .is_some_and(|p| plus_rect.contains(p))
                    {
                        out.automation_picker = Some(track);
                    }
                }

                if resp.clicked() && out.automation_picker != Some(track) {
                    io.track_selected.clear();
                    io.track_selected.insert(track);
                    *io.selection_anchor = Some(track);
                    *io.info_content = Some(crate::right_panel::InfoContent::Track);
                }
            }
        }
    }

    out
}
