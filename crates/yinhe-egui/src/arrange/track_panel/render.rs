use eframe::egui;
use rust_i18n::t;

use yinhe_types::ArRowLayout;

use egui_material_icons::icons::ICON_ADD;

use super::types::TrackAction;
use super::{
    BADGE_ICON_BOTTOM_INSET, LEVEL_METER_GAP, LEVEL_METER_MARGIN, LEVEL_METER_W, badge, draw, hover,
};

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

/// 自动化子行（AM lane）的完整绘制与 M/S 交互。
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
    btn_size: egui::Vec2,
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

    // M/S 试听按钮：位置与普通轨同款（18px 大按钮，右缘对齐）；
    // 与普通轨一致，紧凑模式（!show_details）不显示。
    if show_details {
        let key = (track_info[track].index, lane.target.clone());
        let st = am_ms.get(&key).copied().unwrap_or_default();
        let audibility = st.audibility();
        // 单按钮：始终显示 M；左键静音、右键独奏，亮色区分。
        // 自动化表贴右缘，按钮让位到其左侧（与普通轨布局一致）。
        let total_btn_w = btn_size.x;
        let meter_right = row_rect.max.x - LEVEL_METER_MARGIN;
        let meter_left = meter_right - LEVEL_METER_W;
        let btn_x_start = meter_left - LEVEL_METER_GAP - total_btn_w;
        let btn_y = badge_rect.center().y - btn_size.y * 0.5;
        let ms_rect = egui::Rect::from_min_size(egui::pos2(btn_x_start, btn_y), btn_size);
        no_drag_rects.push(ms_rect);
        let (label, active, color) = crate::widgets::audibility::visual(audibility);
        let resp = draw::draw_inline_button(
            ui,
            painter,
            ms_rect,
            label,
            active,
            color,
            egui::Id::unique(("am_btn_ms", track, sub)),
        );
        crate::widgets::hint::hover(ui.ctx(), &resp, t!("hint.arrange.am_mute"));
        let next = if resp.clicked() {
            Some(crate::widgets::audibility::next_on_primary(audibility))
        } else if resp.secondary_clicked() {
            Some(crate::widgets::audibility::next_on_secondary(audibility))
        } else {
            None
        };
        if let Some(next) = next {
            let mut state = st;
            state.set_audibility(next);
            if state == yinhe_types::AmMsState::default() {
                am_ms.remove(&key);
            } else {
                am_ms.insert(key, state);
            }
            am_ms_dirty = true;
        }
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
    }

    // 最后一条自动化行：在该行右侧（M/S 旁）显示加号，点击再次添加自动化。
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
