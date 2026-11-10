//! 全局事件行：固定显示在时间标尺与音轨/钢琴卷帘之间。
//!
//! 行本身不是音轨行，不进入 `ArRowLayout`，不随 track panel 的 scroll_y 滚动。
//! v1：Marker 支持完整的点击/拖动/重命名/删除；其余行走只读展示 + 点击定位播放头。

use eframe::egui;

use yinhe_types::{
    ChordEvent, EventRowKind, EventRowsState, KeySigEvent, LyricsEvent, NOTE_NAMES, TimeSigEvent,
};

use crate::widgets::time_ruler::{MarkerEdit, TimeRulerView};

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
}

/// 绘制并交互固定事件行。
///
/// `full_rect`：包含左侧行头列与右侧时间内容区的完整区域；
/// `view`：负责 tick↔px 换算；`snap`：点击/拖动时的量化。
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut egui::Ui,
    state: &EventRowsState,
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
    if state.visible_count() == 0 || full_rect.height() <= 0.0 {
        return EventRowsOutcome {
            marker_edit: None,
            jumped: false,
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
        if kind == EventRowKind::Marker {
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
pub(crate) fn controls(ui: &mut egui::Ui, rect: egui::Rect, state: &mut EventRowsState) {
    let button_rect = egui::Rect::from_min_size(
        rect.min + egui::vec2(6.0, 2.0),
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
                let mut visible = state.is_visible(kind);
                if ui.checkbox(&mut visible, kind.label()).changed() {
                    state.set_visible(kind, visible);
                }
            }
        });
}
