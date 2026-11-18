//! AR 轨道动作的落地：`TrackAction` → 文档修改 + undo + 引擎重建标记。
//!
//! 从 `arrange::show` 抽出，避免视图函数同时承担布局、绘制与 20+ 种动作处理。

use rust_i18n::t;

use yinhe_editor_core::document::Document;

use super::track_panel;

/// 动作落地时需要回传的 UI 状态。
pub(crate) struct ActionCtx<'a> {
    pub automation_picker_req: &'a mut Option<usize>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
    pub dock_req: &'a mut Option<crate::chrome::device_chain::DockContext>,
    pub needs_audio_rebuild: &'a mut bool,
    pub needs_audio_notify: &'a mut bool,
}

/// 依次应用本帧收集到的轨道动作。
pub(crate) fn apply_track_actions(
    doc: &mut Document,
    track_actions: Vec<track_panel::TrackAction>,
    ctx: ActionCtx<'_>,
) {
    let ActionCtx {
        automation_picker_req,
        info_content,
        right_tab,
        dock_req,
        needs_audio_rebuild,
        needs_audio_notify,
    } = ctx;
    // Handle track management actions (add/remove/move/AM lane)
    for action in track_actions {
        let before = doc.capture_snapshot();
        // 结构变化（增删移动音轨）→ 重建引擎；AM 内容变化 → 仅 notify。
        let mut structural = true;
        let (undo_action, label) = match &action {
            // 右键「音轨属性」：不产生 undo，选中目标轨后请求打开浮窗。
            track_panel::TrackAction::OpenAutomationPicker { idx } => {
                // 不产生 undo：请求打开参数选择窗口（由调用方落地）。
                structural = false;
                *automation_picker_req = Some(*idx);
                (None, String::new())
            }
            track_panel::TrackAction::ShowProperties { idx } => {
                let track_idx = doc
                    .edit
                    .track_cache
                    .info
                    .get(*idx)
                    .map(|t| t.index)
                    .unwrap_or(*idx as u16);
                doc.edit.track_selected.clear();
                doc.edit.track_selected.insert(track_idx);
                // 音轨属性迁入右侧栏 Info tab（浮窗已移除）。
                *info_content = Some(crate::right_panel::InfoContent::Track);
                *right_tab = Some(crate::right_panel::RightTab::Info);
                (None, String::new())
            }
            track_panel::TrackAction::RenameTrack { idx, name } => {
                structural = false;
                let info = doc.edit.track_cache.info.get(*idx).cloned();
                match info {
                    Some(info) => {
                        let inst = doc
                            .data
                            .model
                            .channel_instruments
                            .get(info.channel as usize)
                            .copied()
                            .flatten();
                        (
                            doc.set_track_info(*idx, name.clone(), info.port, info.channel, inst),
                            t!("undo.edit_track_info").to_string(),
                        )
                    }
                    None => (None, String::new()),
                }
            }
            track_panel::TrackAction::DeleteAutomation { idx, lane_idx } => {
                structural = false;
                (
                    doc.remove_automation_lane(*idx, *lane_idx),
                    t!("undo.delete_automation_lane").to_string(),
                )
            }
            track_panel::TrackAction::AddTrack { after_idx } => {
                let idx = after_idx.unwrap_or(doc.data.model.tracks.len() - 1);
                (doc.add_track(idx), t!("undo.add_track").to_string())
            }
            track_panel::TrackAction::RemoveTrack { idx } => {
                (doc.remove_track(*idx), t!("undo.remove_track").to_string())
            }
            track_panel::TrackAction::MoveUp { idx } => {
                // 重排不改变任何通道，ChannelLayout 不变 → 不重建引擎，
                // 只需 notify（reload_notes 重派发到新的 track 索引）。
                structural = false;
                if *idx > 0 {
                    (
                        doc.move_track(*idx, *idx - 1),
                        t!("undo.move_track_up").to_string(),
                    )
                } else {
                    (None, String::new())
                }
            }
            track_panel::TrackAction::MoveDown { idx } => {
                structural = false;
                if *idx + 1 < doc.data.model.tracks.len() {
                    (
                        doc.move_track(*idx, *idx + 1),
                        t!("undo.move_track_down").to_string(),
                    )
                } else {
                    (None, String::new())
                }
            }
            track_panel::TrackAction::ToggleFolder { idx } => {
                structural = false;
                let model = std::sync::Arc::make_mut(&mut doc.data.model);
                if let Some(track) = model.tracks.get_mut(*idx) {
                    std::sync::Arc::make_mut(track).collapsed ^= true;
                }
                // 折叠后隐藏子轨：清掉 AR 选框，避免后续删除/移动波及不可见音符。
                doc.edit.arr_sel_rect.clear();
                doc.edit.selected.clear_rects();
                (None, String::new())
            }
            track_panel::TrackAction::GroupTracks { indices } => {
                structural = false;
                (
                    doc.group_tracks(indices),
                    t!("undo.group_folder").to_string(),
                )
            }
            track_panel::TrackAction::UngroupTrack { idx } => {
                structural = false;
                (
                    doc.ungroup_track(*idx),
                    t!("undo.ungroup_folder").to_string(),
                )
            }
            track_panel::TrackAction::OpenMasterDock => {
                structural = false;
                *dock_req = Some(crate::chrome::device_chain::DockContext::Master);
                (None, String::new())
            }
            track_panel::TrackAction::CommitMuteSolo { before, after } => {
                structural = false;
                (
                    Some(yinhe_editor_core::history::UndoAction::TrackMuteSolo {
                        before: before.clone(),
                        after: after.clone(),
                    }),
                    t!("undo.track_mute_solo").to_string(),
                )
            }
            track_panel::TrackAction::MoveTracks { indices, insert_at } => {
                // 重排不改变通道集合 → 走 notify 而非 teardown 重建。
                structural = false;
                // 拖拽排序：拆成逐个 move_track，合并为一个 Composite undo。
                let moves = crate::widgets::reorder::plan_moves(
                    doc.data.model.tracks.len(),
                    indices,
                    *insert_at,
                );
                let mut sub_actions = Vec::new();
                for (from, to) in moves {
                    if let Some(a) = doc.move_track(from, to) {
                        sub_actions.push(a);
                    }
                }
                (
                    if sub_actions.is_empty() {
                        None
                    } else {
                        Some(yinhe_editor_core::history::UndoAction::Composite(
                            sub_actions,
                        ))
                    },
                    t!("undo.move_track").to_string(),
                )
            }
        };
        if let Some(action) = undo_action {
            doc.push_undo(action, &label, before);
            if structural {
                // 方案 A：音轨结构变化（add/remove）→ teardown + 下帧重建。
                // 通道集合变化，ChannelLayout 冻结后旧引擎无法 dispatch 新通道。
                // （重排 move 不算结构变化：通道不变，走 notify 的 reload_notes。）
                *needs_audio_rebuild = true;
            } else {
                *needs_audio_notify = true;
            }
        }
    }
}
