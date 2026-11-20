use std::collections::HashSet;
use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;

use yinhe_core::{TrackInfo, TrackKind};
use yinhe_types::{ArRow, ArRowLayout, AutomationLane, AutomationTarget};

use yinhe_editor_core::document::TrackOverride;

pub(crate) mod badge;
pub(crate) mod hover;
mod interaction;
pub(crate) mod render;
mod types;
pub(crate) use types::TrackAction;

/// 色带图标（chevron / 加号）命中区高 16px，其底边距行底（色带底）1px，
/// 因此中心距行底 16 / 2 + 1 = 9px。
const BADGE_ICON_BOTTOM_INSET: f32 = 9.0;

/// 每轨电平表：条宽 / 贴边距（上右下相同）（px）。
const LEVEL_METER_W: f32 = 8.0;
const LEVEL_METER_MARGIN: f32 = 3.0;

/// AR 轨道听感开关的 before 快照（egui 临时数据，跨帧保存）。
fn audibility_undo_id() -> egui::Id {
    egui::Id::unique("arr_track_audibility_undo_before")
}

fn ensure_audibility_undo_snapshot(ui: &egui::Ui, overrides: &[TrackOverride]) {
    if ui.data(|d| {
        d.get_temp::<Vec<TrackOverride>>(audibility_undo_id())
            .is_some()
    }) {
        return;
    }
    ui.data_mut(|d| d.insert_temp(audibility_undo_id(), overrides.to_vec()));
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
    info_content: &mut Option<crate::right_panel::InfoContent>,
    // 行布局（含展开的 AM 子行；与 GPU 区共用同一 ArRowLayout）。
    row_layout: &ArRowLayout,
    // 音轨数据（AM 子行画 lane 名 / 右键菜单判断已有 lane）。
    tracks: &[Arc<yinhe_core::TrackData>],
    // 每轨 LC 的显示标签（LC01..），由调用方按 `mixer.lc_display_map` 算好。
    // 每轨 AM 展开状态（chevron 点击直接翻转，纯视图状态不进 undo）。
    arr_am_expanded: &mut [bool],
    // 被选中的 AM lane（(音轨索引, target)）；点主行清空。
    am_lane_selected: &mut HashSet<(u16, AutomationTarget)>,
    // AR 自动化 lane 的听感状态（子行圆点切换，切后需重载音频）。
    am_ms: &mut std::collections::HashMap<(u16, AutomationTarget), yinhe_types::AmMsState>,
    // 每轨电平表显示值 `((L,R), (holdL,holdR))`（索引 = 音轨；仅播放时非零）。
    track_meters: &[crate::widgets::meter::StereoMeter],
    // 每轨 LC / 音频 LC 节点的 automation。
    track_mixer_lanes: &[Vec<(yinhe_types::ChannelId, Vec<yinhe_types::AutomationLane>)>],
    // 编辑/播放光标 tick（自动化表显示光标处的 lane 值）。
    cursor_tick: Option<f64>,
    // Conductor 的 Tempo lane（Conductor 行自动化表）。
    tempo_lane: &AutomationLane,
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
    // 末尾多一行「添加音轨」占位行：与移动端一致，按钮跟在最后一个轨道后面。
    let scroll_rows = total_rows + 1;
    let max_scroll = (scroll_rows as f32 * *row_height - panel_h).max(0.0);
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
    let drag_id = ui.scope_id().with("track_panel_drag");
    let mut drag: Option<crate::widgets::reorder::DragReorder> =
        ui.data_mut(|d| d.get_temp(drag_id)).unwrap_or_default();

    // Visible row range（含展开的 AM 子行；行高均匀 = 音轨行高）
    let lh = *row_height;
    let (first_row, last_row) = render::visible_range(*scroll_y, panel_h, lh, scroll_rows);

    let painter = ui.painter().clone();
    let any_track_solo = track_overrides
        .iter()
        .any(|o| o.audibility() == yinhe_types::Audibility::Soloed);
    let mut audio_dirty = false;
    // AM lane 听感状态变化 → 调用方重载音频（带 am_ms 旁通）。
    let mut am_ms_dirty = false;

    // 交替行条纹：着色行（偶数行号）与 GPU 区同源颜色，不透明；奇数行用 app_bg 打底。
    // 按全局行号奇偶（AM 子行也参与）：展开奇数条 lane 会错位后续音轨的斑纹。
    let lane_even = crate::theme::stripe_bg();

    let interact_id = egui::Id::unique("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());

    // 主行矩形（含视口外/隐藏行，保证拖拽插入索引全局正确；AM 子行不参与排序）；
    // 仅可视行渲染。
    let item_rects =
        render::build_item_rects(row_layout, panel_rect, panel_w, lh, *scroll_y, num_tracks);

    // 行内按钮命中区：按下在这些区域只操作按钮，不触发轨道排序。
    let mut no_drag_rects: Vec<egui::Rect> = Vec::new();

    // 兄弟轨道联动：鼠标落在某音轨的主行/任意自动化行上，视为悬停整个
    // 「兄弟轨道」（该音轨及其全部自动化轨），它们的 chevron/加号一并显示。
    // 成熟实现：走 pointer_hits + pointer_over_popup，感知 Foreground popup 遮挡
    let hover_track = hover::hover_track(ui, panel_rect, row_layout, *scroll_y, lh);
    let pointer_pos = ui.input(|i| i.pointer.hover_pos());

    for row in first_row..last_row {
        let y = panel_rect.min.y + row as f32 * lh - *scroll_y;
        let row_rect =
            egui::Rect::from_min_size(egui::pos2(panel_rect.min.x, y), egui::vec2(panel_w, lh));
        // 最后一个轨道之后的占位行：整行都是「添加音轨」按钮。
        if row == total_rows {
            render::draw_add_track_row(ui, &painter, row_rect, row);
            continue;
        }
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

        if matches!(row_hit, ArRow::Track(_)) && pointer_pos.is_some_and(|p| row_rect.contains(p)) {
            crate::widgets::hint::set(ui.ctx(), t!("hint.arrange.track_row"));
        }

        // ── AM 行：复用普通轨的行样式（绘制细节见 render::draw_automation_row）。
        if let ArRow::Automation(track, sub) = row_hit {
            let track_data = tracks.get(track);
            let lane = track_data
                .and_then(|t| t.automation_lanes.get(sub))
                .or_else(|| {
                    let t = track_data?;
                    let track_lanes = t.automation_lanes.len();
                    let (node, lane_idx) = super::am_lanes::track_mixer_lane_at(
                        track_mixer_lanes.get(track)?,
                        track_lanes,
                        sub,
                    )?;
                    track_mixer_lanes
                        .get(track)?
                        .iter()
                        .find(|(id, _)| *id == node)
                        .and_then(|(_, lanes)| lanes.get(lane_idx))
                });
            let Some(lane) = lane else {
                continue;
            };
            let (dirty, acts) = render::draw_automation_row(
                ui,
                &painter,
                row_rect,
                row,
                lh,
                track,
                sub,
                lane,
                track_info,
                track_colors,
                tracks,
                am_lane_selected,
                am_ms,
                show_details,
                hover_track,
                pointer_pos,
                cursor_tick,
                &mut no_drag_rects,
                lane_even,
                any_track_solo,
                track_overrides
                    .get(track)
                    .map(|o| o.audibility())
                    .unwrap_or_default(),
            );
            am_ms_dirty |= dirty;
            actions.extend(acts);
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
        let is_folder = tracks.get(idx).is_some_and(|t| t.kind == TrackKind::Folder);
        let folder_depth = tracks.get(idx).map(|t| t.folder_depth).unwrap_or(0);
        let indent = f32::from(folder_depth) * 14.0;
        let content_rect = egui::Rect::from_min_size(
            row_rect.min + egui::vec2(indent, 0.0),
            egui::vec2((row_rect.width() - indent).max(80.0), row_rect.height()),
        );
        let badge_rect = render::draw_badge(&painter, content_rect, lh, color32);

        if !is_conductor {
            let expanded = arr_am_expanded.get(idx).copied().unwrap_or(false);
            let has_any_lane = tracks
                .get(idx)
                .is_some_and(|t| !t.automation_lanes.is_empty())
                || track_mixer_lanes
                    .get(idx)
                    .is_some_and(|lanes| !lanes.is_empty());
            render::draw_badge_icons(
                ui,
                &painter,
                badge_rect,
                lh,
                idx,
                color,
                is_folder,
                tracks.get(idx).is_some_and(|t| t.collapsed),
                expanded,
                has_any_lane,
                hover_track == Some(idx),
                pointer_pos,
                &mut no_drag_rects,
                &mut actions,
                arr_am_expanded,
            );
        }

        let (track_audio_dirty, track_actions) = render::draw_track_row(
            ui,
            &painter,
            row_rect,
            badge_rect,
            lh,
            idx,
            ti,
            tracks,
            is_conductor,
            is_folder,
            show_details,
            track_overrides,
            track_meters,
            tempo_lane,
            cursor_tick,
            color32,
            &mut no_drag_rects,
            any_track_solo,
        );
        audio_dirty |= track_audio_dirty;
        actions.extend(track_actions);
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
        &no_drag_rects,
        &item_rects,
        &mut drag,
        drag_id,
        info_content,
        request_pianoroll,
        tracks,
        &mut actions,
        max_scroll,
    );

    // 听感开关手势结束（松开鼠标）：把整段点击合并成一条 undo。
    if ui.data(|d| {
        d.get_temp::<Vec<TrackOverride>>(audibility_undo_id())
            .is_some()
    }) && !ui.input(|i| i.pointer.primary_down())
        && let Some(before) =
            ui.data_mut(|d| d.remove_temp::<Vec<TrackOverride>>(audibility_undo_id()))
        && before != track_overrides
    {
        actions.push(TrackAction::CommitMuteSolo {
            before,
            after: track_overrides.to_vec(),
        });
    }

    (audio_dirty, am_ms_dirty, actions)
}
