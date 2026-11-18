use super::*;

/// 标尺标签（marker）JSON：`[{"tick":..,"text":".."}]`。
pub fn markers_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, m) in doc.data.model.conductor.markers.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"tick\":{},\"text\":\"{}\"}}",
            m.tick,
            json_escape(&m.text)
        ));
    }
    out.push(']');
    out
}

/// 走带循环状态：`{"enabled":bool,"start":tick,"end":tick}`。
pub fn loop_json() -> String {
    let Ok(s) = scene().lock() else {
        return r#"{"enabled":false,"start":0,"end":0}"#.into();
    };
    let Some(doc) = s.document.as_ref() else {
        return r#"{"enabled":false,"start":0,"end":0}"#.into();
    };
    let playback = &doc.edit.playback;
    let (start, end) = playback.loop_range();
    serde_json::json!({
        "enabled": playback.loop_enabled(),
        "start": start,
        "end": end,
    })
    .to_string()
}

/// 当前文档是否开启循环（播放到末尾时不要自动暂停）。
pub fn loop_enabled() -> bool {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.document.as_ref().map(|d| d.edit.playback.loop_enabled()))
        .unwrap_or(false)
}

/// 切换走带循环。开启时若区间无效，以光标所在小节为起点生成四小节。
/// 返回切换后的开关状态（1 = 开）。
pub fn loop_toggle() -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let cursor = s.cursor_tick.max(0.0) as u32;
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let enabled = !doc.edit.playback.loop_enabled();
    if enabled {
        let (start, end) = doc.edit.playback.loop_range();
        if end <= start {
            let bar = doc.data.model.tempo_map.bar_divide().max(1.0) as u32;
            let start = cursor / bar * bar;
            let _ = doc
                .edit
                .playback
                .set_loop_range(start, start.saturating_add(bar.saturating_mul(4)));
        }
    }
    if doc.edit.playback.set_loop_enabled(enabled) {
        i32::from(doc.edit.playback.loop_enabled())
    } else {
        0
    }
}

/// 直接设置循环开关（EVENT 菜单用）。返回 1 = 成功。
pub fn loop_set_enabled(enabled: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let cursor = s.cursor_tick.max(0.0) as u32;
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if enabled {
        let (start, end) = doc.edit.playback.loop_range();
        if end <= start {
            let bar = doc.data.model.tempo_map.bar_divide().max(1.0) as u32;
            let start = cursor / bar * bar;
            let _ = doc
                .edit
                .playback
                .set_loop_range(start, start.saturating_add(bar.saturating_mul(4)));
        }
    }
    i32::from(doc.edit.playback.set_loop_enabled(enabled))
}

/// 设置循环区间（拖拽 LOOP 行）。成功 1。
pub fn loop_set_range(start: f64, end: f64) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let start = start.max(0.0).min(u32::MAX as f64) as u32;
    let end = end.max(0.0).min(u32::MAX as f64) as u32;
    i32::from(doc.edit.playback.set_loop_range(start, end))
}

/// 全局事件行数据（AR 固定行）：`{"markers":[…],"meter":[…],"key":[…],
/// "chord":[…],"lyrics":[…]}`，每项 `{"tick","text"}`。
///
/// 拍号为空时补一个 tick 0 的工程默认拍号，保证 METER 行始终能显示。
pub fn event_rows_json() -> String {
    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "{}".into();
    };
    let model = &doc.data.model;
    let conductor = &model.conductor;
    let text_events = |iter: &mut dyn Iterator<Item = (u32, String)>| {
        iter.map(|(tick, text)| serde_json::json!({"tick": tick, "text": text}))
            .collect::<Vec<_>>()
    };

    let meter = if conductor.time_sig.is_empty() {
        let (num, den) = model.tempo_map.time_sig_default;
        vec![serde_json::json!({
            "tick": 0,
            "text": format!("{num}/{}", 1u32 << u32::from(den.min(5))),
        })]
    } else {
        text_events(&mut conductor.time_sig.iter().map(|e| {
            (
                e.tick,
                format!(
                    "{}/{}",
                    e.numerator,
                    1u32 << u32::from(e.denominator.min(5))
                ),
            )
        }))
    };
    let key = text_events(&mut conductor.key_sig.iter().map(|e| {
        (
            e.tick,
            format!(
                "{} {}",
                yinhe_types::NOTE_NAMES[(e.root % 12) as usize],
                e.scale.english_name()
            ),
        )
    }));
    let markers = text_events(&mut conductor.markers.iter().map(|e| (e.tick, e.text.clone())));
    let chord = text_events(&mut conductor.chord.iter().map(|e| (e.tick, e.text.clone())));
    let lyrics = text_events(&mut conductor.lyrics.iter().map(|e| (e.tick, e.text.clone())));

    serde_json::json!({
        "markers": markers,
        "meter": meter,
        "key": key,
        "chord": chord,
        "lyrics": lyrics,
    })
    .to_string()
}

/// 在 tick 处插入一个标签（空文本 = 显示小节位置）。成功 1。
pub fn marker_insert(tick: f64, text: String) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.insert_marker_with_undo(t, text, "undo.insert_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 重命名标签。成功 1。
pub fn marker_rename(tick: f64, text: String) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.set_marker_with_undo(t, t, text, "undo.rename_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 删除标签。成功 1。
pub fn marker_delete(tick: f64) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let t = tick.max(0.0).round() as u32;
    if !doc.delete_marker_with_undo(t, "undo.delete_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 移动标签：画布逻辑 x 决定目标 tick（按当前量化吸附）。成功 1。
pub fn marker_move(from: f64, x: f32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let to_tick = tick_at_x(&s, x).round() as u32;
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let from_tick = from.max(0.0).round() as u32;
    let text = doc
        .data
        .model
        .conductor
        .markers
        .iter()
        .find(|m| m.tick == from_tick)
        .map(|m| m.text.clone())
        .unwrap_or_default();
    if !doc.set_marker_with_undo(from_tick, to_tick, text, "undo.move_marker_event") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 在 conductor 指定 tick 插入拍号事件（一步一 undo）。成功 1。
pub fn insert_time_sig(tick: f64, numerator: u8, denominator_power: u8) -> i32 {
    use yinhe_editor_core::history::{EventListDelta, EventListItem, EventListTarget, UndoAction};
    let tick = tick.max(0.0) as u32;
    let numerator = numerator.clamp(1, 32);
    let denominator_power = denominator_power.min(6);
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.data.model.conductor.time_sig.clone();
    let snapshot = doc.capture_snapshot();
    doc.insert_time_sig_event(tick, numerator, denominator_power);
    let after = doc.data.model.conductor.time_sig.clone();
    doc.push_undo(
        UndoAction::EventList(EventListDelta {
            target: EventListTarget::TimeSig,
            old: before.into_iter().map(EventListItem::TimeSig).collect(),
            new: after.into_iter().map(EventListItem::TimeSig).collect(),
        }),
        "插入拍号",
        snapshot,
    );
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!(
        "插入拍号: tick={tick} {numerator}/{}",
        1u32 << denominator_power
    );
    1
}

/// 在 conductor 指定 tick 插入调号事件（一步一 undo）。成功 1。
pub fn insert_key_sig(tick: f64, root: u8, scale: i32) -> i32 {
    use yinhe_editor_core::history::{EventListDelta, EventListItem, EventListTarget, UndoAction};
    use yinhe_types::ScaleType;
    let tick = tick.max(0.0) as u32;
    let root = root.min(11);
    let scale = match scale {
        1 => ScaleType::NaturalMinor,
        2 => ScaleType::HarmonicMinor,
        3 => ScaleType::MelodicMinor,
        _ => ScaleType::Major,
    };
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.data.model.conductor.key_sig.clone();
    let snapshot = doc.capture_snapshot();
    doc.insert_key_sig_event(tick, root, scale);
    let after = doc.data.model.conductor.key_sig.clone();
    doc.push_undo(
        UndoAction::EventList(EventListDelta {
            target: EventListTarget::KeySig,
            old: before.into_iter().map(EventListItem::KeySig).collect(),
            new: after.into_iter().map(EventListItem::KeySig).collect(),
        }),
        "插入调号",
        snapshot,
    );
    refresh_snapshot(&mut s);
    log::info!("插入调号: tick={tick} root={root} scale={scale:?}");
    1
}

// =========================================================
//  事件列表（移动端：音符 + 自动化 + 全局事件穿插）
// =========================================================

/// 事件行（收集阶段只存字段，排序后再生成 JSON）。
enum EventRow {
    Note {
        track: u16,
        tick: u32,
        key: u8,
        vel: u8,
        end: u32,
        id: u32,
    },
    Auto {
        track: u16,
        tick: u32,
        lane: usize,
        /// 与插入菜单一致：1=PB 2=CC 3=RPN（0 留给 Tempo）。
        kind: u8,
        number: i32,
        display: f32,
        max: f32,
    },
    Tempo {
        tick: u32,
        bpm: f32,
    },
    Marker {
        tick: u32,
        text: String,
    },
}

impl EventRow {
    fn tick(&self) -> u32 {
        match self {
            EventRow::Note { tick, .. }
            | EventRow::Auto { tick, .. }
            | EventRow::Tempo { tick, .. }
            | EventRow::Marker { tick, .. } => *tick,
        }
    }

    /// 次级排序键：音符按音高、自动化按显示值、全局事件最后。
    fn secondary(&self) -> f32 {
        match self {
            EventRow::Note { key, .. } => f32::from(*key),
            EventRow::Auto { display, .. } => 128.0 + display,
            _ => 400.0,
        }
    }

    /// 三级排序键：音符时长；其它 0。
    fn duration(&self) -> u32 {
        match self {
            EventRow::Note { tick, end, .. } => end.saturating_sub(*tick),
            _ => 0,
        }
    }
}

/// 事件列表总行数（与 [`events_json`] 的收集/过滤规则一致）。
///
/// 音符数用缓存的 `note_count` / `track_note_count`，自动化只数事件条数，
/// 因此即使用于大工程也只是 O(轨道数 + 自动化事件数)。
fn events_total_for(model: &yinhe_core::YinModel, track: i32) -> u64 {
    use yinhe_types::automation::AutomationTarget;
    let notes = if track < 0 {
        model.note_count
    } else {
        model
            .track_note_count
            .get(track as usize)
            .copied()
            .unwrap_or(0)
    };
    let mut total = notes;
    for (ti, t) in model.tracks.iter().enumerate() {
        if track >= 0 && ti as i32 != track {
            continue;
        }
        for lane in &t.automation_lanes {
            if matches!(
                lane.target,
                AutomationTarget::PitchBend
                    | AutomationTarget::CC { .. }
                    | AutomationTarget::Rpn { .. }
            ) {
                total += lane.events.len() as u64;
            }
        }
    }
    // Conductor 事件只在“全部轨道”或选中 Conductor（track 0）时显示。
    if track <= 0 {
        total += model.conductor.tempo.events.len() as u64;
        total += model.conductor.markers.len() as u64;
    }
    total
}

/// 事件列表：从 `from_tick` 起收集（音符/自动化/Tempo/标签），
/// 按 (tick, 音高或值, 时长) 排序后取前 `limit` 条。
///
/// `track`：-1 = 全部；其它 = 只收该轨（track 0 仍包含 Conductor 事件）。
/// JSON：`{"total":N,"rows":[ ... ]}`；`total` 是当前筛选下的真实总行数。
pub fn events_json(from_tick: f64, limit: i32, track: i32, skip: u32) -> String {
    use yinhe_types::automation::AutomationTarget;

    let from = from_tick.max(0.0) as u32;
    let limit = limit.clamp(50, 2000) as usize;
    let skip = skip as usize;
    // 多取一行用于判断是否还有下一页；skip 只会在同一 tick 内累加，
    // 因此取 skip+limit+1 足以覆盖当前页及下一页的边界。
    let take = limit.saturating_add(skip).saturating_add(1);
    let Ok(s) = scene().lock() else {
        return r#"{"total":0,"rows":[]}"#.into();
    };
    let Some(doc) = s.document.as_ref() else {
        return r#"{"total":0,"rows":[]}"#.into();
    };
    let model = &doc.data.model;
    let total = events_total_for(model, track);
    let mut rows: Vec<EventRow> = Vec::new();

    // 音符：每个 key 取 from 之后的前 limit 个（区间迭代，块内二分）。
    for key in 0..=yinhe_types::MAX_KEY {
        for n in model
            .key_notes(key)
            .range(from, u32::MAX)
            .filter(|n| track < 0 || n.track as i32 == track)
            .take(take)
        {
            rows.push(EventRow::Note {
                track: n.track,
                tick: n.start_tick,
                key,
                vel: n.velocity,
                end: n.end_tick,
                id: n.id,
            });
        }
    }

    // 普通轨自动化（PB/CC/RPN）。
    for (ti, t) in model.tracks.iter().enumerate() {
        if track >= 0 && ti as i32 != track {
            continue;
        }
        for (li, lane) in t.automation_lanes.iter().enumerate() {
            let kind = match &lane.target {
                AutomationTarget::PitchBend => 1u8,
                AutomationTarget::CC { .. } => 2,
                AutomationTarget::Rpn { .. } => 3,
                _ => continue,
            };
            let number = match &lane.target {
                AutomationTarget::CC { controller } => i32::from(*controller),
                AutomationTarget::Rpn { parameter } => i32::from(*parameter),
                _ => 0,
            };
            let start = lane.events.partition_point(|e| e.tick < from);
            let max = lane.target.to_display_value(1.0);
            for e in lane.events[start..].iter().take(take) {
                rows.push(EventRow::Auto {
                    track: ti as u16,
                    tick: e.tick,
                    lane: li,
                    kind,
                    number,
                    display: lane.target.to_display_value(e.value),
                    max,
                });
            }
        }
    }

    // Conductor：Tempo + 标签（全部轨道或选中 Conductor 时）。
    if track <= 0 {
        let lane = &model.conductor.tempo;
        let start = lane.events.partition_point(|e| e.tick < from);
        for e in lane.events[start..].iter().take(take) {
            rows.push(EventRow::Tempo {
                tick: e.tick,
                bpm: e.value,
            });
        }
        for m in model
            .conductor
            .markers
            .iter()
            .filter(|m| m.tick >= from)
            .take(take)
        {
            rows.push(EventRow::Marker {
                tick: m.tick,
                text: m.text.clone(),
            });
        }
    }

    rows.sort_by(|a, b| {
        a.tick()
            .cmp(&b.tick())
            .then(a.secondary().total_cmp(&b.secondary()))
            .then(a.duration().cmp(&b.duration()))
    });
    if rows.len() <= skip {
        return format!("{{\"total\":{total},\"rows\":[],\"next\":null}}");
    }
    let end = (skip + limit).min(rows.len());
    let page = &rows[skip..end];
    let next = if rows.len() > end {
        let next_tick = rows[end].tick();
        // 下一页从 next_tick 的第 next_skip 个事件开始（跳过本页已消费的同 tick 事件）。
        let next_skip = rows[..end].iter().filter(|r| r.tick() == next_tick).count();
        Some((next_tick, next_skip))
    } else {
        None
    };

    let mut out = format!("{{\"total\":{total},\"rows\":[");
    for (i, row) in page.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&event_row_json(row, model));
    }
    out.push_str("],\"next\":");
    match next {
        Some((tick, next_skip)) => {
            out.push_str(&format!("{{\"tick\":{tick},\"skip\":{next_skip}}}"));
        }
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

/// 单行 JSON：`k`=note/cc/pb/rpn/tempo/marker；`t`=tick；`tt`=小节.拍.tick；`tr`=轨号；`tn`=轨名。
fn event_row_json(row: &EventRow, model: &YinModel) -> String {
    let track_name = |track: u16| {
        model
            .tracks
            .get(track as usize)
            .map(|t| t.name.clone())
            .unwrap_or_default()
    };
    let tt = json_escape(&bar_beat_text_for_model(model, row.tick() as f64));
    match row {
        EventRow::Note {
            track,
            tick,
            key,
            vel,
            end,
            id,
        } => format!(
            "{{\"k\":\"note\",\"t\":{tick},\"tt\":\"{tt}\",\"tr\":{track},\"tn\":\"{}\",\"key\":{key},\"vel\":{vel},\"end\":{end},\"id\":{id}}}",
            json_escape(&track_name(*track))
        ),
        EventRow::Auto {
            track,
            tick,
            lane,
            kind,
            number,
            display,
            max,
            ..
        } => {
            let k = match kind {
                1 => "pb",
                2 => "cc",
                _ => "rpn",
            };
            format!(
                "{{\"k\":\"{k}\",\"t\":{tick},\"tt\":\"{tt}\",\"tr\":{track},\"tn\":\"{}\",\"lane\":{lane},\"kind\":{kind},\"num\":{number},\"val\":{display:.3},\"max\":{max:.3}}}",
                json_escape(&track_name(*track))
            )
        }
        EventRow::Tempo { tick, bpm } => format!(
            "{{\"k\":\"tempo\",\"t\":{tick},\"tt\":\"{tt}\",\"tr\":0,\"tn\":\"Conductor\",\"val\":{bpm:.3},\"max\":60000000}}"
        ),
        EventRow::Marker { tick, text } => format!(
            "{{\"k\":\"marker\",\"t\":{tick},\"tt\":\"{tt}\",\"tr\":0,\"tn\":\"Conductor\",\"text\":\"{}\"}}",
            json_escape(text)
        ),
    }
}

/// 删除事件行。成功 1。
pub fn event_delete(json: &str) -> i32 {
    use yinhe_types::automation::AutomationTarget;
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(kind) = event_field(&v, "k").and_then(|x| x.as_str()) else {
        return 0;
    };
    let tick = event_field(&v, "t").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let track = event_field(&v, "tr").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let action = match kind {
        "note" => {
            let key = event_field(&v, "key").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
            doc.delete_notes(&[(track as u16, tick, key)])
        }
        "cc" | "pb" | "rpn" => {
            let lane = event_field(&v, "lane")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as usize;
            let num = event_field(&v, "num").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let kind = event_field(&v, "kind")
                .and_then(|x| x.as_i64())
                .unwrap_or(0) as i32;
            let Some(target) = insert_target(kind, num) else {
                return 0;
            };
            doc.delete_automation_event(track, lane, &target, tick)
        }
        "tempo" => doc.delete_automation_event(0, 0, &AutomationTarget::Tempo, tick),
        "marker" => {
            let ok = doc.delete_marker_with_undo(tick, "删除标签");
            let model = doc.data.model.clone();
            refresh_snapshot(&mut s);
            drop(s);
            crate::audio::reload_notes(model);
            return i32::from(ok);
        }
        _ => None,
    };
    let Some(action) = action else {
        return 0;
    };
    doc.push_undo(action, "事件列表删除", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!("事件列表删除: {kind} tick={tick} track={track}");
    1
}

/// 修改事件：音符 = (力度, 结束 tick)；自动化 = 显示值；标签走 `event_set_text`。
pub fn event_update(json: &str, value: f64, gate: f64) -> i32 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(kind) = event_field(&v, "k").and_then(|x| x.as_str()) else {
        return 0;
    };
    let tick = event_field(&v, "t").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let track = event_field(&v, "tr").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let mut actions: Vec<yinhe_editor_core::history::UndoAction> = Vec::new();
    match kind {
        "note" => {
            let key = event_field(&v, "key").and_then(|x| x.as_u64()).unwrap_or(0) as u8;
            let id = event_field(&v, "id").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let vel = value.round().clamp(1.0, 127.0) as u8;
            let end = gate.round().max(1.0) as u32;
            if let Some(a) = doc.set_note_velocity(track as u16, tick, key, vel) {
                actions.push(a);
            }
            if let Some(a) = doc.set_note_end_tick(key, id, end) {
                actions.push(a);
            }
        }
        "cc" | "pb" | "rpn" => {
            let num = event_field(&v, "num").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let kind = event_field(&v, "kind")
                .and_then(|x| x.as_i64())
                .unwrap_or(0) as i32;
            let Some(target) = insert_target(kind, num) else {
                return 0;
            };
            let normalized = target.from_display_value(value as f32);
            let event = yinhe_types::automation::AutomationEvent {
                id: 0,
                tick,
                value: normalized.clamp(0.0, 1.0),
                shape: target.default_shape(),
            };
            let Some((_, _, a)) = doc.upsert_automation_event(track, target, event) else {
                return 0;
            };
            actions.push(a);
        }
        "tempo" => {
            let event = yinhe_types::automation::AutomationEvent {
                id: 0,
                tick,
                value: (value as f32).clamp(0.0, 60_000_000.0),
                shape: yinhe_types::SegmentShape::Step,
            };
            let Some((_, _, a)) = doc.upsert_automation_event(
                0,
                yinhe_types::automation::AutomationTarget::Tempo,
                event,
            ) else {
                return 0;
            };
            actions.push(a);
        }
        _ => return 0,
    }
    if actions.is_empty() {
        return 0;
    }
    let action = if actions.len() == 1 {
        actions.pop().unwrap()
    } else {
        yinhe_editor_core::history::UndoAction::Composite(actions)
    };
    doc.push_undo(action, "事件列表修改", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 修改标签文本。成功 1。
pub fn event_set_text(json: &str, text: String) -> i32 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let tick = event_field(&v, "t").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if !doc.set_marker_with_undo(tick, tick, text, "修改标签") {
        return 0;
    }
    refresh_snapshot(&mut s);
    1
}

/// 把 `source` 行复制到 `target` 行的时间位置 / 音轨（粘贴）。
pub fn event_paste(source: &str, target: &str) -> i32 {
    use yinhe_types::automation::AutomationEvent;

    let (Ok(src), Ok(dst)) = (
        serde_json::from_str::<serde_json::Value>(source),
        serde_json::from_str::<serde_json::Value>(target),
    ) else {
        return 0;
    };
    let Some(sk) = event_field(&src, "k").and_then(|x| x.as_str()) else {
        return 0;
    };
    let target_tick = event_field(&dst, "t").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
    let target_track = event_field(&dst, "tr")
        .and_then(|x| x.as_u64())
        .unwrap_or(0) as usize;
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let action = match sk {
        "note" => {
            let key = event_field(&src, "key")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u8;
            let vel = event_field(&src, "vel")
                .and_then(|x| x.as_u64())
                .unwrap_or(100) as u8;
            let src_tick = event_field(&src, "t").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
            let end = event_field(&src, "end")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as u32;
            let len = end.saturating_sub(src_tick).max(1);
            let note = yinhe_core::NoteEvent {
                id: 0,
                key,
                start_tick: target_tick,
                end_tick: target_tick.saturating_add(len),
                velocity: vel,
            };
            doc.add_note(target_track as u16, note)
        }
        "cc" | "pb" | "rpn" | "tempo" => {
            let kind = if sk == "tempo" {
                0
            } else {
                event_field(&src, "kind")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0) as i32
            };
            let num = event_field(&src, "num")
                .and_then(|x| x.as_i64())
                .unwrap_or(0) as i32;
            let Some(target_ev) = insert_target(kind, num) else {
                return 0;
            };
            let display = event_field(&src, "val")
                .and_then(|x| x.as_f64())
                .unwrap_or(0.0) as f32;
            let value = if kind == 0 {
                display.clamp(0.0, 60_000_000.0)
            } else {
                target_ev.from_display_value(display).clamp(0.0, 1.0)
            };
            let event = AutomationEvent {
                id: 0,
                tick: target_tick,
                value,
                shape: target_ev.default_shape(),
            };
            let track_idx = if kind == 0 { 0 } else { target_track };
            doc.upsert_automation_event(track_idx, target_ev, event)
                .map(|(_, _, a)| a)
        }
        "marker" => {
            let text = event_field(&src, "text")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if doc.insert_marker_with_undo(target_tick, text.to_string(), "粘贴标签") {
                let model = doc.data.model.clone();
                refresh_snapshot(&mut s);
                drop(s);
                crate::audio::reload_notes(model);
                return 1;
            }
            None
        }
        _ => None,
    };
    let Some(action) = action else {
        return 0;
    };
    doc.push_undo(action, "事件列表粘贴", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 解析事件行 JSON 的公共字段。
fn event_field<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    v.get(key)
}
