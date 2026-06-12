use std::collections::HashMap;

use eframe::egui;

use yinhe_midi::TrackInfo;
use yinhe_types::{AutomationLane, AutomationTarget, TRACK_PALETTE};

/// CC automation sub-row within a channel folder.
#[derive(Clone, Debug)]
pub struct CcLane {
    /// CC controller number (0-127).
    pub controller: u8,
    /// Display name (e.g. "Volume", "Pan").
    pub name: String,
    /// Whether this sub-row is expanded.
    pub expanded: bool,
}

/// A channel group (Port + Channel combination).
///
/// Tracks sharing the same port and MIDI channel are grouped into one folder.
#[derive(Clone, Debug)]
pub struct ChannelGroup {
    /// Port number (0-15).
    pub port: u8,
    /// MIDI channel (1-16).
    pub channel: u8,
    /// Track indices belonging to this group (sorted).
    pub track_indices: Vec<u16>,
    /// Whether the folder is expanded.
    pub expanded: bool,
    /// CC automation sub-lanes (one per controller with data).
    pub cc_lanes: Vec<CcLane>,
}

/// Build channel groups from track info and automation lanes.
///
/// Groups tracks by port+channel, and attaches relevant CC lanes.
pub fn build_channel_groups(
    track_info: &[TrackInfo],
    automation_lanes: &[AutomationLane],
) -> Vec<ChannelGroup> {
    use std::collections::BTreeMap;

    // Group tracks by (port, channel)
    let mut map: BTreeMap<(u8, u8), Vec<u16>> = BTreeMap::new();
    for ti in track_info {
        map.entry((ti.port, ti.channel))
            .or_default()
            .push(ti.index);
    }

    // Collect unique CC controllers from automation lanes (excluding Velocity, Tempo, PitchBend, RPN)
    let cc_controllers: Vec<u8> = automation_lanes
        .iter()
        .filter_map(|lane| match &lane.target {
            AutomationTarget::CC { controller } => Some(*controller),
            _ => None,
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    map.into_iter()
        .map(|((port, channel), mut indices)| {
            indices.sort();
            let cc_lanes = cc_controllers
                .iter()
                .map(|&controller| CcLane {
                    controller,
                    name: AutomationTarget::CC { controller }.display_name(),
                    expanded: false,
                })
                .collect();
            ChannelGroup {
                port,
                channel,
                track_indices: indices,
                expanded: true,
                cc_lanes,
            }
        })
        .collect()
}

// ── Row types and layout ──────────────────────────────────────────────────

/// Height constants for non-track rows (folder headers, automation sub-rows).
const HEADER_H: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKind {
    Conductor,
    BpmAutomation,
    FolderHeader(usize),
    Track(usize, usize),
    CcAutomation(usize, usize),
}

struct RowLayoutEntry {
    kind: RowKind,
    y_start: f32,
    height: f32,
}

/// Compute the flat row layout from channel groups and conductor state.
fn compute_rows(
    channel_groups: &[ChannelGroup],
    conductor_expanded: bool,
    has_tempo: bool,
    base_row_height: f32,
) -> Vec<RowLayoutEntry> {
    let mut rows = Vec::new();
    let mut y = 0.0;

    // Conductor row
    rows.push(RowLayoutEntry {
        kind: RowKind::Conductor,
        y_start: y,
        height: HEADER_H,
    });
    y += HEADER_H;

    // BPM automation sub-row (when conductor expanded and tempo data exists)
    if conductor_expanded && has_tempo {
        rows.push(RowLayoutEntry {
            kind: RowKind::BpmAutomation,
            y_start: y,
            height: HEADER_H,
        });
        y += HEADER_H;
    }

    // Channel groups
    for (gi, group) in channel_groups.iter().enumerate() {
        // Folder header
        rows.push(RowLayoutEntry {
            kind: RowKind::FolderHeader(gi),
            y_start: y,
            height: HEADER_H,
        });
        y += HEADER_H;

        if group.expanded {
            // Track rows
            for (ti, _track_idx) in group.track_indices.iter().enumerate() {
                rows.push(RowLayoutEntry {
                    kind: RowKind::Track(gi, ti),
                    y_start: y,
                    height: base_row_height,
                });
                y += base_row_height;
            }
            // CC automation sub-rows
            for (ci, cc) in group.cc_lanes.iter().enumerate() {
                if cc.expanded {
                    rows.push(RowLayoutEntry {
                        kind: RowKind::CcAutomation(gi, ci),
                        y_start: y,
                        height: HEADER_H,
                    });
                    y += HEADER_H;
                }
            }
        }
    }

    rows
}

/// Find the first visible row index given scroll_y.
fn find_first_visible(rows: &[RowLayoutEntry], scroll_y: f32) -> usize {
    rows.partition_point(|r| r.y_start + r.height <= scroll_y)
}

/// Port number to letter (0→A, 1→B, ...).
fn port_letter(port: u8) -> char {
    match port {
        0 => 'A', 1 => 'B', 2 => 'C', 3 => 'D',
        4 => 'E', 5 => 'F', 6 => 'G', 7 => 'H',
        _ => '?',
    }
}

/// Draw a small automation preview bar for a CC lane within a rect.
fn draw_cc_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    lane: &AutomationLane,
    channel: u8,
    track_indices: &[u16],
    scroll_x: f32,
    pixels_per_tick: f32,
    combo_width: f32,
) {
    let content_x = rect.min.x + combo_width;
    let content_w = rect.width() - combo_width;
    if content_w < 1.0 {
        return;
    }

    let target = &lane.target;
    let max_val = target.max_value() as f32;
    if max_val <= 0.0 {
        return;
    }

    // Background
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 20, 23));

    // Draw vertical bars for visible events
    let bar_w = 2.0_f32;
    let tick_start = (scroll_x / pixels_per_tick).max(0.0) as u32;
    let tick_end = ((scroll_x + content_w) / pixels_per_tick).max(0.0) as u32;

    let events = lane.events_in_range(tick_start, tick_end);
    for evt in events {
        // Only show events for this channel's tracks
        if evt.channel != channel || !track_indices.contains(&evt.track) {
            continue;
        }
        let val = evt.value as f32;
        let bar_h = ((val + 1.0) / (max_val + 1.0)) * rect.height();
        let x = content_x + (evt.tick as f32 * pixels_per_tick) - scroll_x;
        if x + bar_w < content_x || x > content_x + content_w {
            continue;
        }
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(x, rect.max.y - bar_h),
            egui::vec2(bar_w, bar_h),
        );
        let color = TRACK_PALETTE[evt.track as usize % TRACK_PALETTE.len()];
        let color32 = egui::Color32::from_rgb(
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );
        painter.rect_filled(bar_rect, 0.0, color32);
    }
}

/// Draw the BPM automation preview within a rect.
fn draw_bpm_preview(
    painter: &egui::Painter,
    rect: egui::Rect,
    tempo_lane: &AutomationLane,
    scroll_x: f32,
    pixels_per_tick: f32,
    combo_width: f32,
) {
    let content_x = rect.min.x + combo_width;
    let content_w = rect.width() - combo_width;
    if content_w < 1.0 {
        return;
    }

    // Background
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(20, 20, 23));

    let max_val = tempo_lane.target.max_value() as f32;
    if max_val <= 0.0 {
        return;
    }

    let bar_w = 2.0_f32;
    let tick_start = (scroll_x / pixels_per_tick).max(0.0) as u32;
    let tick_end = ((scroll_x + content_w) / pixels_per_tick).max(0.0) as u32;

    let events = tempo_lane.events_in_range(tick_start, tick_end);
    for evt in events {
        let val = evt.value as f32;
        let bar_h = ((val + 1.0) / (max_val + 1.0)) * rect.height();
        let x = content_x + (evt.tick as f32 * pixels_per_tick) - scroll_x;
        if x + bar_w < content_x || x > content_x + content_w {
            continue;
        }
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(x, rect.max.y - bar_h),
            egui::vec2(bar_w, bar_h),
        );
        painter.rect_filled(
            bar_rect,
            0.0,
            egui::Color32::from_rgb(100, 180, 255),
        );
    }

    // Line chart overlay
    if events.len() >= 2 {
        let mut points = Vec::new();
        for evt in events {
            let val = evt.value as f32;
            let x = content_x + (evt.tick as f32 * pixels_per_tick) - scroll_x;
            let y = rect.max.y - ((val + 1.0) / (max_val + 1.0)) * rect.height();
            points.push(egui::pos2(x, y));
        }
        // Draw line segments
        let line_color = egui::Color32::from_rgb(100, 180, 255).gamma_multiply(0.6);
        for w in points.windows(2) {
            painter.line_segment([w[0], w[1]], egui::Stroke::new(1.0, line_color));
        }
    }
}

// ── Main entry point ──────────────────────────────────────────────────────

/// Render the track panel with channel folders, conductor track, and automation sub-rows.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    track_info: &[TrackInfo],
    _track_visible: &mut [bool],
    track_selected: &mut Option<u16>,
    pc_map: &HashMap<u8, u8>,
    row_height: &mut f32,
    scroll_y: &mut f32,
    channel_groups: &mut [ChannelGroup],
    conductor_expanded: &mut bool,
    tempo_lane: &Option<AutomationLane>,
) {
    let panel_rect = ui.max_rect();
    let panel_w = panel_rect.width();
    let panel_h = panel_rect.height();

    if panel_w < 1.0 || panel_h < 1.0 {
        return;
    }

    let show_details = *row_height >= 30.0;

    // Build row layout
    let rows = compute_rows(
        channel_groups,
        *conductor_expanded,
        tempo_lane.is_some(),
        *row_height,
    );

    let total_height = rows.last().map(|r| r.y_start + r.height).unwrap_or(0.0);
    let max_scroll = (total_height - panel_h).max(0.0);
    *scroll_y = scroll_y.clamp(0.0, max_scroll);

    // Visible row range
    let first = find_first_visible(&rows, *scroll_y);
    let mut last = first;
    while last < rows.len() && rows[last].y_start < *scroll_y + panel_h {
        last += 1;
    }

    let painter = ui.painter().clone();
    let interact_id = egui::Id::new("track_panel_area");
    let resp = ui.interact(panel_rect, interact_id, egui::Sense::click_and_drag());
    let hover_pos = ui.input(|i| i.pointer.hover_pos()).unwrap_or_default();

    // We need pixels_per_tick for automation previews — use a default since
    // the arrangement view syncs this via scroll_x. For the track panel
    // we derive it from the arrangement view state if available, or use a sensible default.
    let ppt = 0.08_f32; // default, will be overridden by arrange.rs if needed
    let scroll_x = 0.0_f32; // track panel doesn't have horizontal scroll

    for ri in first..last {
        let entry = &rows[ri];
        let y = panel_rect.min.y + entry.y_start - *scroll_y;
        let row_rect = egui::Rect::from_min_size(
            egui::pos2(panel_rect.min.x, y),
            egui::vec2(panel_w, entry.height),
        );

        if y > panel_rect.max.y || y + entry.height < panel_rect.min.y {
            continue;
        }

        match entry.kind {
            RowKind::Conductor => {
                // ── Conductor row ──
                let hovered = row_rect.contains(hover_pos);
                painter.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(35, 35, 40));
                if hovered {
                    painter.rect_filled(
                        row_rect,
                        0.0,
                        egui::Color32::WHITE.gamma_multiply(0.05),
                    );
                }

                // Blue accent bar
                let badge_w = 8.0_f32;
                let badge_rect = egui::Rect::from_min_size(
                    row_rect.min,
                    egui::vec2(badge_w, entry.height),
                );
                painter.rect_filled(badge_rect, 0.0, egui::Color32::from_rgb(100, 180, 255));

                // Expand/collapse triangle
                let triangle_x = badge_rect.max.x + 6.0;
                let triangle_cy = row_rect.center().y;
                let tri_size = 5.0_f32;
                let tri_color = egui::Color32::WHITE.gamma_multiply(0.7);
                if *conductor_expanded {
                    // Down-pointing triangle ▼
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(triangle_x - tri_size, triangle_cy - tri_size * 0.6),
                            egui::pos2(triangle_x + tri_size, triangle_cy - tri_size * 0.6),
                            egui::pos2(triangle_x, triangle_cy + tri_size * 0.6),
                        ],
                        tri_color,
                        egui::Stroke::NONE,
                    ));
                } else {
                    // Right-pointing triangle ▶
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(triangle_x - tri_size * 0.6, triangle_cy - tri_size),
                            egui::pos2(triangle_x + tri_size * 0.6, triangle_cy),
                            egui::pos2(triangle_x - tri_size * 0.6, triangle_cy + tri_size),
                        ],
                        tri_color,
                        egui::Stroke::NONE,
                    ));
                }

                // Label
                let text_x = triangle_x + tri_size + 8.0;
                let font = egui::FontId::proportional((entry.height * 0.50).clamp(9.0, 13.0));
                painter.text(
                    egui::pos2(text_x, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    "Conductor",
                    font,
                    egui::Color32::WHITE.gamma_multiply(0.85),
                );

                // Click to toggle
                if resp.clicked() && hovered {
                    *conductor_expanded = !*conductor_expanded;
                }
            }

            RowKind::BpmAutomation => {
                // ── BPM automation sub-row ──
                let indent_x = panel_rect.min.x + 24.0;
                let label_w = 40.0_f32;
                let label_rect = egui::Rect::from_min_size(
                    egui::pos2(indent_x, y),
                    egui::vec2(label_w, entry.height),
                );
                let preview_rect = egui::Rect::from_min_size(
                    egui::pos2(indent_x + label_w, y),
                    egui::vec2(panel_w - indent_x - label_w, entry.height),
                );

                // Background
                painter.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(22, 22, 25));

                // Label
                let font = egui::FontId::proportional((entry.height * 0.45).clamp(8.0, 11.0));
                painter.text(
                    label_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "BPM",
                    font,
                    egui::Color32::from_rgb(100, 180, 255).gamma_multiply(0.8),
                );

                // Preview
                if let Some(lane) = tempo_lane {
                    draw_bpm_preview(&painter, preview_rect, lane, scroll_x, ppt, 0.0);
                }
            }

            RowKind::FolderHeader(gi) => {
                // ── Channel folder header ──
                let group = &channel_groups[gi];
                let hovered = row_rect.contains(hover_pos);
                painter.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(30, 30, 34));
                if hovered {
                    painter.rect_filled(
                        row_rect,
                        0.0,
                        egui::Color32::WHITE.gamma_multiply(0.04),
                    );
                }

                // Color badge — use the first track's color
                let first_track = group.track_indices.first().copied().unwrap_or(0);
                let color = TRACK_PALETTE[first_track as usize % TRACK_PALETTE.len()];
                let color32 = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                let badge_w = 8.0_f32;
                let badge_rect = egui::Rect::from_min_size(
                    row_rect.min,
                    egui::vec2(badge_w, entry.height),
                );
                painter.rect_filled(badge_rect, 0.0, color32);

                // Expand/collapse triangle
                let triangle_x = badge_rect.max.x + 6.0;
                let triangle_cy = row_rect.center().y;
                let tri_size = 5.0_f32;
                let tri_color = egui::Color32::WHITE.gamma_multiply(0.7);
                if group.expanded {
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(triangle_x - tri_size, triangle_cy - tri_size * 0.6),
                            egui::pos2(triangle_x + tri_size, triangle_cy - tri_size * 0.6),
                            egui::pos2(triangle_x, triangle_cy + tri_size * 0.6),
                        ],
                        tri_color,
                        egui::Stroke::NONE,
                    ));
                } else {
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(triangle_x - tri_size * 0.6, triangle_cy - tri_size),
                            egui::pos2(triangle_x + tri_size * 0.6, triangle_cy),
                            egui::pos2(triangle_x - tri_size * 0.6, triangle_cy + tri_size),
                        ],
                        tri_color,
                        egui::Stroke::NONE,
                    ));
                }

                // Channel label (e.g. "A01")
                let text_x = triangle_x + tri_size + 8.0;
                let port_ch = format!("{}{:02}", port_letter(group.port), group.channel);
                let font = egui::FontId::proportional((entry.height * 0.50).clamp(9.0, 13.0));
                painter.text(
                    egui::pos2(text_x, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &port_ch,
                    font.clone(),
                    egui::Color32::WHITE.gamma_multiply(0.85),
                );

                // Track count
                let count_text = format!("{} tracks", group.track_indices.len());
                let count_font = egui::FontId::proportional((entry.height * 0.40).clamp(8.0, 11.0));
                painter.text(
                    egui::pos2(text_x + 40.0, row_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    &count_text,
                    count_font,
                    egui::Color32::GRAY,
                );

                // Click to toggle
                if resp.clicked() && hovered {
                    channel_groups[gi].expanded = !channel_groups[gi].expanded;
                }
            }

            RowKind::Track(gi, ti_local) => {
                // ── Track row (indented under folder) ──
                let group = &channel_groups[gi];
                let track_idx = group.track_indices[ti_local] as usize;
                if track_idx >= track_info.len() {
                    continue;
                }
                let ti = &track_info[track_idx];
                let hovered = row_rect.contains(hover_pos);
                let selected = *track_selected == Some(ti.index);

                // Indent
                let indent = 24.0_f32;
                let indented_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + indent, y),
                    egui::vec2(panel_w - indent, entry.height),
                );

                // Background
                if selected {
                    painter.rect_filled(row_rect, 0.0, ui.visuals().selection.bg_fill);
                } else if hovered {
                    painter.rect_filled(
                        row_rect,
                        0.0,
                        egui::Color32::WHITE.gamma_multiply(0.03),
                    );
                } else {
                    // Alternating background
                    let bg = if ti_local % 2 == 0 {
                        egui::Color32::from_rgb(25, 25, 28)
                    } else {
                        egui::Color32::from_rgb(28, 28, 31)
                    };
                    painter.rect_filled(row_rect, 0.0, bg);
                }

                // Badge color strip
                let color = TRACK_PALETTE[track_idx % TRACK_PALETTE.len()];
                let color32 = egui::Color32::from_rgb(
                    (color[0] * 255.0) as u8,
                    (color[1] * 255.0) as u8,
                    (color[2] * 255.0) as u8,
                );
                let badge_w = 8.0_f32;
                let badge_rect = egui::Rect::from_min_size(
                    indented_rect.min,
                    egui::vec2(badge_w, entry.height),
                );
                painter.rect_filled(badge_rect, 0.0, color32);

                // Text
                let text_x = badge_rect.max.x + 6.0;
                let track_num_text = format!("{:03}", ti.index + 1);

                if show_details {
                    let port_ch = format!("{}{:02}", port_letter(ti.port), ti.channel);
                    let font =
                        egui::FontId::proportional((entry.height * 0.25).clamp(8.0, 13.0));

                    // Row 1: track number + port/channel + note count
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + entry.height * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &track_num_text,
                        font.clone(),
                        egui::Color32::WHITE.gamma_multiply(0.85),
                    );
                    painter.text(
                        egui::pos2(text_x + 32.0, badge_rect.min.y + entry.height * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &port_ch,
                        font.clone(),
                        egui::Color32::WHITE.gamma_multiply(0.85),
                    );
                    let global_ch = ti.port * 16 + (ti.channel - 1);
                    let mut detail = format!("{} notes", ti.note_count);
                    if let Some(pc) = pc_map.get(&global_ch) {
                        detail.push_str(&format!(" | PC:{}", pc));
                    }
                    let detail_font =
                        egui::FontId::proportional((entry.height * 0.20).clamp(8.0, 11.0));
                    painter.text(
                        egui::pos2(text_x + 62.0, badge_rect.min.y + entry.height * 0.30),
                        egui::Align2::LEFT_CENTER,
                        &detail,
                        detail_font,
                        egui::Color32::GRAY,
                    );

                    // Row 2: track name
                    let name_font =
                        egui::FontId::proportional((entry.height * 0.25).clamp(9.0, 13.0));
                    painter.text(
                        egui::pos2(text_x, badge_rect.min.y + entry.height * 0.70),
                        egui::Align2::LEFT_CENTER,
                        &ti.name,
                        name_font,
                        egui::Color32::WHITE.gamma_multiply(0.85),
                    );
                } else {
                    // Single-row: track number + name
                    let font =
                        egui::FontId::proportional((entry.height * 0.45).clamp(8.0, 14.0));
                    painter.text(
                        egui::pos2(text_x, badge_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &track_num_text,
                        font,
                        egui::Color32::WHITE.gamma_multiply(0.85),
                    );
                    let name_font =
                        egui::FontId::proportional((entry.height * 0.45).clamp(8.0, 14.0));
                    painter.text(
                        egui::pos2(text_x + 40.0, badge_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &ti.name,
                        name_font,
                        egui::Color32::WHITE.gamma_multiply(0.85),
                    );
                }

                // Click to select track
                if resp.clicked() && hovered {
                    *track_selected = Some(ti.index);
                }
            }

            RowKind::CcAutomation(gi, ci) => {
                // ── CC automation sub-row ──
                let group = &channel_groups[gi];
                let cc = &group.cc_lanes[ci];
                let indent = 24.0_f32;

                // Background
                painter.rect_filled(row_rect, 0.0, egui::Color32::from_rgb(22, 22, 25));

                // Indent + label
                let label_w = 48.0_f32;
                let label_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + indent, y),
                    egui::vec2(label_w, entry.height),
                );
                let _preview_rect = egui::Rect::from_min_size(
                    egui::pos2(panel_rect.min.x + indent + label_w, y),
                    egui::vec2(panel_w - indent - label_w, entry.height),
                );

                // Label — abbreviated CC name
                let font = egui::FontId::proportional((entry.height * 0.40).clamp(8.0, 10.0));
                let short_name = cc_short_name(cc.controller);
                painter.text(
                    label_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    short_name,
                    font,
                    egui::Color32::GRAY.gamma_multiply(0.8),
                );

                // Find the matching automation lane and draw preview
                // We'll look for it in the automation_lanes by target
                // Since we don't have direct access, we pass the controller info
                // The caller should provide automation_lanes, but for now we use
                // a simplified approach: draw a placeholder bar
                // TODO: pass automation_lanes to show() for real preview rendering
            }
        }
    }

    // ── Interaction: scroll ──
    if resp.hovered() {
        let scroll_delta = ui.input(|i| i.smooth_scroll_delta);
        if scroll_delta.y.abs() > 0.5 {
            *scroll_y = (*scroll_y - scroll_delta.y).max(0.0);
        }
    }
}

/// Short display name for a CC controller (for automation sub-row labels).
fn cc_short_name(cc: u8) -> &'static str {
    match cc {
        0 => "Bank",
        1 => "Mod",
        7 => "Vol",
        10 => "Pan",
        11 => "Expr",
        64 => "Sust",
        71 => "Res",
        72 => "Rel",
        73 => "Atk",
        74 => "Cut",
        _ => "CC",
    }
}
