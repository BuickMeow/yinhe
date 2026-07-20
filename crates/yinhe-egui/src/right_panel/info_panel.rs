//! 右侧 Info 面板入口。
//!
//! 按 `InfoContent` 分发到子模块：
//! - [`anchor`] — 自动化锚点信息（Tick / Value / Shape / Ctrl X / Ctrl Y）
//! - [`track`] — 音轨信息（名称 / 端口 / 通道 / Mute / Solo / 摘要）
//! - `project_info` — 项目设置（无选择时）

mod anchor;
mod track;

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationEvent, AutomationTarget};

use super::InfoContent;

// re-export：arrange.rs 通过 `crate::right_panel::info_panel::send_skip_tracks` 调用
pub(crate) use track::send_skip_tracks;

/// Show the Info panel.
///
/// Returns `true` if the port or channel was changed (caller should tear
/// down the audio engine so it gets rebuilt with the new channel map).
pub fn show(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> bool {
    let Some(doc) = doc else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未打开文档）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    };

    // 记录初始 revision：编辑 automation / shape / ctrl 后会 bump_revision，
    // 退出时若发现 revision 变了就通知音频线程 reload。
    let rev_before = doc.data.revision;
    let port_changed = render(ui, doc, audio, info_content, automation_drag_ghost);
    let rev_after = doc.data.revision;
    if rev_after != rev_before {
        if let Some(audio) = audio {
            audio.reload_notes(doc.data.model.clone());
        }
    }
    port_changed
}

fn render(
    ui: &mut egui::Ui,
    doc: &mut Document,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    info_content: &mut Option<InfoContent>,
    automation_drag_ghost: Option<(u32, f32)>,
) -> bool {
    match info_content.clone() {
        // ── 锚点信息 ──
        Some(InfoContent::Anchor { track_idx, lane_idx, event_idx, target }) => {
            // Tempo 走 conductor.tempo；其他走 track.automation_lanes
            let lane_events: Option<&[AutomationEvent]> = if matches!(target, AutomationTarget::Tempo) {
                Some(&doc.data.model.conductor.tempo.events)
            } else {
                doc.data.model.tracks
                    .get(track_idx as usize)
                    .and_then(|t| t.automation_lanes.get(lane_idx))
                    .map(|l| l.events.as_slice())
            };
            let live_event = lane_events.and_then(|events| events.get(event_idx));

            if let Some(evt) = live_event {
                let (live_tick, live_value) = if let Some((g_tick, g_value)) = automation_drag_ghost {
                    (g_tick, g_value)
                } else {
                    (evt.tick, evt.value)
                };
                anchor::show_anchor_info(ui, doc, track_idx, lane_idx, event_idx, live_tick, live_value, evt.shape, &target, info_content);
            } else {
                *info_content = None;
            }
            false
        }

        // ── 音轨信息 ──
        Some(InfoContent::Track) => {
            track::show_track_info(ui, doc, audio, info_content)
        }

        // ── 无选择 → 项目设置 ──
        None => {
            super::project_info::show(ui, Some(doc));
            false
        }
    }
}
