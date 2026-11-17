//! 右侧 Info 面板入口（多栏停靠内容源）。
//!
//! 由右栏多栏布局按选项卡类型分发：
//! - `Track` → [`show_track`]：自动化锚点信息优先，否则音轨属性。
//! - `Batch` → [`show_batch`]：选框批量改值/变速/翻转。
//! - `Layers` → [`show_layers`]：每轨的选中 / 可见 / 锁定。
//! - `History` → [`show_history`]：撤销/重做栈标签。
//! - `Summary` → [`show_summary`]：当前选中轨的音轨统计。
//! - `Selection` → [`show_selection_info`]：选框统计。
//!
//! 工程设置只在独立浮窗（见 `dialogs::prop_panels`）。

mod anchor;
mod channel;
mod history;
mod layers;
mod layers_mix;
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

/// 「音轨属性」选项卡。返回本帧的改动结果。
///
/// 自动化锚点信息优先于音轨属性。
pub(crate) fn show_track(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> track::TrackInfoOut {
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(ui, t!("common.no_document").as_ref());
        return track::TrackInfoOut::default();
    };
    render_track(ui, doc, info_content, automation_drag_ghost)
}

fn render_track(
    ui: &mut egui::Ui,
    doc: &mut Document,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> track::TrackInfoOut {
    match info_content.clone() {
        Some(InfoContent::Anchor {
            track_idx,
            lane_idx,
            event_idx,
            target,
            node,
        }) => {
            let lane_events: Option<&[AutomationEvent]> = if let Some(node) = node {
                doc.mixer
                    .node(node)
                    .and_then(|n| {
                        n.automation
                            .iter()
                            .find(|l| l.target == target)
                            .or_else(|| n.automation.get(lane_idx))
                    })
                    .map(|l| l.events.as_slice())
            } else if matches!(target, AutomationTarget::Tempo) {
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
                    node,
                    info_content,
                );
            } else {
                *info_content = None;
            }
            track::TrackInfoOut::default()
        }
        // 通道焦点不影响「音轨属性」选项卡：它仍显示当前选中轨。
        Some(InfoContent::Track) | Some(InfoContent::Channel(_)) | None => {
            track::show_track_info(ui, doc)
        }
    }
}

/// 「通道属性」选项卡。返回本帧通道级改动。
pub(crate) fn show_channel(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    info_content: &mut Option<InfoContent>,
    per_track_supported: bool,
) -> channel::ChannelInfoOut {
    let Some(doc) = doc else {
        crate::widgets::hint::empty_hint(ui, t!("common.no_document").as_ref());
        return channel::ChannelInfoOut::default();
    };
    channel::show_channel_info(ui, doc, info_content, per_track_supported)
}

/// 「图层」选项卡。`is_mix` 时显示 MIX 通道级图层（MIX 图层面板），
/// 否则显示轨道级图层。返回是否发生音轨结构变化（拖动排序）。
pub(crate) fn show_layers(ui: &mut egui::Ui, doc: &mut Document, is_mix: bool) -> bool {
    if is_mix {
        layers_mix::show(ui, doc)
    } else {
        layers::show(ui, doc)
    }
}

/// 「历史记录」选项卡。返回被点击行的目标撤销栈长度。
pub(crate) fn show_history(ui: &mut egui::Ui, doc: &mut Document) -> Option<usize> {
    history::show(ui, doc)
}

/// 「音轨统计」选项卡。
pub(crate) fn show_summary(ui: &mut egui::Ui, doc: &mut Document) {
    summary::show_summary_panel(ui, doc);
}

/// 「选框统计」选项卡：选框位置/数量/跨度等只读信息。
pub(crate) fn show_selection_info(ui: &mut egui::Ui, doc: &Document) {
    selection::show_info(ui, doc);
}

/// 「批处理」选项卡：对选框批量改值（力度/键位/…）、变速、翻转。
pub(crate) fn show_batch(ui: &mut egui::Ui, doc: &mut Document) {
    selection::show_batch(ui, doc);
}
