use std::sync::Arc;

use eframe::egui;

use yinhe_editor_core::document::Document;
use yinhe_editor_core::history::{AutomationDelta, UndoAction, UndoEntry};
use yinhe_types::{AutomationEvent, AutomationTarget, SegmentShape};

use super::InfoContent;

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

    // 记录初始 revision：编辑 automation / shape / tension 后会 bump_revision，
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
                show_anchor_info(ui, doc, track_idx, lane_idx, event_idx, live_tick, live_value, evt.shape, &target, info_content);
            } else {
                *info_content = None;
            }
            false
        }

        // ── 音轨信息 ──
        Some(InfoContent::Track) => {
            show_track_info(ui, doc, audio, info_content)
        }

        // ── 无选择 → 项目设置 ──
        None => {
            super::project_info::show(ui, Some(doc));
            false
        }
    }
}

// ────────────────────────────────────────────────────────────────
// 音轨信息
// ────────────────────────────────────────────────────────────────

fn show_track_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    audio: Option<&yinhe_audio::CpalAudioHandle>,
    info_content: &mut Option<InfoContent>,
) -> bool {
    let num_tracks = doc.data.model.tracks.len();
    if num_tracks == 0 {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（无音轨）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    }

    // ── Track selector ──
    let track_names: Vec<String> = doc
        .data
        .track_names
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{:03} – {}", i + 1, name))
        .collect();

    let sel_idx = doc
        .edit
        .track_selected
        .iter()
        .next()
        .copied()
        .map(|i| (i as usize).min(num_tracks - 1))
        .unwrap_or(0);

    egui::ComboBox::from_id_salt("info_track_sel")
        .selected_text(&track_names[sel_idx])
        .show_ui(ui, |ui| {
            for (i, tn) in track_names.iter().enumerate() {
                if ui.selectable_label(i == sel_idx, tn).clicked() {
                    doc.edit.track_selected.clear();
                    doc.edit.track_selected.insert(i as u16);
                }
            }
        });

    ui.add_space(6.0);

    // ── 多选汇总 ──
    if doc.edit.track_selected.len() > 1 {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(format!("已选 {} 个音轨", doc.edit.track_selected.len()))
                .strong()
                .size(14.0)
                .color(egui::Color32::from_gray(220)),
        );
        ui.add_space(2.0);

        let total_notes: u64 = doc.edit.track_selected.iter()
            .map(|&idx| doc.edit.track_info_cache.get(idx as usize).map(|ti| ti.note_count).unwrap_or(0))
            .sum();
        let total_events: u64 = doc.edit.track_selected.iter()
            .map(|&idx| doc.edit.track_info_cache.get(idx as usize).map(|ti| ti.event_count).unwrap_or(0))
            .sum();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("总音符数:").size(11.0).color(egui::Color32::GRAY));
            ui.label(egui::RichText::new(format!("{}", total_notes)).size(11.0));
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("总事件数:").size(11.0).color(egui::Color32::GRAY));
            ui.label(egui::RichText::new(format!("{}", total_events)).size(11.0));
        });
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("（多选模式：卷帘将显示所有选中音轨的音符）")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        if ui.add(egui::Button::new(egui::RichText::new("清除选择").size(12.0))).clicked() {
            *info_content = None;
        }
        return false;
    }

    let Some(&track_idx) = doc.edit.track_selected.iter().next() else {
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("（未选中音轨）")
                .color(egui::Color32::from_gray(100))
                .size(12.0),
        );
        return false;
    };
    let track_idx = track_idx as usize;
    let track_idx = track_idx.min(num_tracks - 1);

    // ── Conductor track ──
    if Some(track_idx as u16) == doc.edit.conductor_track_idx {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Conductor")
                .strong()
                .size(14.0)
                .color(egui::Color32::from_gray(220)),
        );
        ui.add_space(2.0);
        ui.label(
            egui::RichText::new("（指挥轨：tempo / time-sig 等全局元事件）")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        ui.add_space(8.0);

        if !doc.data.project_name.is_empty() {
            ui.horizontal(|ui| {
                ui.label("歌曲标题:");
                ui.label(
                    egui::RichText::new(&doc.data.project_name)
                        .color(egui::Color32::from_gray(200))
                        .size(13.0),
                );
            });
            ui.add_space(2.0);
        }

        ui.horizontal(|ui| {
            ui.label("Tempo 数:");
            ui.label(
                egui::RichText::new(format!("{}", doc.data.model.conductor.tempo.events.len()))
                    .color(egui::Color32::from_gray(180))
                    .size(13.0),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Time-sig 数:");
            ui.label(
                egui::RichText::new(format!("{}", doc.data.model.conductor.time_sig.len()))
                    .color(egui::Color32::from_gray(180))
                    .size(13.0),
            );
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        if ui.add(egui::Button::new(egui::RichText::new("清除选择").size(12.0))).clicked() {
            *info_content = None;
        }
        return false;
    }

    // ── Track name ──
    let mut name_change: Option<String> = None;
    let mut name_resp_id: Option<egui::Id> = None;
    let mut name_gained_focus = false;
    let mut name_lost_focus = false;
    ui.horizontal(|ui| {
        ui.label("音轨名称:");
        let mut name = doc.data.track_names[track_idx].clone();
        let resp = ui.add_sized(
            egui::vec2(ui.available_width().max(60.0), 18.0),
            egui::TextEdit::singleline(&mut name).id_salt(("track_name", track_idx)),
        );
        if resp.changed() {
            name_change = Some(name);
        }
        name_resp_id = Some(resp.id);
        name_gained_focus = resp.gained_focus();
        name_lost_focus = resp.lost_focus();
    });
    if let Some(id) = name_resp_id {
        if name_gained_focus {
            yinhe_editor_core::history::begin_edit(
                &mut doc.edit.pending_edits,
                id.value(),
                &doc.data.track_names[track_idx],
            );
        }
        if let Some(new_name) = name_change {
            doc.data.track_names[track_idx] = new_name.clone();
            if let Some(ti_mut) = doc.edit.track_info_cache.get_mut(track_idx) {
                ti_mut.name = new_name;
            }
        }
        if name_lost_focus {
            yinhe_editor_core::history::commit_track_name(
                &mut doc.history,
                &mut doc.edit.pending_edits,
                id.value(),
                track_idx,
                &doc.data.track_names[track_idx],
                doc.edit.selected.clone(),
                doc.edit.track_selected.clone(),
                doc.edit.sel_rect.clone(),
            );
        }
    }
    let ti = &doc.edit.track_info_cache[track_idx];

    ui.add_space(4.0);

    // ── Port / Channel ──
    let mut port_changed = false;
    let mut new_port = ti.port;
    let mut new_ch = ti.channel;

    ui.horizontal(|ui| {
        ui.label("端口/通道:");

        let port_options: Vec<String> = (0..16)
            .map(|p| format!("Port {}", (b'A' + p) as char))
            .collect();
        let _port_sel = egui::ComboBox::from_id_salt("track_port")
            .selected_text(format!("Port {}", (b'A' + ti.port) as char))
            .width(70.0)
            .show_ui(ui, |ui| {
                for (i, label) in port_options.iter().enumerate() {
                    if ui.selectable_label(i == ti.port as usize, label).clicked() {
                        new_port = i as u8;
                        port_changed = true;
                    }
                }
            });

        ui.add_space(4.0);

        let ch_options: Vec<String> = (1..=16).map(|c| format!("{:02}", c)).collect();
        let _ch_sel = egui::ComboBox::from_id_salt("track_channel")
            .selected_text(format!("{:02}", ti.channel))
            .width(50.0)
            .show_ui(ui, |ui| {
                for (i, label) in ch_options.iter().enumerate() {
                    if ui.selectable_label(i + 1 == ti.channel as usize, label).clicked() {
                        new_ch = (i + 1) as u8;
                        port_changed = true;
                    }
                }
            });
    });

    if port_changed {
        {
            let model = Arc::make_mut(&mut doc.data.model);
            if track_idx < model.tracks.len() {
                let td = Arc::make_mut(&mut model.tracks[track_idx]);
                td.port = new_port;
                td.channel = new_ch.saturating_sub(1);
            }
        }
        doc.data.rebuild_model();
        doc.edit.track_info_cache = doc.data.track_info();
        doc.edit.pc_map_cache = doc.data.pc_map_cache();
        doc.data.bump_revision();
        return true;
    }

    ui.add_space(6.0);

    // ── Mute / Solo ──
    while doc.edit.track_overrides.len() <= track_idx {
        doc.edit.track_overrides
            .push(yinhe_editor_core::document::TrackOverride::default());
    }

    let muted = doc.edit.track_overrides[track_idx].muted;
    let soloed = doc.edit.track_overrides[track_idx].soloed;

    let mut mute_clicked = false;
    let mut solo_clicked = false;

    ui.horizontal(|ui| {
        let mute_label = if muted { "🔇 静音" } else { "🔊 静音" };
        let mute_color = if muted {
            crate::theme::MUTE_ACTIVE
        } else {
            egui::Color32::from_gray(140)
        };
        let r1 = ui.add(
            egui::Button::new(egui::RichText::new(mute_label).color(mute_color).size(12.0))
                .min_size(egui::vec2(60.0, 22.0)),
        );

        ui.add_space(4.0);

        let solo_label = if soloed { "🔊 独奏" } else { "🔈 独奏" };
        let solo_color = if soloed {
            crate::theme::SOLO_ACTIVE
        } else {
            egui::Color32::from_gray(140)
        };
        let r2 = ui.add(
            egui::Button::new(egui::RichText::new(solo_label).color(solo_color).size(12.0))
                .min_size(egui::vec2(60.0, 22.0)),
        );

        mute_clicked = r1.clicked();
        solo_clicked = r2.clicked();
    });

    if mute_clicked || solo_clicked {
        while doc.edit.track_overrides.len() <= track_idx {
            doc.edit.track_overrides
                .push(yinhe_editor_core::document::TrackOverride::default());
        }
        if mute_clicked {
            doc.edit.track_overrides[track_idx].muted = !muted;
        }
        if solo_clicked {
            doc.edit.track_overrides[track_idx].soloed = !soloed;
        }
        send_skip_tracks(doc, audio);
    }

    ui.add_space(8.0);

    // ── 摘要 ──
    ui.separator();
    ui.add_space(4.0);
    ui.label(egui::RichText::new("属性摘要").size(11.0).strong());
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("音符数:").size(11.0).color(egui::Color32::GRAY));
        ui.label(egui::RichText::new(format!("{}", ti.note_count)).size(11.0));
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("事件数:").size(11.0).color(egui::Color32::GRAY));
        ui.label(egui::RichText::new(format!("{}", ti.event_count)).size(11.0));
    });

    // Program Change
    let global_ch = ti.port as u32 * 16 + (ti.channel as u32 - 1);
    if let Some(pc) = doc.edit.pc_map_cache.get(&(global_ch as u8)) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("音色:").size(11.0).color(egui::Color32::GRAY));
            ui.label(egui::RichText::new(format!("PC {}", pc)).size(11.0));
        });
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    if ui.add(egui::Button::new(egui::RichText::new("清除选择").size(12.0))).clicked() {
        *info_content = None;
    }

    false
}

// ────────────────────────────────────────────────────────────────
// 锚点信息
// ────────────────────────────────────────────────────────────────

fn show_anchor_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    track_idx: u16,
    lane_idx: usize,
    _event_idx: usize,
    tick: u32,
    value: f32,
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

    // ── Tick（DragValue，gained_focus 记录 before，lost_focus 提交 undo） ──
    let tick_focus_id = ui.id().with("info_anchor_tick_focus");
    let before_tick_id = ui.id().with("info_anchor_before_tick_events");
    let mut edit_tick = tick as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Tick:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(egui::DragValue::new(&mut edit_tick).range(0..=u32::MAX as i64).speed(1.0));

        let gained = resp.gained_focus();
        let lost = resp.lost_focus();

        if gained {
            let before = snapshot_lane_events(doc, track_idx, lane_idx, target);
            ui.ctx().data_mut(|d| {
                d.insert_temp(before_tick_id, before);
                d.insert_temp(tick_focus_id, true);
            });
        }

        if resp.changed() {
            let new_tick = edit_tick as u32;
            if new_tick != tick {
                let _actions = doc.apply_automation_edits(vec![
                    yinhe_types::AutomationEdit::Move {
                        track_idx,
                        lane_idx,
                        target: target.clone(),
                        old_tick: tick,
                        new_tick,
                        new_value: value,
                    },
                ]);
            }
        }

        if lost {
            let before = ui.ctx().data(|d| d.get_temp::<Vec<AutomationEvent>>(before_tick_id));
            if let Some(before) = before {
                let after = snapshot_lane_events(doc, track_idx, lane_idx, target);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Automation(AutomationDelta {
                            track_idx: track_idx as usize,
                            lane_idx,
                            target: target.clone(),
                            before,
                            after,
                        }),
                        label: "Edit automation anchor tick",
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
            }
            ui.ctx().data_mut(|d| {
                d.remove::<Vec<AutomationEvent>>(before_tick_id);
                d.remove::<bool>(tick_focus_id);
            });
        }
    });
    ui.add_space(4.0);

    // ── Value（DragValue，gained_focus 记录 before，lost_focus 提交 undo） ──
    let val_focus_id = ui.id().with("info_anchor_val_focus");
    let before_val_id = ui.id().with("info_anchor_before_val_events");
    let mut edit_value = value as f64;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Value:")
                .size(11.0)
                .color(egui::Color32::GRAY),
        );
        let resp = ui.add(egui::DragValue::new(&mut edit_value).range(0.0..=max_val as f64).speed(1.0));

        let gained = resp.gained_focus();
        let lost = resp.lost_focus();

        if gained {
            let before = snapshot_lane_events(doc, track_idx, lane_idx, target);
            ui.ctx().data_mut(|d| {
                d.insert_temp(before_val_id, before);
                d.insert_temp(val_focus_id, true);
            });
        }

        if resp.changed() {
            let new_value = edit_value as f32;
            if new_value != value {
                let _actions = doc.apply_automation_edits(vec![
                    yinhe_types::AutomationEdit::Move {
                        track_idx,
                        lane_idx,
                        target: target.clone(),
                        old_tick: tick,
                        new_tick: tick,
                        new_value,
                    },
                ]);
            }
        }

        if lost {
            let before = ui.ctx().data(|d| d.get_temp::<Vec<AutomationEvent>>(before_val_id));
            if let Some(before) = before {
                let after = snapshot_lane_events(doc, track_idx, lane_idx, target);
                if before != after {
                    doc.history.push(UndoEntry {
                        action: UndoAction::Automation(AutomationDelta {
                            track_idx: track_idx as usize,
                            lane_idx,
                            target: target.clone(),
                            before,
                            after,
                        }),
                        label: "Edit automation anchor value",
                        selected: doc.edit.selected.clone(),
                        track_selected: doc.edit.track_selected.clone(),
                        sel_rect: doc.edit.sel_rect.clone(),
                    });
                }
            }
            ui.ctx().data_mut(|d| {
                d.remove::<Vec<AutomationEvent>>(before_val_id);
                d.remove::<bool>(val_focus_id);
            });
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
                    target: target.clone(),
                    tick,
                },
            ]);
            push_undo(doc, actions, "Toggle anchor shape");
        }
    });

    // ── Tension（仅 Curve 模式下显示） ──
    if let SegmentShape::Curve { tension } = shape {
        let tension_focus_id = ui.id().with("info_anchor_tension_focus");
        let before_tension_id = ui.id().with("info_anchor_before_tension_events");
        let mut edit_tension = tension;
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Tension:")
                    .size(11.0)
                    .color(egui::Color32::GRAY),
            );
            let resp = ui.add(
                egui::DragValue::new(&mut edit_tension)
                    .range(-1.0..=1.0)
                    .speed(0.02)
                    .fixed_decimals(2),
            );

            let gained = resp.gained_focus();
            let lost = resp.lost_focus();
            if gained {
                let before = snapshot_lane_events(doc, track_idx, lane_idx, target);
                ui.ctx().data_mut(|d| {
                    d.insert_temp(before_tension_id, before);
                    d.insert_temp(tension_focus_id, true);
                });
            }
            if resp.changed() && edit_tension != tension {
                let _action = doc.set_automation_shape(
                    track_idx as usize,
                    lane_idx,
                    target,
                    tick,
                    SegmentShape::Curve { tension: edit_tension },
                );
            }
            if lost {
                let before = ui.ctx().data(|d| d.get_temp::<Vec<AutomationEvent>>(before_tension_id));
                if let Some(before) = before {
                    let after = snapshot_lane_events(doc, track_idx, lane_idx, target);
                    if before != after {
                        doc.history.push(UndoEntry {
                            action: UndoAction::Automation(AutomationDelta {
                                track_idx: track_idx as usize,
                                lane_idx,
                                target: target.clone(),
                                before,
                                after,
                            }),
                            label: "Edit automation anchor tension",
                            selected: doc.edit.selected.clone(),
                            track_selected: doc.edit.track_selected.clone(),
                            sel_rect: doc.edit.sel_rect.clone(),
                        });
                    }
                }
                ui.ctx().data_mut(|d| {
                    d.remove::<Vec<AutomationEvent>>(before_tension_id);
                    d.remove::<bool>(tension_focus_id);
                });
            }
        });
    }

    let shape_desc = match shape {
        SegmentShape::Step => "离散 (Step) — 值在下一个锚点前保持恒定",
        SegmentShape::Curve { tension } => {
            if tension == 0.0 {
                "曲线 (Linear) — 线性插值"
            } else if tension > 0.0 {
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

    if ui.add(egui::Button::new(egui::RichText::new("清除选择").size(12.0))).clicked() {
        *info_content = None;
    }
}

// ────────────────────────────────────────────────────────────────
// 工具函数
// ────────────────────────────────────────────────────────────────

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

fn snapshot_lane_events(
    doc: &Document,
    track_idx: u16,
    lane_idx: usize,
    target: &AutomationTarget,
) -> Vec<AutomationEvent> {
    if matches!(target, AutomationTarget::Tempo) {
        doc.data.model.conductor.tempo.events.clone()
    } else {
        doc.data.model.tracks
            .get(track_idx as usize)
            .and_then(|t| t.automation_lanes.get(lane_idx))
            .map(|l| l.events.clone())
            .unwrap_or_default()
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
