//! 右侧 Info 面板入口（多栏停靠内容源）。
//!
//! 由右栏多栏布局按选项卡类型分发：
//! - `Track` → [`show_track`]：自动化锚点信息优先，否则音轨属性。
//! - `Batch` → [`show_batch`]：选框批量改值/变速/翻转。
//! - `Layers` → [`show_layers`]：每轨的选中 / 可见 / 锁定。
//! - `History` → [`show_history`]：撤销/重做栈标签。
//! - `Summary` → [`show_summary`]：当前选中轨的统计。
//! - `Selection` → [`show_selection_info`]：选框只读属性。
//!
//! 工程设置只在独立浮窗（见 `dialogs::prop_panels`）。

mod anchor;
mod history;
mod layers;
pub(crate) mod selection;
mod summary;
mod track;

use eframe::egui;

use rust_i18n::t;
use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationEvent, AutomationTarget};

use super::InfoContent;

// re-export：arrange.rs 通过 `crate::right_panel::info_panel::send_skip_tracks` 调用
pub(crate) use track::send_skip_tracks;

/// 「音轨」选项卡。返回 `true` 表示端口/通道改变（需重建音频引擎）。
///
/// 自动化锚点信息优先于音轨属性。
pub(crate) fn show_track(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> bool {
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(ui, t!("common.no_document").as_ref());
        return false;
    };
    let rev_before = doc.data.revision;
    let port_changed = render_track(ui, doc, info_content, automation_drag_ghost);
    let _ = rev_before;
    port_changed
}

fn render_track(
    ui: &mut egui::Ui,
    doc: &mut Document,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> bool {
    match info_content.clone() {
        Some(InfoContent::Anchor {
            track_idx,
            lane_idx,
            event_idx,
            target,
        }) => {
            let lane_events: Option<&[AutomationEvent]> =
                if matches!(target, AutomationTarget::Tempo) {
                    Some(&doc.data.model.conductor.tempo.events)
                } else {
                    doc.data
                        .model
                        .tracks
                        .get(track_idx as usize)
                        .and_then(|t| t.automation_lanes.get(lane_idx))
                        .map(|l| l.events.as_slice())
                };
            let live_event = lane_events.and_then(|events| events.get(event_idx));
            if let Some(evt) = live_event {
                let (live_tick, live_value) =
                    automation_drag_ghost.unwrap_or((evt.tick, evt.value));
                anchor::show_anchor_info(
                    ui,
                    doc,
                    track_idx,
                    lane_idx,
                    event_idx,
                    live_tick,
                    live_value,
                    evt.shape,
                    &target,
                    info_content,
                );
            } else {
                *info_content = None;
            }
            false
        }
        Some(InfoContent::Track) | None => track::show_track_info(ui, doc),
    }
}

/// 「图层」选项卡。
pub(crate) fn show_layers(ui: &mut egui::Ui, doc: &mut Document) {
    layers::show(ui, doc);
}

/// 「历史记录」选项卡。
pub(crate) fn show_history(ui: &mut egui::Ui, doc: &mut Document) {
    history::show(ui, doc);
}

/// 「属性概要」选项卡。
pub(crate) fn show_summary(ui: &mut egui::Ui, doc: &mut Document) {
    summary::show_summary_panel(ui, doc);
}

/// 「选框属性」选项卡：选框位置/数量/跨度等只读信息。
pub(crate) fn show_selection_info(ui: &mut egui::Ui, doc: &Document) {
    selection::show_info(ui, doc);
}

/// 「批处理」选项卡：对选框批量改值（力度/键位/…）、变速、翻转。
pub(crate) fn show_batch(ui: &mut egui::Ui, doc: &mut Document) {
    selection::show_batch(ui, doc);
}
