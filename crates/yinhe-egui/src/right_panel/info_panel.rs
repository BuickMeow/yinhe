use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_types::{AutomationTarget, SegmentShape};

use super::InfoContent;

/// Show the Info panel.
///
/// When `info_content` is `Some(InfoContent::Anchor { .. })`, shows anchor
/// editing controls. Otherwise falls back to showing project settings.
///
/// Returns `true` if the port or channel was changed (caller should tear
/// down the audio engine so it gets rebuilt with the new channel map).
pub fn show(
    ui: &mut egui::Ui,
    doc: Option<&mut Document>,
    _audio: Option<&yinhe_audio::CpalAudioHandle>,
    info_content: &mut Option<InfoContent>,
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

    // ── 锚点信息编辑 ──
    if let Some(content) = info_content.clone() {
        match content {
            InfoContent::Anchor { track_idx, lane_idx, tick, target } => {
                // 从模型实时读取锚点最新状态
                let track = doc.data.model.tracks.get(track_idx as usize);
                let lane = track.and_then(|t| t.automation_lanes.get(lane_idx));
                let live_event = lane
                    .and_then(|l| l.events.iter().find(|e| e.tick == tick));

                if let Some(evt) = live_event {
                    // 锚点仍存在，用最新值渲染
                    show_anchor_info(ui, doc, track_idx, lane_idx, tick, evt.value, evt.shape, &target, info_content);
                } else {
                    // 锚点已被删除，清除选择
                    *info_content = None;
                }
                return false;
            }
        }
    }

    // ── 无选择内容时，回退到项目设置 ──
    super::project_info::show(ui, Some(doc));
    false
}

/// 渲染锚点信息编辑界面。
fn show_anchor_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    track_idx: u16,
    lane_idx: usize,
    tick: u32,
    value: u16,
    shape: SegmentShape,
    target: &AutomationTarget,
    info_content: &mut Option<InfoContent>,
) {
    let max_val = target.max_value();

    ui.add_space(4.0);

    // ── 标题 ──
    ui.label(
        egui::RichText::new("自动化锚点")
            .strong()
            .size(14.0)
            .color(egui::Color32::from_gray(220)),
    );
    ui.add_space(2.0);

    // ── 目标名称（只读） ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("目标:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.label(
            egui::RichText::new(target.display_name())
                .size(12.0)
                .color(egui::Color32::from_gray(200)),
        );
    });
    ui.add_space(4.0);

    // ── Tick（可编辑，DragValue，实时同步） ──
    let mut edit_tick = tick as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Tick:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(egui::DragValue::new(&mut edit_tick).range(0..=u32::MAX as i64).speed(1.0));
        if resp.lost_focus() {
            let new_tick = edit_tick as u32;
            if new_tick != tick {
                let actions = doc.apply_automation_edits(vec![
                    yinhe_types::AutomationEdit::Move {
                        track_idx,
                        lane_idx,
                        old_tick: tick,
                        new_tick,
                        new_value: value,
                    },
                ]);
                push_undo(doc, actions, "Edit automation anchor tick");
                // 更新 info_content 中的 tick（下帧会从模型读取新 tick 处的锚点）
                *info_content = Some(InfoContent::Anchor {
                    track_idx, lane_idx,
                    tick: new_tick,
                    target: target.clone(),
                });
            }
        }
    });
    ui.add_space(4.0);

    // ── Value（可编辑，DragValue，实时同步） ──
    let mut edit_value = value as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Value:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(egui::DragValue::new(&mut edit_value).range(0..=max_val as i64).speed(1.0));
        if resp.lost_focus() {
            let new_value = edit_value as u16;
            if new_value != value {
                let actions = doc.apply_automation_edits(vec![
                    yinhe_types::AutomationEdit::Move {
                        track_idx,
                        lane_idx,
                        old_tick: tick,
                        new_tick: tick,
                        new_value,
                    },
                ]);
                push_undo(doc, actions, "Edit automation anchor value");
                *info_content = Some(InfoContent::Anchor {
                    track_idx, lane_idx,
                    tick,
                    target: target.clone(),
                });
            }
        }
    });
    ui.add_space(6.0);

    // ── Shape（离散/曲线切换） ──
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("曲线类型:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let is_step = matches!(shape, SegmentShape::Step);
        let mut discrete = is_step;
        let resp = ui.checkbox(&mut discrete, "离散 (Step)");
        if resp.changed() {
            let actions = doc.apply_automation_edits(vec![
                yinhe_types::AutomationEdit::CycleShape {
                    track_idx,
                    lane_idx,
                    tick,
                },
            ]);
            push_undo(doc, actions, "Toggle anchor shape");
            // CycleShape 会自动在 Step ↔ Curve{tension:0} 间切换
            // 不需要手动更新 info_content，下帧会从模型读取
        }
    });

    // 显示当前 shape 描述
    let shape_desc = match shape {
        SegmentShape::Step => "离散 (Step) — 值在下一个锚点前保持恒定",
        SegmentShape::Curve { tension } => {
            if tension == 0 {
                "曲线 (Linear) — 线性插值"
            } else if tension > 0 {
                "曲线 (缓入) — 加速变化"
            } else {
                "曲线 (缓出) — 减速变化"
            }
        }
    };
    ui.label(
        egui::RichText::new(shape_desc)
            .size(10.0)
            .color(egui::Color32::from_gray(140)),
    );

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);

    // ── 清除选择按钮 ──
    if ui
        .add(egui::Button::new(
            egui::RichText::new("清除选择").size(12.0),
        ))
        .clicked()
    {
        *info_content = None;
    }
}

fn push_undo(
    doc: &mut Document,
    actions: Vec<yinhe_editor_core::history::UndoAction>,
    label: &'static str,
) {
    for action in actions {
        doc.history.push(yinhe_editor_core::history::UndoEntry {
            action,
            label,
            selected: doc.edit.selected.clone(),
            track_selected: doc.edit.track_selected.clone(),
            sel_rect: doc.edit.sel_rect.clone(),
        });
    }
}

/// Compute the per-track skip mask and send it to the audio engine.
pub(crate) fn send_skip_tracks(doc: &Document, audio: Option<&yinhe_audio::CpalAudioHandle>) {
    let has_solo = doc.edit.track_overrides.iter().any(|t| t.soloed);
    let skip: Vec<bool> = doc
        .edit
        .track_overrides
        .iter()
        .map(|ov| if has_solo { !ov.soloed } else { ov.muted })
        .collect();

    if let Some(audio) = audio {
        audio
            .handle
            .send(yinhe_audio::AudioCommand::SkipTracks { skip });
    }
}
