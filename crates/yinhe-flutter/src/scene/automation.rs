use super::*;

/// PR 自动化栏数据（PB/CC/RPN；Conductor 轨只有 Tempo），JSON：
/// `[{"kind":0,"max":16383,"name":"Pitch Bend","points":[[tick,value],...]}, ...]`
///
/// `points` 的 value 已归一化 0..1（Conductor 的 Tempo 按当前最大 BPM 归一化）。
pub fn automation_json(track: usize) -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let Some(t) = doc.data.model.tracks.get(track) else {
        return "[]".into();
    };
    if t.kind == yinhe_core::TrackKind::Master {
        // Conductor：AM 只有 Tempo 一种自动化；值域 = 当前最大 Tempo。
        let lane = &doc.data.model.conductor.tempo;
        let max = tempo_max_bpm(&doc.data.model);
        let mut out = String::from("[");
        out.push_str(&format!(
            "{{\"kind\":0,\"num\":-1,\"max\":{max},\"name\":\"Tempo\",\"points\":["
        ));
        for (i, e) in lane.events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("[{},{}]", e.tick, e.value / max));
        }
        out.push_str("]}]");
        return out;
    }
    let mut out = String::from("[");
    let mut first = true;
    for lane in &t.automation_lanes {
        let Some(kind) = lane_kind(&lane.target) else {
            continue;
        };
        let (num, max) = lane_display_range(&lane.target);
        if !first {
            out.push(',');
        }
        first = false;
        out.push_str(&format!(
            "{{\"kind\":{},\"num\":{},\"max\":{},\"name\":\"{}\",\"points\":[",
            kind,
            num,
            max,
            json_escape(&lane.target.display_name())
        ));
        for (i, e) in lane.events.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("[{},{}]", e.tick, e.value));
        }
        out.push_str("]}");
    }
    out.push(']');
    out
}

/// Dart 布局后写入 AM 面板在主纹理中的绘制区域与当前 lane。
#[allow(clippy::too_many_arguments)]
pub fn set_automation_panel(
    w: f32,
    h: f32,
    visible: bool,
    active_track: usize,
    lane: Option<usize>,
    show_anchors: bool,
) {
    let Ok(mut s) = scene().lock() else {
        return;
    };
    let visible = visible && w >= 1.0 && h >= 1.0;
    let p = &mut s.automation_panel;
    // 切轨/切 lane 才清空 AM 选区；尺寸变化（旋转/键盘）保留选中。
    let lane_changed = p.active_track != active_track || p.lane != lane;
    let selected = if lane_changed {
        Vec::new()
    } else {
        p.selected_ticks.clone()
    };
    if p.visible == visible
        && p.w == w
        && p.h == h
        && p.active_track == active_track
        && p.lane == lane
        && p.show_anchors == show_anchors
    {
        return;
    }
    *p = AutomationPanelState {
        visible,
        w,
        h,
        active_track,
        lane,
        show_anchors,
        ghost: Vec::new(),
        selected_ticks: selected,
    };
}

/// AM 选区同步：当前 lane 上被选中的锚点 tick（升序去重）。
pub fn set_automation_selection(ticks: &[u32]) {
    let Ok(mut s) = scene().lock() else {
        return;
    };
    let mut ticks = ticks.to_vec();
    ticks.sort_unstable();
    ticks.dedup();
    s.automation_panel.selected_ticks = ticks;
}

/// 当前 AM 面板对应的自动化 target（Conductor 固定 Tempo）。
fn panel_automation_target(
    doc: &Document,
    panel: &AutomationPanelState,
) -> Option<AutomationTarget> {
    let track = doc.data.model.tracks.get(panel.active_track)?;
    if track.kind == yinhe_core::TrackKind::Master {
        return Some(AutomationTarget::Tempo);
    }
    let lane_index = panel.lane?;
    visible_lane(track, lane_index).map(|(_, target)| target)
}

/// 复制当前 AM lane 选中的锚点到应用内剪贴板。成功 1。
pub fn automation_copy() -> i32 {
    let Ok(s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_ref() else {
        return 0;
    };
    let panel = &s.automation_panel;
    let Some(target) = panel_automation_target(doc, panel) else {
        return 0;
    };
    if panel.selected_ticks.is_empty() {
        return 0;
    }
    let selected = &panel.selected_ticks;
    let mut events: Vec<(u32, f32, yinhe_types::SegmentShape)> =
        if matches!(target, AutomationTarget::Tempo) {
            doc.data
                .model
                .conductor
                .tempo
                .events
                .iter()
                .filter(|e| selected.binary_search(&e.tick).is_ok())
                .map(|e| (e.tick, e.value, e.shape))
                .collect()
        } else {
            let track = &doc.data.model.tracks[panel.active_track];
            let Some((raw_idx, _)) = panel.lane.and_then(|i| visible_lane(track, i)) else {
                return 0;
            };
            track.automation_lanes[raw_idx]
                .events
                .iter()
                .filter(|e| selected.binary_search(&e.tick).is_ok())
                .map(|e| (e.tick, e.value, e.shape))
                .collect()
        };
    if events.is_empty() {
        return 0;
    }
    events.sort_by_key(|(tick, _, _)| *tick);
    drop(s);
    let Ok(mut slot) = AUTOMATION_CLIPBOARD.lock() else {
        return 0;
    };
    *slot = Some(AutomationClip { target, events });
    1
}

/// 当前 AM lane 的锚点插值状态 + UI 预设列表（JSON）。
///
/// ```json
/// {"current":12,"mixed":false,"count":3,
///  "items":[{"i":0,"f":"","d":"Step"},{"i":1,"f":"","d":"Linear"},
///           {"i":2,"f":"Sine","d":"In"}, ...]}
/// ```
///
/// `current = -1`：没有可改的锚点或选中形状不一致；`mixed` 标明是否混合。
/// `count` = 这次应用会改多少个锚点（有选中时是选中数，否则整条 lane）。
pub fn automation_shape_json() -> String {
    use yinhe_types::automation::EASING_PRESETS;

    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "{}".into();
    };
    let panel = &s.automation_panel;
    let Some(target) = panel_automation_target(doc, panel) else {
        return "{}".into();
    };
    let events: &[yinhe_types::AutomationEvent] = if matches!(target, AutomationTarget::Tempo) {
        &doc.data.model.conductor.tempo.events
    } else {
        let Some(track) = doc.data.model.tracks.get(panel.active_track) else {
            return "{}".into();
        };
        let Some((raw_idx, _)) = panel.lane.and_then(|i| visible_lane(track, i)) else {
            return "{}".into();
        };
        &track.automation_lanes[raw_idx].events
    };
    let selected: Vec<&yinhe_types::AutomationEvent> = if panel.selected_ticks.is_empty() {
        events.iter().collect()
    } else {
        events
            .iter()
            .filter(|e| panel.selected_ticks.binary_search(&e.tick).is_ok())
            .collect()
    };
    let count = selected.len();
    let first = selected.first().and_then(|e| e.shape.ui_index());
    let mixed = selected.iter().any(|e| e.shape.ui_index() != first);
    let current = if mixed { None } else { first };
    let current = current.map_or(-1, |i| i as i32);

    let mut items =
        String::from("[{\"i\":0,\"f\":\"\",\"d\":\"Step\"},{\"i\":1,\"f\":\"\",\"d\":\"Linear\"}");
    for (idx, preset) in EASING_PRESETS.iter().enumerate() {
        let yinhe_types::SegmentShape::Curve { x1, y1, x2, y2 } = preset.curve else {
            continue;
        };
        let scale = yinhe_types::SegmentShape::SCALE;
        // 输出 CSS 绝对控制点（0..1 归一化空间），Dart 直接按参数曲线采样画预览。
        items.push_str(&format!(
            ",{{\"i\":{},\"f\":\"{}\",\"d\":\"{}\",\"x1\":{:.4},\"y1\":{:.4},\"x2\":{:.4},\"y2\":{:.4}}}",
            idx + 2,
            preset.family.label(),
            preset.direction.label(),
            x1 * scale,
            y1 * scale,
            1.0 + x2 * scale,
            1.0 + y2 * scale,
        ));
    }
    items.push(']');
    format!("{{\"current\":{current},\"mixed\":{mixed},\"count\":{count},\"items\":{items}}}")
}

/// 应用锚点插值形状（编号见 `SegmentShape::from_ui_index`）。
///
/// 有选中锚点时只改选中，否则改整条 lane；一次 undo。返回改动的锚点数。
pub fn automation_apply_shape(index: i32) -> i32 {
    use yinhe_types::automation::AutomationEdit;

    let Some(shape) = yinhe_types::SegmentShape::from_ui_index(index.max(0) as usize) else {
        return 0;
    };
    let panel = {
        let Ok(s) = scene().lock() else {
            return 0;
        };
        if s.document.is_none() {
            return 0;
        }
        s.automation_panel.clone()
    };
    crate::doc_job::try_start(
        "automation-shape",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            let Some(target) = panel_automation_target(doc, &panel) else {
                return false;
            };
            let (track_idx, lane_idx) = if matches!(target, AutomationTarget::Tempo) {
                (0u16, 0usize)
            } else {
                let Some(track) = doc.data.model.tracks.get(panel.active_track) else {
                    return false;
                };
                let Some((raw_idx, _)) = panel.lane.and_then(|i| visible_lane(track, i)) else {
                    return false;
                };
                (panel.active_track as u16, raw_idx)
            };
            let ticks: Vec<u32> = {
                let events: &[yinhe_types::AutomationEvent] =
                    if matches!(target, AutomationTarget::Tempo) {
                        &doc.data.model.conductor.tempo.events
                    } else {
                        &doc.data.model.tracks[track_idx as usize].automation_lanes[lane_idx].events
                    };
                if panel.selected_ticks.is_empty() {
                    events.iter().map(|e| e.tick).collect()
                } else {
                    events
                        .iter()
                        .filter(|e| panel.selected_ticks.binary_search(&e.tick).is_ok())
                        .map(|e| e.tick)
                        .collect()
                }
            };
            if ticks.is_empty() {
                return false;
            }
            let edits: Vec<AutomationEdit> = ticks
                .iter()
                .map(|&tick| AutomationEdit::SetShape {
                    track_idx,
                    lane_idx,
                    target: target.clone(),
                    tick,
                    shape,
                })
                .collect();
            let before = doc.capture_snapshot();
            let actions = doc.apply_automation_edits(edits);
            if actions.is_empty() {
                return false;
            }
            let undo = match actions.len() {
                1 => actions.into_iter().next(),
                _ => Some(yinhe_editor_core::history::UndoAction::Composite(actions)),
            };
            if let Some(undo) = undo {
                doc.push_undo(undo, "修改锚点插值", before);
            }
            doc.edit.pianoroll_view.base.dirty = true;
            true
        },
        None::<fn()>,
    )
}

/// AM 剪贴板是否为空（Dart 菜单置灰用）。
pub fn automation_clipboard_empty() -> bool {
    AUTOMATION_CLIPBOARD
        .lock()
        .map(|slot| slot.as_ref().is_none_or(|clip| clip.events.is_empty()))
        .unwrap_or(true)
}

/// 把 AM 剪贴板粘贴到当前 lane：首个锚点对齐 `tick`，一次 undo。成功 1。
pub fn automation_paste(tick: f64) -> i32 {
    let Some(clip) = AUTOMATION_CLIPBOARD.lock().ok().and_then(|s| s.clone()) else {
        return 0;
    };
    if clip.events.is_empty() {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let panel = s.automation_panel.clone();
    let target = {
        let Some(doc) = s.document.as_ref() else {
            return 0;
        };
        let Some(target) = panel_automation_target(doc, &panel) else {
            return 0;
        };
        target
    };
    // 跨 target 的值域不同（Tempo 是 BPM，CC/PB 是归一化值），不做隐式换算。
    if target != clip.target {
        return 0;
    }
    let base = tick.max(0.0).round() as u32;
    let min_tick = clip.events.iter().map(|e| e.0).min().unwrap_or(base);
    let delta = i64::from(base) - i64::from(min_tick);
    let edits: Vec<yinhe_types::automation::AutomationEdit> = clip
        .events
        .iter()
        .map(
            |(t, value, shape)| yinhe_types::automation::AutomationEdit::Add {
                track_idx: panel.active_track as u16,
                target: target.clone(),
                tick: (i64::from(*t) + delta).max(0) as u32,
                value: *value,
                shape: *shape,
            },
        )
        .collect();
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let actions = doc.apply_automation_edits(edits);
    if actions.is_empty() {
        return 0;
    }
    let undo = match actions.len() {
        1 => actions.into_iter().next(),
        _ => Some(yinhe_editor_core::history::UndoAction::Composite(actions)),
    };
    if let Some(undo) = undo {
        doc.push_undo(undo, "粘贴自动化", before);
    }
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 批量处理（AM 当前 lane）：值与位置各一个表达式（`100`/`+2`/`x0.5`），
/// 空串或无效 = 跳过。值是显示值（CC 0..127、PB 0..16383）。
pub fn automation_batch_edit(value_expr: &str, tick_expr: &str) -> i32 {
    use yinhe_editor_core::num_expr::{apply_ops, apply_ops_round};

    let panel = {
        let Ok(s) = scene().lock() else {
            return 0;
        };
        if s.document.is_none() {
            return 0;
        }
        s.automation_panel.clone()
    };
    let Some(lane_index) = panel.lane else {
        return 0;
    };
    if panel.selected_ticks.is_empty() {
        return 0;
    }
    let value_expr = value_expr.to_string();
    let tick_expr = tick_expr.to_string();
    crate::doc_job::try_start(
        "automation-batch",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            let value_ops = field_ops(&value_expr);
            let tick_ops = field_ops(&tick_expr);
            if value_ops.is_none() && tick_ops.is_none() {
                return false;
            }
            let track = panel.active_track;
            let Some((raw_idx, target)) = doc
                .data
                .model
                .tracks
                .get(track)
                .and_then(|t| visible_lane(t, lane_index))
            else {
                return false;
            };
            let display_max = f64::from(target.display_max().unwrap_or(1.0));
            let Some(lane) = doc
                .data
                .model
                .tracks
                .get(track)
                .and_then(|t| t.automation_lanes.get(raw_idx))
            else {
                return false;
            };
            let events: Vec<(u32, f32)> = lane
                .events
                .iter()
                .filter(|e| panel.selected_ticks.binary_search(&e.tick).is_ok())
                .map(|e| (e.tick, e.value))
                .collect();
            if events.is_empty() {
                return false;
            }

            let mut moves: Vec<(u32, u32, f32)> = Vec::new();
            for (old_tick, old_value) in events {
                let new_tick = match &tick_ops {
                    Some(ops) => apply_ops_round(ops, f64::from(old_tick))
                        .clamp(0.0, f64::from(u32::MAX)) as u32,
                    None => old_tick,
                };
                let new_value = match &value_ops {
                    Some(ops) => {
                        let cur = f64::from(target.to_display_value(old_value));
                        let display = apply_ops(ops, cur).clamp(0.0, display_max) as f32;
                        target.from_display_value(display)
                    }
                    None => old_value,
                };
                if new_tick != old_tick || new_value != old_value {
                    moves.push((old_tick, new_tick, new_value));
                }
            }
            if moves.is_empty() {
                return false;
            }

            let before = doc.capture_snapshot();
            let Some(action) = doc.move_automation_events_batch(track, raw_idx, &target, &moves)
            else {
                return false;
            };
            doc.push_undo(action, "批量处理", before);
            doc.edit.pianoroll_view.base.dirty = true;
            true
        },
        None::<fn()>,
    )
}

/// AM 当前 lane 选中锚点的批量操作。
///
/// 只支持删除 / 重复 / 时值 ×2÷2 / 水平翻转；移调、垂直翻转、填缝、
/// 合并、网格切割对自动化锚点没有意义，返回 false。
pub(super) fn automation_selection_action(
    doc: &mut Document,
    panel: &AutomationPanelState,
    action: i32,
) -> bool {
    use yinhe_types::automation::AutomationEdit;
    let Some(lane_index) = panel.lane else {
        return false;
    };
    let ticks = &panel.selected_ticks;
    if ticks.is_empty() {
        return false;
    }
    let track_idx = panel.active_track;
    let Some((raw_idx, target)) = doc
        .data
        .model
        .tracks
        .get(track_idx)
        .and_then(|t| visible_lane(t, lane_index))
    else {
        return false;
    };
    let Some(lane) = doc
        .data
        .model
        .tracks
        .get(track_idx)
        .and_then(|t| t.automation_lanes.get(raw_idx))
    else {
        return false;
    };
    // 选中事件（按 tick 升序，lane 本身有序）。
    let selected: Vec<(u32, f32, yinhe_types::SegmentShape)> = lane
        .events
        .iter()
        .filter(|e| ticks.binary_search(&e.tick).is_ok())
        .map(|e| (e.tick, e.value, e.shape))
        .collect();
    if selected.is_empty() {
        return false;
    }
    let t0 = selected[0].0;
    let t1 = selected[selected.len() - 1].0;
    let track_u16 = track_idx as u16;
    let ppq = doc.data.model.meta.ppq;
    let (num, den) = doc.data.model.tempo_map.time_sig_default;
    let interval = doc
        .edit
        .quantize_pianoroll
        .resolved_with_time_sig(
            doc.edit.pianoroll_view.base.pixels_per_tick,
            ppq,
            num,
            den,
            true,
        )
        .tick_interval(ppq)
        .max(1);

    let mut edits: Vec<AutomationEdit> = Vec::new();
    match action {
        11 => {
            for (tick, _, _) in &selected {
                edits.push(AutomationEdit::Delete {
                    track_idx: track_u16,
                    lane_idx: raw_idx,
                    target: target.clone(),
                    tick: *tick,
                });
            }
        }
        10 => {
            let delta = if t1 > t0 { t1 - t0 } else { interval };
            let existing: HashSet<u32> = lane.events.iter().map(|e| e.tick).collect();
            for (tick, value, shape) in &selected {
                let new_tick = tick.saturating_add(delta);
                if existing.contains(&new_tick) {
                    continue;
                }
                edits.push(AutomationEdit::Add {
                    track_idx: track_u16,
                    target: target.clone(),
                    tick: new_tick,
                    value: *value,
                    shape: *shape,
                });
            }
        }
        19 | 20 => {
            let span = t1.saturating_sub(t0);
            if span == 0 {
                return false;
            }
            let (num, den) = if action == 19 { (2u64, 1u64) } else { (1, 2) };
            let new_span = (u64::from(span) * num / den).max(1);
            let factor = new_span as f64 / f64::from(span);
            let moves: Vec<(u32, u32, f32)> = selected
                .iter()
                .filter_map(|(tick, value, _)| {
                    let nt = (f64::from(t0) + (f64::from(*tick) - f64::from(t0)) * factor)
                        .round()
                        .clamp(0.0, f64::from(u32::MAX)) as u32;
                    (nt != *tick).then_some((*tick, nt, *value))
                })
                .collect();
            if moves.is_empty() {
                return false;
            }
            edits.push(AutomationEdit::MoveBatch {
                track_idx: track_u16,
                lane_idx: raw_idx,
                target: target.clone(),
                moves,
            });
        }
        21 => {
            let moves: Vec<(u32, u32, f32)> = selected
                .iter()
                .filter_map(|(tick, value, _)| {
                    let nt = t0 + t1 - *tick;
                    (nt != *tick).then_some((*tick, nt, *value))
                })
                .collect();
            if moves.is_empty() {
                return false;
            }
            edits.push(AutomationEdit::MoveBatch {
                track_idx: track_u16,
                lane_idx: raw_idx,
                target: target.clone(),
                moves,
            });
        }
        _ => return false,
    }
    if edits.is_empty() {
        return false;
    }
    let before = doc.capture_snapshot();
    let actions = doc.apply_automation_edits(edits);
    if actions.is_empty() {
        return false;
    }
    // 多步编辑合成一次 undo（与桌面 automation_undo 一致）。
    let undo = match actions.len() {
        1 => actions.into_iter().next(),
        _ => Some(yinhe_editor_core::history::UndoAction::Composite(actions)),
    };
    if let Some(undo) = undo {
        doc.push_undo(undo, "自动化选区操作", before);
    }
    doc.edit.sel_rect.clear();
    doc.edit.selected.clear();
    true
}

/// 拖动预览：扁平 `[old_tick, new_tick, new_value, ...]`；空数组清除。
pub fn set_automation_ghost(flat: &[f64]) {
    let Ok(mut s) = scene().lock() else {
        return;
    };
    let mut ghost = Vec::with_capacity(flat.len() / 3);
    for c in flat.chunks(3) {
        if c.len() == 3 && c.iter().all(|v| v.is_finite()) {
            ghost.push((c[0].max(0.0) as u32, c[1].max(0.0) as u32, c[2] as f32));
        }
    }
    s.automation_panel.ghost = ghost;
}

/// 插入目标：kind 0=Tempo、1=PB、2=CC、3=RPN；number 为 CC 号/RPN 号。
pub(super) fn insert_target(
    kind: i32,
    number: i32,
) -> Option<yinhe_types::automation::AutomationTarget> {
    use yinhe_types::automation::AutomationTarget;
    match kind {
        0 => Some(AutomationTarget::Tempo),
        1 => Some(AutomationTarget::PitchBend),
        2 => Some(AutomationTarget::CC {
            controller: number.clamp(0, 127) as u8,
        }),
        3 => Some(AutomationTarget::Rpn {
            parameter: number.clamp(0, 16383) as u16,
        }),
        _ => None,
    }
}

/// 长按"插入"：`value` 为显示值（PB/CC/RPN 按各自值域，Tempo 为 BPM）。
/// 成功 1。
pub fn insert_automation(kind: i32, number: i32, tick: f64, track: i32, value: f64) -> i32 {
    use yinhe_types::automation::{AutomationEvent, AutomationTarget};

    let Some(target) = insert_target(kind, number) else {
        return 0;
    };
    let is_tempo = matches!(target, AutomationTarget::Tempo);
    let track_idx = if is_tempo {
        0usize
    } else {
        track.max(0) as usize
    };
    let at_tick = tick.max(0.0).round() as u32;

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if !is_tempo {
        let Some(t) = doc.data.model.tracks.get(track_idx) else {
            return 0;
        };
        if t.kind == yinhe_core::TrackKind::Master {
            return 0;
        }
    }
    let display = value as f32;
    let mut normalized = target.from_display_value(display);
    if is_tempo {
        normalized = normalized.clamp(0.0, 60_000_000.0);
    } else {
        normalized = normalized.clamp(0.0, 1.0);
    }
    let event = AutomationEvent {
        id: 0,
        tick: at_tick,
        value: normalized,
        shape: target.default_shape(),
    };
    let before = doc.capture_snapshot();
    let Some((_, _, action)) = doc.upsert_automation_event(track_idx, target, event) else {
        return 0;
    };
    doc.push_undo(action, "插入自动化事件", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!(
        "插入自动化事件: kind={kind} number={number} tick={at_tick} track={track_idx} display={display:.1}"
    );
    1
}

/// 查询该位置当前显示值（PB/CC/RPN 原始值域；Tempo = BPM）。
/// 无历史值时返回 NaN，Dart 用目标默认值。
pub fn automation_display_value(kind: i32, number: i32, tick: f64, track: i32) -> f64 {
    use yinhe_types::automation::AutomationTarget;

    let Some(target) = insert_target(kind, number) else {
        return f64::NAN;
    };
    let Ok(s) = scene().lock() else {
        return f64::NAN;
    };
    let Some(doc) = s.document.as_ref() else {
        return f64::NAN;
    };
    let at_tick = tick.max(0.0).round() as u32;
    let normalized = if matches!(target, AutomationTarget::Tempo) {
        doc.data
            .model
            .conductor
            .tempo
            .value_at(at_tick)
            .map(|(v, _)| v)
    } else {
        doc.data
            .model
            .tracks
            .get(track.max(0) as usize)
            .and_then(|t| t.automation_lanes.iter().find(|l| l.target == target))
            .and_then(|l| l.value_at(at_tick))
            .map(|(v, _)| v)
    };
    match normalized {
        Some(v) => f64::from(target.to_display_value(v)),
        None => f64::NAN,
    }
}

/// 批量移动自动化锚点：`moves = [(old_tick, new_tick, value)]`。
///
/// `lane_index` 为 1-based（与 `automation_json` 输出顺序一致）；保留每个事件
/// 的 id / shape，一笔只产生一个 undo。成功返回 1。
pub fn move_automation_anchors(track: usize, lane_index: usize, moves: &[(u32, u32, f32)]) -> i32 {
    if moves.is_empty() {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let is_conductor = doc
        .data
        .model
        .tracks
        .get(track)
        .is_some_and(|t| t.kind == yinhe_core::TrackKind::Master);
    let (raw_idx, target, value_scale) = if is_conductor {
        // Conductor 的 AM 只有 Tempo；Dart 传归一化值，存盘时还原成 BPM。
        (
            0usize,
            yinhe_types::automation::AutomationTarget::Tempo,
            tempo_max_bpm(&doc.data.model),
        )
    } else {
        let Some((raw_idx, target)) = doc
            .data
            .model
            .tracks
            .get(track)
            .and_then(|t| visible_lane(t, lane_index))
        else {
            return 0;
        };
        (raw_idx, target, 1.0)
    };
    let scaled: Vec<(u32, u32, f32)> = if value_scale == 1.0 {
        moves.to_vec()
    } else {
        moves
            .iter()
            .map(|&(old, new, value)| (old, new, value * value_scale))
            .collect()
    };
    let target_track = if is_conductor { 0 } else { track };
    let before_snapshot = doc.capture_snapshot();
    let Some(action) = doc.move_automation_events_batch(target_track, raw_idx, &target, &scaled)
    else {
        return 0;
    };
    doc.push_undo(action, "移动自动化锚点", before_snapshot);
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    // 自动化变化：重建 CC（不中断播放）。
    crate::audio::reload_notes(model);
    log::info!(
        "移动自动化锚点: track={track} lane={lane_index} count={}",
        moves.len()
    );
    1
}

/// 删除指定 tick 的自动化锚点（AM 单击删除）。成功 1。
///
/// 复用选区删除逻辑：把要删的 tick 作为临时选择交给 `automation_selection_action`。
pub fn delete_automation_anchors(track: usize, lane_index: usize, ticks: &[u32]) -> i32 {
    if ticks.is_empty() {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let mut panel = s.automation_panel.clone();
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    panel.active_track = track;
    panel.lane = Some(lane_index);
    panel.selected_ticks = ticks.to_vec();
    panel.selected_ticks.sort_unstable();
    panel.selected_ticks.dedup();
    if !automation_selection_action(doc, &panel, 11) {
        return 0;
    }
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 确保指定 target 的自动化 lane 存在（不存在则创建空 lane）。成功 1。
///
/// AM 面板下拉用：`kind` 编码与 `insert_automation` 一致
/// （0=Tempo、1=PB、2=CC、3=RPN）；Tempo 不在轨 lane 里，返回 0。
pub fn ensure_automation_lane(track: usize, kind: i32, number: i32) -> i32 {
    let Some(target) = insert_target(kind, number) else {
        return 0;
    };
    if matches!(target, yinhe_types::automation::AutomationTarget::Tempo) {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let Some(t) = doc.data.model.tracks.get(track) else {
        return 0;
    };
    if t.automation_lanes.iter().any(|l| l.target == target) {
        return 1;
    }
    let before = doc.capture_snapshot();
    let Some((_lane_idx, action)) = doc.add_automation_lane(track, target) else {
        return 0;
    };
    doc.push_undo(action, "添加自动化", before);
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 自动化类型（与 Flutter/插入自动化编码一致）：1=PB、2=CC、3=RPN。
/// 注意不是 0/1/2；0 是 Tempo（AM 列表不展示 Tempo）。
pub(crate) fn lane_kind(target: &yinhe_types::automation::AutomationTarget) -> Option<u8> {
    use yinhe_types::automation::{AutomationTarget, ParamDevice, xsynth_param};
    match target {
        AutomationTarget::PitchBend => Some(1),
        AutomationTarget::CC { .. } => Some(2),
        AutomationTarget::Rpn { .. } => Some(3),
        AutomationTarget::Param {
            device: ParamDevice::ChannelInstrument { .. },
            id,
            ..
        } => match *id {
            xsynth_param::PITCH_BEND => Some(1),
            xsynth_param::PB_SENSITIVITY | xsynth_param::FINE_TUNE | xsynth_param::COARSE_TUNE => {
                Some(3)
            }
            _ => None,
        },
        _ => None,
    }
}

/// 可见 lane 的显示编号与上限：CC 号 / RPN 号，以及归一化值还原上限。
fn lane_display_range(target: &yinhe_types::automation::AutomationTarget) -> (i32, f32) {
    use yinhe_types::automation::AutomationTarget;
    let num = match target {
        AutomationTarget::CC { controller } => i32::from(*controller),
        AutomationTarget::Rpn { parameter } => i32::from(*parameter),
        _ => -1,
    };
    (num, target.display_max().unwrap_or(1.0))
}

/// 按 `automation_json` 的过滤顺序（1-based）查可见 lane 的原始下标与 target。
pub(super) fn visible_lane(
    track: &yinhe_core::TrackData,
    lane_index: usize,
) -> Option<(usize, yinhe_types::automation::AutomationTarget)> {
    // 与 Flutter/渲染层的 `state.lane` 一致：0 = 第一条可绘制自动化 lane
    //（Velocity 由 `panel.lane == None` 表示，不占下标）。
    let mut seen = 0usize;
    for (i, lane) in track.automation_lanes.iter().enumerate() {
        if lane_kind(&lane.target).is_some() {
            if seen == lane_index {
                return Some((i, lane.target.clone()));
            }
            seen += 1;
        }
    }
    None
}
