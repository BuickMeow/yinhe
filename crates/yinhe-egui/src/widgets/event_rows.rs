//! 全局事件行：固定显示在时间标尺与音轨/钢琴卷帘之间。
//!
//! 行本身不是音轨行，不进入 `ArRowLayout`，不随 track panel 的 scroll_y 滚动。
//! v1：Marker 支持完整的点击/拖动/重命名/删除；其余行走只读展示 + 点击定位播放头。

use eframe::egui;

use yinhe_types::{
    ChordEvent, EventRowKind, EventRowsState, KeySigEvent, LyricsEvent, NOTE_NAMES, TimeSigEvent,
};

use crate::widgets::time_ruler::{MarkerEdit, TimeRulerView};
use yinhe_editor_core::PlaybackState;

/// 全局事件行需要的数据。AR / 通道 AR / PR 共用。
pub(crate) struct EventRowsData<'a> {
    pub markers: &'a [yinhe_types::MarkerEvent],
    pub time_sig: &'a [TimeSigEvent],
    pub key_sig: &'a [KeySigEvent],
    pub chords: &'a [ChordEvent],
    pub lyrics: &'a [LyricsEvent],
}

pub(crate) struct EventRowsOutcome {
    pub marker_edit: Option<MarkerEdit>,
    pub jumped: bool,
    /// LOOP 行拖动或显隐发生变化（调用方需要把状态推给音频引擎）。
    pub loop_changed: bool,
}

/// 绘制并交互固定事件行。
///
/// `full_rect`：包含左侧行头列与右侧时间内容区的完整区域；
/// `view`：负责 tick↔px 换算；`snap`：点击/拖动时的量化。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &mut EventRowsState,
    playback: &mut PlaybackState,
    data: EventRowsData<'_>,
    view: &mut impl TimeRulerView,
    full_rect: egui::Rect,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    snap: &impl Fn(f64) -> f64,
    id_salt: &str,
    cursor_tick: &mut Option<f64>,
) -> EventRowsOutcome {
    // LOOP 行是走带循环的镜像：循环开启自动出现；关闭自动隐藏。
    state.sync_loop(playback.loop_enabled());
    if state.visible_count() == 0 || full_rect.height() <= 0.0 {
        return EventRowsOutcome {
            marker_edit: None,
            jumped: false,
            loop_changed: false,
        };
    }

    let painter = ui.painter_at(full_rect);
    painter.rect_filled(full_rect, 0.0, crate::theme::control_bg());

    let content_left = (full_rect.min.x + view.content_left()).min(full_rect.max.x);
    let header_rect =
        egui::Rect::from_min_max(full_rect.min, egui::pos2(content_left, full_rect.max.y));
    painter.rect_filled(header_rect, 0.0, crate::theme::control_bg());

    let mut outcome = EventRowsOutcome {
        marker_edit: None,
        jumped: false,
        loop_changed: false,
    };
    for (row_idx, kind) in state.visible_rows().enumerate() {
        let y0 = full_rect.min.y + row_idx as f32 * state.row_height;
        let y1 = (y0 + state.row_height).min(full_rect.max.y);
        if y1 <= y0 {
            break;
        }
        let full_row = egui::Rect::from_min_max(
            egui::pos2(full_rect.min.x, y0),
            egui::pos2(full_rect.max.x, y1),
        );
        let header = egui::Rect::from_min_max(
            egui::pos2(full_rect.min.x, y0),
            egui::pos2(content_left, y1),
        );
        let content = egui::Rect::from_min_max(
            egui::pos2(content_left, y0),
            egui::pos2(full_rect.max.x, y1),
        );

        // 与时间标尺（track_bg）区分：事件行内容用 app_bg，行头用 control_bg。
        let content_bg = if row_idx.is_multiple_of(2) {
            crate::theme::app_bg()
        } else {
            crate::theme::control_bg()
        };
        painter.rect_filled(full_row, 0.0, content_bg);
        painter.rect_filled(header, 0.0, crate::theme::control_bg());
        painter.hline(
            full_row.x_range(),
            y1 - 0.5,
            egui::Stroke::new(1.0, crate::theme::line_fg()),
        );
        painter.text(
            egui::pos2(header.min.x + 8.0, header.center().y),
            egui::Align2::LEFT_CENTER,
            kind.label(),
            egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
            crate::theme::text_muted(),
        );

        // 行空白处点击 = 定位播放头。Marker 行在 marker_band 之前注册，
        // 标签交互后注册，从而标签优先。
        let row_resp = ui.interact(
            content,
            ui.scope_id().with((id_salt, "row", kind.label())),
            egui::Sense::click(),
        );

        let mut pointer_over_label = false;
        if kind == EventRowKind::Loop {
            let (start_tick, end_tick) = playback.loop_range();
            let x0 = (content.min.x + view.tick_to_main_px(start_tick as f64))
                .clamp(content.min.x, content.max.x);
            let x1 = (content.min.x + view.tick_to_main_px(end_tick as f64))
                .clamp(content.min.x, content.max.x);
            let band = egui::Rect::from_min_max(
                egui::pos2(x0, content.min.y + 2.0),
                egui::pos2(x1.max(x0 + 1.0), content.max.y - 2.0),
            );
            let accent = crate::theme::accent_active();
            painter.rect_filled(band, 2.0, accent.gamma_multiply(0.18));
            painter.rect_stroke(
                band,
                2.0,
                egui::Stroke::new(1.0, accent),
                egui::StrokeKind::Inside,
            );

            let handle_w = 5.0;
            let left = egui::Rect::from_min_max(
                egui::pos2(band.min.x - handle_w, band.min.y),
                egui::pos2(band.min.x + handle_w, band.max.y),
            );
            let right = egui::Rect::from_min_max(
                egui::pos2(band.max.x - handle_w, band.min.y),
                egui::pos2(band.max.x + handle_w, band.max.y),
            );

            for (which, handle_rect) in [("left", left), ("right", right)] {
                let resp = ui.interact(
                    handle_rect.expand2(egui::vec2(2.0, 0.0)),
                    ui.scope_id().with((id_salt, "loop_handle", which)),
                    egui::Sense::drag(),
                );
                if resp.hovered() || resp.dragged() {
                    pointer_over_label = true;
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                if resp.dragged()
                    && let Some(pos) = resp.interact_pointer_pos()
                {
                    let tick = snap(view.x_to_tick(pos.x - full_rect.min.x))
                        .clamp(0.0, u32::MAX as f64) as u32;
                    let (mut s0, mut s1) = playback.loop_range();
                    if which == "left" {
                        s0 = tick.min(s1.saturating_sub(1));
                    } else {
                        s1 = tick.max(s0.saturating_add(1));
                    }
                    if playback.set_loop_range(s0, s1) {
                        outcome.loop_changed = true;
                        ui.ctx().request_repaint();
                    }
                }
            }
            // 点击循环带内不跳动播放头（拖拽手柄优先）。
            if band.contains(ui.input(|i| i.pointer.hover_pos().unwrap_or(egui::Pos2::ZERO))) {
                pointer_over_label = true;
            }
        } else if kind == EventRowKind::Marker {
            let marker = crate::widgets::time_ruler::marker_band(
                ui,
                content,
                view,
                data.markers,
                tpb,
                default_num,
                default_den,
                time_sig_events,
                content.center().y,
                snap,
                id_salt,
                cursor_tick,
            );
            pointer_over_label = marker.pointer_over_label;
            if marker.edit.is_some() {
                outcome.marker_edit = marker.edit;
            }
            outcome.jumped |= marker.jumped;
        } else {
            match kind {
                EventRowKind::Meter => paint_text_events(
                    &painter,
                    content,
                    view,
                    data.time_sig.iter().map(|e| (e.tick, format_meter(e))),
                    crate::theme::text_secondary(),
                ),
                EventRowKind::Key => paint_text_events(
                    &painter,
                    content,
                    view,
                    data.key_sig.iter().map(|e| (e.tick, format_key(e))),
                    crate::theme::text_secondary(),
                ),
                EventRowKind::Chord => paint_text_events(
                    &painter,
                    content,
                    view,
                    data.chords.iter().map(|e| (e.tick, e.text.clone())),
                    crate::theme::text_muted(),
                ),
                EventRowKind::Lyrics => paint_text_events(
                    &painter,
                    content,
                    view,
                    data.lyrics.iter().map(|e| (e.tick, e.text.clone())),
                    crate::theme::text_muted(),
                ),
                EventRowKind::Marker => {}
                EventRowKind::Loop => {}
            }
        }

        if row_resp.clicked()
            && !pointer_over_label
            && let Some(pos) = row_resp.interact_pointer_pos()
        {
            let tick = view.x_to_tick(pos.x - full_rect.min.x);
            *cursor_tick = Some(snap(tick).max(0.0));
            ui.ctx().request_repaint();
            outcome.jumped = true;
        }
    }

    outcome
}

fn paint_text_events(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &impl TimeRulerView,
    events: impl Iterator<Item = (u32, String)>,
    text_color: egui::Color32,
) {
    let mut last_text_x = f32::NEG_INFINITY;
    for (tick, text) in events {
        let x = rect.min.x + view.tick_to_main_px(tick as f64);
        if x < rect.min.x {
            continue;
        }
        if x > rect.max.x {
            break;
        }
        painter.line_segment(
            [
                egui::pos2(x, rect.min.y + 2.0),
                egui::pos2(x, rect.max.y - 2.0),
            ],
            egui::Stroke::new(1.0, crate::theme::line_fg()),
        );
        if text.is_empty() || x - last_text_x < 38.0 {
            continue;
        }
        painter.text(
            egui::pos2(x + 4.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT),
            text_color,
        );
        last_text_x = x;
    }
}

fn format_meter(e: &TimeSigEvent) -> String {
    format!("{}/{}", e.numerator, 1u32 << e.denominator.min(5))
}

fn format_key(e: &KeySigEvent) -> String {
    format!(
        "{} {}",
        NOTE_NAMES[(e.root % 12) as usize],
        e.scale.english_name()
    )
}

/// AR 左上角（标尺与轨道面板交界）的 Events 下拉按钮。
///
/// 勾选控制事件行显示；默认全部关闭，不占用音轨区域高度。
pub(crate) fn controls(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    state: &mut EventRowsState,
    playback: &mut PlaybackState,
    tpb: u32,
) {
    let button_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(6.0, 1.0),
        egui::vec2(
            (rect.width() - 12.0).max(42.0),
            (rect.height() - 4.0).max(16.0),
        ),
    );
    let resp = ui.put(
        button_rect,
        egui::Button::new(
            egui::RichText::new("Events ▾")
                .size(crate::theme::SMALL_LABEL_FONT)
                .color(crate::theme::text_secondary()),
        )
        .frame(false),
    );
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(120.0);
            for kind in EventRowKind::ALL {
                let mut visible = if kind == EventRowKind::Loop {
                    playback.loop_enabled()
                } else {
                    state.is_visible(kind)
                };
                if ui.checkbox(&mut visible, kind.label()).changed() {
                    if kind == EventRowKind::Loop {
                        if visible {
                            // Events 菜单直接开启时给一个四小节默认区间；
                            // 播放菜单/走带按钮会用光标所在小节覆盖它。
                            if playback.loop_range().1 <= playback.loop_range().0 {
                                let _ = playback.set_loop_range(0, tpb.max(1) * 16);
                            }
                            let _ = playback.set_loop_enabled(true);
                        } else {
                            let _ = playback.set_loop_enabled(false);
                        }
                        state.sync_loop(playback.loop_enabled());
                    } else {
                        state.set_visible(kind, visible);
                    }
                }
            }
        });
}
