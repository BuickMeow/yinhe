use super::*;

/// 设置当前工具（0=选框 1=手形 2=铅笔 3=橡皮 4=刷子）。
pub fn set_tool(tool: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.tool = match tool {
        1 => Tool::Pan,
        2 => Tool::Pencil,
        3 => Tool::Eraser,
        4 => Tool::Brush,
        5 => Tool::Line,
        _ => Tool::Select,
    };
    1
}

/// 设置量化吸附档位：view 0=AR 1=PR；kind 0=Off 1=Auto 2=Fraction(num/den) 3=Absolute(ticks)。
pub fn set_quantize(view: i32, kind: i32, num: u32, den: u32) -> i32 {
    let preset = match kind {
        0 => QuantizePreset::Off,
        2 => QuantizePreset::Fraction(num.max(1), den.max(1)),
        3 => QuantizePreset::Absolute(num.max(1)),
        _ => QuantizePreset::Auto,
    };
    apply_quantize_preset(view, preset)
}

/// 按预设标签设置量化（`1/8`、`120 tick`、`Auto`、`Off`/`关闭`）。
///
/// Dart 不再解析标签，档位语义只有 Rust 一份。
pub fn set_quantize_label(view: i32, label: &str) -> i32 {
    let label = label.trim();
    let preset = match label {
        "Off" | "关闭" => QuantizePreset::Off,
        "Auto" => QuantizePreset::Auto,
        _ => {
            if let Some(rest) = label.strip_suffix(" tick") {
                match rest.trim().parse::<u32>() {
                    Ok(n) if n >= 1 => QuantizePreset::Absolute(n),
                    _ => return 0,
                }
            } else if let Some((num, den)) = label.split_once('/') {
                let num = num.trim().parse::<u32>().unwrap_or(0);
                let den = den.trim().parse::<u32>().unwrap_or(0);
                if num == 0 || den == 0 {
                    return 0;
                }
                QuantizePreset::Fraction(num, den)
            } else {
                return 0;
            }
        }
    };
    apply_quantize_preset(view, preset)
}

fn apply_quantize_preset(view: i32, preset: QuantizePreset) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if view == 1 {
        doc.edit.quantize_pianoroll = preset;
    } else {
        doc.edit.quantize_arrange = preset;
    }
    1
}

/// 当前视图量化档位的分数形式：`[num, den]`（非 Fraction 返回 `[0, 0]`）。
pub fn quantize_fraction(view: i32) -> (u32, u32) {
    let Ok(s) = scene().lock() else {
        return (0, 0);
    };
    let Some(doc) = s.document.as_ref() else {
        return (0, 0);
    };
    let preset = if view == 1 {
        doc.edit.quantize_pianoroll
    } else {
        doc.edit.quantize_arrange
    };
    match preset {
        QuantizePreset::Fraction(num, den) => (num, den),
        _ => (0, 0),
    }
}

/// 设置当前编辑轨（-1 = 无）。
pub fn set_active_track(track: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.active_track = if track < 0 {
        None
    } else {
        Some(track as usize)
    };
    log::info!("当前编辑轨: {track}");
    1
}

/// AR 长按拖动排序：把 `from` 轨移动到 `to` 位置（一步一 undo）。成功返回 1。
pub fn move_track(from: usize, to: usize) -> i32 {
    // Conductor（Master）固定在 0 号位，不参与排序。
    if from == 0 || to == 0 {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        let before = doc.capture_snapshot();
        let Some(action) = doc.move_track(from, to) else {
            return 0;
        };
        doc.push_undo(action, "undo.move_track", before);
    }
    // 面板索引随轨道顺序变化：活动/过滤轨同步 remap。
    remap_scene_track_index(&mut s, from, to);
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    let model = s.model.clone();
    drop(s);
    if let Some(model) = model {
        crate::audio::reload_notes(model);
    }
    crate::audio::set_skip_mask(mask);
    log::info!("轨道已移动: {from} -> {to}");
    1
}

/// 删除音轨（连同其音符；一步一 undo）。成功返回 1。
pub fn delete_track(idx: usize) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    {
        let Some(doc) = s.document.as_mut() else {
            return 0;
        };
        let before = doc.capture_snapshot();
        let Some(action) = doc.remove_track(idx) else {
            return 0;
        };
        doc.push_undo(action, "undo.delete_track", before);
    }
    let num = s
        .document
        .as_ref()
        .map(|d| d.data.model.tracks.len())
        .unwrap_or(0);
    // 活动/过滤轨越界或指向被删轨时收敛。
    let remap = |slot: &mut Option<usize>| {
        if let Some(t) = *slot {
            if t == idx {
                *slot = None;
            } else if t > idx {
                *slot = Some(t - 1);
            } else if t >= num {
                *slot = None;
            }
        }
    };
    remap(&mut s.active_track);
    remap(&mut s.display_track);
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    let model = s.model.clone();
    drop(s);
    if let Some(model) = model {
        crate::audio::reload_notes(model);
    }
    crate::audio::set_skip_mask(mask);
    log::info!("音轨已删除: {idx}");
    1
}

/// 轨道移动后把场景里的活动轨/过滤轨索引按同一规则 remap。
fn remap_scene_track_index(s: &mut RenderScene, from: usize, to: usize) {
    let map = |t: usize| -> usize {
        if t == from {
            to
        } else if from < to && t > from && t <= to {
            t - 1
        } else if from > to && t >= to && t < from {
            t + 1
        } else {
            t
        }
    };
    s.active_track = s.active_track.map(map);
    s.display_track = s.display_track.map(map);
}

/// 全选。`scope`: 0 = PR，1 = AR；`track < 0` = 该视图的全工程。
///
/// PR 单轨：选中该轨全部音符，选框覆盖 0→曲末、全音高。
/// AR 单轨：同上，并写 AR 选框，让 AR 视图画出整轨框。
pub fn select_all(scope: i32, track: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let arrange = scope == 1;
    let changed = match track {
        t if t >= 0 => select_track_all(doc, t as u16, arrange),
        _ if arrange => {
            doc.select_all_ar();
            true
        }
        _ => {
            doc.select_all_pr();
            true
        }
    };
    if !changed {
        return 0;
    }
    doc.edit.pianoroll_view.base.dirty = true;
    // 纯选区变化：不重建轨道/音符快照，也不 bump doc_generation
    //（否则 Dart 会全量重扫力度条与自动化）。
    selection_changed(&mut s);
    // 大选区松手/全选后立刻后台物化成员位图与拖动 ghost，按下拖动时直接复用。
    crate::selection_materializer::schedule(&s);
    1
}

/// 清空当前音符/AR 选区（顶栏空白点击）。返回 1 = 原本有选区。
pub fn clear_selection() -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if doc.edit.selected.is_empty()
        && doc.edit.sel_rect.rects.is_empty()
        && doc.edit.arr_sel_rect.is_empty()
        && doc.edit.chan_ar_sel.is_empty()
    {
        return 0;
    }
    doc.edit.selected.clear();
    doc.edit.sel_rect.clear();
    doc.edit.arr_sel_rect.clear();
    doc.edit.chan_ar_sel.clear();
    doc.edit.track_selected.clear();
    doc.edit.pianoroll_view.base.dirty = true;
    // 同 select_all：选区清空只刷新覆盖层；同时释放大选区的 ghost 缓存。
    crate::selection_materializer::invalidate();
    selection_changed(&mut s);
    1
}

/// 单轨全选（PR/AR 共用成员集，仅选框类型不同）。
fn select_track_all(doc: &mut Document, track: u16, arrange: bool) -> bool {
    use yinhe_types::MAX_KEY;
    let conductor = doc.edit.track_cache.conductor_idx;
    if Some(track) == conductor || doc.data.model.tracks.get(track as usize).is_none() {
        return false;
    }
    let max_end = doc.data.model.tick_length as u32;
    if max_end == 0 {
        return false;
    }
    doc.edit.track_selected.clear();
    doc.edit.selected.clear();
    doc.edit
        .selected
        .add_rect_track(0, max_end + 1, 0, MAX_KEY, track, track);
    if arrange {
        // AR 只写 AR 选框，避免切回 PR 时残留一个全轨 PR 选框。
        doc.edit.sel_rect.clear();
        doc.edit.arr_sel_rect = vec![(0.0, (max_end + 1) as f64, track as usize, track as usize)];
        doc.edit.chan_ar_sel = vec![yinhe_editor_core::edit_state::ChanSelRect {
            t_start: 0.0,
            t_end: (max_end + 1) as f64,
            tracks: vec![track],
        }];
    } else {
        doc.edit.track_selected.insert(track);
        doc.edit.sel_rect.rects = vec![(0.0, max_end as f64 + 1.0, 0, MAX_KEY)];
        doc.edit.sel_rect.auto_vertical = vec![false];
        doc.edit.arr_sel_rect.clear();
        doc.edit.chan_ar_sel.clear();
    }
    true
}

/// 人性化（可调幅度）：时间/力度/时值三路独立随机偏差。
///
/// 幅度全 0 = 不改。一次操作一个 undo。
pub fn humanize_selection(timing: u32, velocity: u32, gate: u32) -> i32 {
    use yinhe_editor_core::HumanizeParams;
    let params = HumanizeParams {
        timing,
        velocity: velocity.min(127) as u8,
        gate,
    };
    if params.is_noop() {
        return 0;
    }
    crate::doc_job::try_start(
        "humanize",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            let before = doc.capture_snapshot();
            let Some(action) = doc.humanize_selected(params) else {
                return false;
            };
            doc.push_undo(action, "人性化", before);
            doc.edit.pianoroll_view.base.dirty = true;
            true
        },
        None::<fn()>,
    )
}

/// 当前音符选区的 tick 范围 `(t0, t1)`；无选区返回 None。
pub fn selection_tick_range() -> Option<(f64, f64)> {
    let s = scene().lock().ok()?;
    let doc = s.document.as_ref()?;
    let (mut t0, mut t1) = (u64::MAX, 0u64);
    for &(ts, te, _, _, _, _) in &doc.edit.selected.rects {
        t0 = t0.min(ts as u64);
        t1 = t1.max(te as u64);
    }
    if t1 > t0 {
        Some((t0 as f64, t1 as f64))
    } else {
        None
    }
}

/// 解析批量处理输入框：空串 = 该字段不改；无效表达式 = 不改。
pub(super) fn field_ops(expr: &str) -> Option<Vec<yinhe_editor_core::num_expr::NumOp>> {
    if expr.trim().is_empty() {
        return None;
    }
    yinhe_editor_core::num_expr::parse_num_expr(expr)
}

/// 批量处理（音符）：每个字段一个表达式（`100`/`+2`/`x1.5`/`3/7`），
/// 空串或无效 = 跳过。`stretch_expr` 解析成倍率（`1.5`、`4/3`、`x2`）。
pub fn batch_edit(
    velocity_expr: &str,
    gate_expr: &str,
    key_expr: &str,
    tick_expr: &str,
    stretch_expr: &str,
) -> i32 {
    use yinhe_editor_core::document::note_edit::NoteField;
    use yinhe_editor_core::num_expr::apply_ops;

    let velocity_expr = velocity_expr.to_string();
    let gate_expr = gate_expr.to_string();
    let key_expr = key_expr.to_string();
    let tick_expr = tick_expr.to_string();
    let stretch_expr = stretch_expr.to_string();
    crate::doc_job::try_start(
        "batch-edit",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            if doc.edit.selected.is_empty() {
                return false;
            }
            let before = doc.capture_snapshot();
            let mut actions = Vec::new();

            // 变速：把表达式应用到 1.0 得到倍率，直接用 f64 算新跨度（只舍入一次，
            // 不再转整数比例，避免 1.3333333333 这类输入多一次误差）。
            let factor = field_ops(&stretch_expr)
                .map(|ops| apply_ops(&ops, 1.0))
                .unwrap_or(1.0);
            if factor.is_finite() && factor > 0.0 && (factor - 1.0).abs() > 1e-9 {
                let (mut t0, mut t1) = (u64::MAX, 0u64);
                for &(ts, te, _, _, _, _) in &doc.edit.selected.rects {
                    t0 = t0.min(ts as u64);
                    t1 = t1.max(te as u64);
                }
                let span = t1.saturating_sub(t0);
                if span > 0 {
                    let new_span = (span as f64 * factor).round().max(1.0) as u64;
                    if new_span != span
                        && let Some(a) = doc.rescale_selection_span(new_span)
                    {
                        actions.push(a);
                    }
                }
            }

            for (field, expr) in [
                (NoteField::Velocity, &velocity_expr),
                (NoteField::Gate, &gate_expr),
                (NoteField::Key, &key_expr),
                (NoteField::Tick, &tick_expr),
            ] {
                let Some(ops) = field_ops(expr) else {
                    continue;
                };
                if let Some(a) = doc.apply_note_field_edit(field, &ops) {
                    actions.push(a);
                }
            }
            if actions.is_empty() {
                return false;
            }
            let undo = match actions.len() {
                1 => actions.into_iter().next(),
                _ => Some(yinhe_editor_core::history::UndoAction::Composite(actions)),
            };
            if let Some(undo) = undo {
                doc.push_undo(undo, "批量处理", before);
            }
            doc.edit.pianoroll_view.base.dirty = true;
            true
        },
        None::<fn()>,
    )
}

/// 框选批量操作（编号与桌面 transport bar「操作」菜单一致）。
///
/// `scope`: 0 = 音符（PR/AR 共用 `edit.selected`），1 = 当前 AM lane 的锚点。
/// 返回 1 = 已执行，0 = 无变化或该操作不适用。
pub fn selection_action(scope: i32, action: i32) -> i32 {
    let (panel, view_mode) = {
        let Ok(s) = scene().lock() else {
            return 0;
        };
        if s.document.is_none() {
            return 0;
        }
        (s.automation_panel.clone(), s.view_mode)
    };
    crate::doc_job::try_start(
        "selection-action",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            let changed = if scope == 1 {
                automation_selection_action(doc, &panel, action)
            } else {
                note_selection_action(doc, action, view_mode)
            };
            if changed {
                doc.edit.pianoroll_view.base.dirty = true;
            }
            changed
        },
        None::<fn()>,
    )
}

/// 音符选区批量操作（PR/AR 共用）。
fn note_selection_action(doc: &mut Document, action: i32, view_mode: ViewMode) -> bool {
    use yinhe_editor_core::{FlipAxis, HumanizeParams};
    if doc.edit.selected.is_empty() {
        return false;
    }
    let before = doc.capture_snapshot();
    let arrangement = view_mode == ViewMode::Arrangement;
    let (label, undo) = match action {
        11 => (
            "删除选区",
            if arrangement {
                doc.delete_selected_with_automation()
            } else {
                doc.delete_selected()
            },
        ),
        10 => (
            "重复选区",
            if arrangement {
                doc.duplicate_selected_with_automation()
            } else {
                doc.duplicate_selected()
            },
        ),
        12 => ("升高八度", doc.transpose_selected(12)),
        13 => ("降低八度", doc.transpose_selected(-12)),
        19 => (
            "时值 ×2",
            if arrangement {
                doc.scale_selected_span_with_automation(2, 1)
            } else {
                doc.scale_selected_span(2, 1)
            },
        ),
        20 => (
            "时值 ÷2",
            if arrangement {
                doc.scale_selected_span_with_automation(1, 2)
            } else {
                doc.scale_selected_span(1, 2)
            },
        ),
        21 => (
            "水平翻转",
            if arrangement {
                doc.flip_selected_with_automation(FlipAxis::Horizontal)
            } else {
                doc.flip_selected_notes(FlipAxis::Horizontal)
            },
        ),
        22 => ("垂直翻转", doc.flip_selected_notes(FlipAxis::Vertical)),
        16 => (
            "人性化",
            doc.humanize_selected(HumanizeParams {
                timing: 8,
                velocity: 8,
                gate: 8,
            }),
        ),
        17 => ("填缝", doc.fill_gaps_selected()),
        18 => ("合并音符", doc.merge_adjacent_selected()),
        23 => (
            "网格切割",
            if view_mode == ViewMode::Arrangement {
                doc.split_selection_by_grid_arrangement()
            } else {
                doc.split_selection_by_grid()
            },
        ),
        _ => return false,
    };
    let Some(undo) = undo else {
        return false;
    };
    doc.push_undo(undo, label, before);
    // 网格切割会清空选框（与桌面一致）。
    if action == 23 {
        doc.edit.sel_rect.clear();
        doc.edit.arr_sel_rect.clear();
        doc.edit.selected.clear();
    }
    true
}

/// 当前力度摘要的键：与渲染线程构建 `VelocitySummary` 时的 `vs_key` 完全一致。
fn velocity_key(s: &RenderScene) -> u64 {
    let visible = crate::render::effective_visible(&s.pr_track_visible, s.display_track);
    let tv_hash = yinhe_wgpu::hash_bools(&visible);
    s.model_revision ^ tv_hash.rotate_left(32)
}

/// 力度条数据是否已就绪。
///
/// 高倍率走原始窗口扫描，不依赖摘要；低倍率必须等渲染线程的摘要键追上
/// 当前文档。Dart 在未就绪时延迟重试，而不是退回同步全量扫描。
pub fn velocity_ready() -> i32 {
    let Ok(s) = scene().lock() else {
        return 1;
    };
    if yinhe_wgpu::pianoroll::select_summary_level(s.pr_view.base.pixels_per_tick).is_none() {
        return 1;
    }
    i32::from(crate::velocity_cache::get(velocity_key(&s)).is_some())
}

/// PR 力度条数据（当前可见 tick 范围 + 可见轨）。
///
/// 扁平的 `[tick, length, velocity01, track, ...]` f64 数组，不再经过 JSON。
/// 低倍率直接切渲染线程后台预计算的摘要（与 GPU 同一份数据），不再在 UI
/// 线程同步扫全曲；高倍率可见音符本来就少，按窗口扫原始音符。
pub fn velocity_bars() -> Vec<f64> {
    let Ok(s) = scene().lock() else {
        return Vec::new();
    };
    let Some(doc) = s.document.as_ref() else {
        return Vec::new();
    };
    let model = &doc.data.model;
    let ppu = s.pr_view.base.pixels_per_tick;
    let (lw, lh) = crate::logical_size();
    let (scan_lo, scan_hi) = if lw > 0.0 {
        s.pr_view
            .visible_main_range(s.pr_view.main_axis_len(lw, lh))
    } else {
        (0.0, s.total_ticks.max(1.0))
    };
    let scan_lo_tick = scan_lo.max(0.0).floor() as u32;
    let scan_hi_tick = scan_hi.max(scan_lo + 1.0).ceil().min(u32::MAX as f64) as u32;

    let mut bars: Vec<(u32, u32, u8, u16)> = Vec::new();
    if yinhe_wgpu::pianoroll::select_summary_level(ppu).is_some() {
        let Some(summary) = crate::velocity_cache::get(velocity_key(&s)) else {
            return Vec::new();
        };
        let Some(level) = summary.level_for_ppu(ppu) else {
            return Vec::new();
        };
        for b in summary.level(level) {
            if b.start_tick > scan_hi_tick {
                break;
            }
            if b.end_tick <= scan_lo_tick {
                continue;
            }
            bars.push((
                b.start_tick,
                b.end_tick.saturating_sub(b.start_tick),
                b.velocity,
                b.track,
            ));
        }
    } else {
        let visible = crate::render::effective_visible(&s.pr_track_visible, s.display_track);
        for key in 0..=yinhe_types::MAX_KEY {
            for note in model.key_notes_in_range(key, scan_lo_tick, scan_hi_tick) {
                if note.velocity <= 1 {
                    continue;
                }
                if f64::from(note.end_tick) <= scan_lo || f64::from(note.start_tick) >= scan_hi {
                    continue;
                }
                if !visible.get(note.track as usize).copied().unwrap_or(true) {
                    continue;
                }
                bars.push((
                    note.start_tick,
                    note.end_tick.saturating_sub(note.start_tick),
                    note.velocity,
                    note.track,
                ));
            }
        }
    }
    bars.sort_unstable_by_key(|b| (b.0, b.3));
    let mut out = Vec::with_capacity(bars.len() * 4);
    for (tick, len, vel, track) in bars {
        out.push(f64::from(tick));
        out.push(f64::from(len));
        out.push((f64::from(vel) - 1.0) / 126.0);
        out.push(f64::from(track));
    }
    out
}

/// PR 力度栏笔划：按 `(tick, velocity)` 折线命中的音符力度批量修改。
///
/// 与桌面 automation 面板的 velocity 笔划同规则：只改当前编辑轨；有 PR 选框时
/// 只改选框内音符；一笔只产生一个 undo。返回 1 表示有实际修改。
pub fn velocity_stroke(points: &[(f64, f32)]) -> i32 {
    if points.is_empty() {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_ref() else {
        return 0;
    };
    let Some(track) = s.active_track.and_then(|t| u16::try_from(t).ok()) else {
        return 0;
    };
    if doc.edit.track_cache.conductor_idx == Some(track) {
        return 0;
    }

    let ppu = s.pr_view.base.pixels_per_tick.max(0.001);
    let hit_ticks = (2.0 / ppu) as f64;
    let model = &doc.data.model;
    let sel_rects = doc.edit.sel_rect.effective_rects();
    if !sel_rects.is_empty()
        && !doc.edit.track_selected.is_empty()
        && !doc.edit.track_selected.contains(&track)
    {
        return 0;
    }
    let has_sel = !sel_rects.is_empty();

    // (key, start_tick) → 新力度；同一力度条被笔划反复经过时以后经过的值为准。
    let mut touched: std::collections::HashMap<(u8, u32), u8> = std::collections::HashMap::new();
    let mut apply_segment = |a: (f64, f32), b: (f64, f32)| {
        let lo = a.0.min(b.0) - hit_ticks;
        let hi = a.0.max(b.0) + hit_ticks;
        let lo_u = lo.max(0.0).floor() as u32;
        let hi_u = hi.max(0.0).ceil() as u32;
        for key in 0..=yinhe_types::MAX_KEY {
            for note in model.key_notes_in_range(key, lo_u, hi_u.saturating_add(1)) {
                if note.track != track || note.start_tick < lo_u || note.start_tick > hi_u {
                    continue;
                }
                if has_sel {
                    let st = f64::from(note.start_tick);
                    let inside = sel_rects
                        .iter()
                        .any(|&(ts, te, kl, kh)| st >= ts && st < te && key >= kl && key <= kh);
                    if !inside {
                        continue;
                    }
                }
                let denom = b.0 - a.0;
                let f = if denom.abs() < f64::EPSILON {
                    0.0
                } else {
                    ((f64::from(note.start_tick) - a.0) / denom) as f32
                };
                let velocity = (a.1 + (b.1 - a.1) * f).round().clamp(1.0, 127.0) as u8;
                touched.insert((key, note.start_tick), velocity);
            }
        }
    };

    if points.len() == 1 {
        apply_segment(points[0], points[0]);
    } else {
        for pair in points.windows(2) {
            apply_segment(pair[0], pair[1]);
        }
    }
    if touched.is_empty() {
        return 0;
    }

    let edits: Vec<yinhe_types::VelocityEdit> = touched
        .into_iter()
        .map(|((key, start_tick), velocity)| yinhe_types::VelocityEdit {
            track,
            start_tick,
            key,
            velocity,
        })
        .collect();
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let Some(action) = doc.set_notes_velocity(&edits) else {
        return 0;
    };
    doc.push_undo(action, "力度绘制", before);
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    1
}

/// 复制当前选中音符到应用内剪贴板（快照 O(1)）。成功 1。
pub fn copy_selection() -> i32 {
    let Ok(s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_ref() else {
        return 0;
    };
    if doc.edit.selected.is_empty() {
        return 0;
    }
    let clipboard =
        NotesClipboard::from_snapshot(doc.data.model.clone(), doc.edit.selected.clone());
    drop(s);
    let Ok(mut slot) = CLIPBOARD.lock() else {
        return 0;
    };
    *slot = Some(clipboard);
    1
}

/// 剪贴板是否为空（Dart 菜单置灰用）。
pub fn clipboard_empty() -> bool {
    let Ok(slot) = CLIPBOARD.lock() else {
        return true;
    };
    match slot.as_ref() {
        Some(clipboard) => clipboard.is_empty(),
        None => true,
    }
}

/// 画布逻辑 x → 当前视图 tick（按当前量化档吸附，与点击跳转同规则）。
pub fn tick_at_x(s: &RenderScene, x: f32) -> f64 {
    let (ppu, scroll_x, origin) = match s.view_mode {
        ViewMode::Pianoroll => (
            s.pr_view.base.pixels_per_tick,
            s.pr_view.base.scroll_x,
            s.pr_view.keyboard_width(),
        ),
        ViewMode::Arrangement => (
            s.arr_view.base.pixels_per_tick,
            s.arr_view.base.scroll_x,
            s.arr_view.base.left_panel_width,
        ),
    };
    let raw = f64::from((x - origin + scroll_x) / ppu.max(0.0001)).max(0.0);
    let Some(doc) = s.document.as_ref() else {
        return raw;
    };
    let q = match s.view_mode {
        ViewMode::Pianoroll => doc.edit.quantize_pianoroll,
        ViewMode::Arrangement => doc.edit.quantize_arrange,
    };
    let (num, den_pow) = doc.data.model.tempo_map.time_sig_at_tick(raw as u32);
    let q = q.resolved_with_time_sig(
        ppu,
        s.ppq,
        num,
        den_pow,
        matches!(s.view_mode, ViewMode::Pianoroll),
    );
    SnapCtx::new(s)
        .round(q, raw)
        .clamp(0.0, (s.total_ticks - 1.0).max(0.0))
}

/// 在长按位置粘贴剪贴板（按当前量化吸附）。成功 1。
pub fn paste_selection(x: f32) -> i32 {
    let Some(clipboard) = CLIPBOARD.lock().ok().and_then(|slot| slot.clone()) else {
        return 0;
    };
    if clipboard.is_empty() {
        return 0;
    }
    // 先按当前视口算出长按位置对应的 tick（进入后台后视口可能变化）。
    let tick = {
        let Ok(s) = scene().lock() else {
            return 0;
        };
        if s.document.is_none() {
            return 0;
        }
        tick_at_x(&s, x)
    };
    crate::doc_job::try_start(
        "paste",
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            let track_selected = doc.edit.track_selected.clone();
            let before = doc.capture_snapshot();
            let Some((action, _span)) =
                doc.paste_notes(&clipboard, tick, &track_selected, PasteMode::AtCursor)
            else {
                return false;
            };
            doc.push_undo(action, "undo.paste", before);

            // paste_notes 只更新成员集；这里同步 PR/AR 选框，粘贴后可直接再次长按复制。
            let rects = doc.edit.selected.rects.clone();
            doc.edit.sel_rect.clear();
            doc.edit.arr_sel_rect.clear();
            for (t0, t1, klo, khi, trlo, trhi) in rects {
                doc.edit
                    .sel_rect
                    .push_rect((f64::from(t0), f64::from(t1), klo, khi), false);
                doc.edit.arr_sel_rect.push((
                    f64::from(t0),
                    f64::from(t1),
                    trlo as usize,
                    trhi as usize,
                ));
            }
            true
        },
        Some(move || set_play_cursor(tick)),
    )
}

/// 通道级默认乐器 JSON：16 项，`[bank_msb, bank_lsb, program, is_drum]` 或 null。
pub fn channel_instruments_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, inst) in doc.data.model.channel_instruments.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match inst {
            Some(v) => out.push_str(&format!(
                "[{},{},{},{}]",
                v.bank_msb,
                v.bank_lsb,
                v.program,
                u8::from(v.is_drum)
            )),
            None => out.push_str("null"),
        }
    }
    out.push(']');
    out
}

/// 轨道音频 skip 掩码：桌面同一份 mute/solo 规则（Solo 优先）+ 空音频轨。
fn skip_mask(s: &RenderScene) -> Vec<bool> {
    let Some(doc) = s.document.as_ref() else {
        return Vec::new();
    };
    // 与桌面 Document::compute_skip_mask 完全同源：有任意 solo 时忽略 mute，
    // 只放行 solo 轨；无 solo 时才按 muted 过滤。移动端只额外叠加空音频轨。
    let tracks = &doc.data.model.tracks;
    let mut mask = doc.compute_skip_mask();
    mask.resize(tracks.len(), false);
    for (i, t) in tracks.iter().enumerate() {
        if t.kind == yinhe_core::TrackKind::Audio && t.audio_clips.is_empty() {
            mask[i] = true;
        }
    }
    mask
}

fn toggle_override(track: usize, mute: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if track >= doc.data.model.tracks.len() {
        return 0;
    }
    while doc.edit.track_overrides.len() <= track {
        doc.edit.track_overrides.push(Default::default());
    }
    {
        let o = &mut doc.edit.track_overrides[track];
        if mute {
            o.muted = !o.muted;
        } else {
            o.soloed = !o.soloed;
        }
    }
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::set_skip_mask(mask);
    1
}

/// 切换轨道 mute（不参与 undo，与桌面一致）。成功 1。
pub fn toggle_track_mute(track: usize) -> i32 {
    toggle_override(track, true)
}

/// 切换轨道 solo（不参与 undo，与桌面一致）。成功 1。
pub fn toggle_track_solo(track: usize) -> i32 {
    toggle_override(track, false)
}

/// 新建 MIDI 轨：`cycle=true` 时在同一 port 内循环通道（B3→B16→B1），
/// 否则全部使用同一通道。成功 1。
pub fn add_tracks(count: usize, port: u8, start_channel: u8, cycle: bool) -> i32 {
    use yinhe_editor_core::NewTrackSpec;
    use yinhe_editor_core::channel_alloc;

    let count = count.clamp(1, 256);
    let alloc = if cycle {
        channel_alloc::alloc_channels_in_port(port, start_channel, count)
    } else {
        vec![(port & 0x0F, start_channel & 0x0F); count]
    };
    let specs: Vec<NewTrackSpec> = alloc
        .into_iter()
        .map(|(p, c)| NewTrackSpec {
            kind: yinhe_core::TrackKind::Midi,
            port: p,
            channel: c,
            audio_channel: None,
        })
        .collect();
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    let Some(action) = doc.add_tracks_batch(&specs) else {
        return 0;
    };
    doc.push_undo(action, "新建音轨", before);
    let model = doc.data.model.clone();
    let mask = skip_mask(&s);
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    crate::audio::set_skip_mask(mask);
    log::info!("新建音轨: {count} 条, port={port}, start={start_channel}, cycle={cycle}");
    1
}

/// 编辑音轨信息（名称/端口/通道 + 通道默认乐器）。成功 1。
#[allow(clippy::too_many_arguments)] // 对话框字段一一透传
pub fn set_track_info(
    track: usize,
    name: String,
    port: u8,
    channel: u8,
    bank_msb: u8,
    bank_lsb: u8,
    program: u8,
    is_drum: bool,
    use_instrument: bool,
) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let instrument = use_instrument.then_some(yinhe_core::ChannelInstrument {
        bank_msb,
        bank_lsb,
        program,
        is_drum,
    });
    let before = doc.capture_snapshot();
    let Some(action) = doc.set_track_info(track, name, port, channel, instrument) else {
        return 0;
    };
    doc.push_undo(action, "编辑音轨信息", before);
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!("音轨信息已更新: track={track}");
    1
}

/// 当前 PR 量化的 tick 间隔（Auto 按当前缩放解析；Off 回退 1）。
fn pr_quantize_interval(doc: &Document, cursor_tick: f64) -> u32 {
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let (num, den_pow) = model
        .tempo_map
        .time_sig_at_tick(cursor_tick.max(0.0) as u32);
    doc.edit
        .quantize_pianoroll
        .resolved_with_time_sig(
            doc.edit.pianoroll_view.base.pixels_per_tick,
            ppq,
            num,
            den_pow,
            true,
        )
        .tick_interval(ppq)
        .max(1)
}

/// 直线工具的插值点：`[t0, t1]` 上按量化间隔取 tick（含首尾）。
fn line_ticks(t0: f64, t1: f64, interval: u32) -> Vec<u32> {
    let a = t0.max(0.0).round() as u32;
    let b = t1.max(0.0).round() as u32;
    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
    let step = interval.max(1);
    let base = (lo / step) * step;
    let mut ticks = Vec::new();
    let mut t = base;
    while t <= hi {
        if t >= lo {
            ticks.push(t);
        }
        t = match t.checked_add(step) {
            Some(next) => next,
            None => break,
        };
    }
    if ticks.last().copied() != Some(b) {
        ticks.push(b);
    }
    ticks
}

/// AM 直线：按当前 PR 量化在起止点之间插入自动化锚点（一次 undo）。
///
/// `v0/v1` 为归一化 0..1（Tempo 额外乘当前最大 BPM）。线段形状统一为直线，
/// 这样 AM 里看到的是一条直斜线而不是台阶。
pub fn automation_line(
    kind: i32,
    number: i32,
    track: i32,
    t0: f64,
    t1: f64,
    v0: f64,
    v1: f64,
) -> i32 {
    use yinhe_types::automation::{AutomationEvent, AutomationTarget, SegmentShape};

    let Some(target) = insert_target(kind, number) else {
        return 0;
    };
    let is_tempo = matches!(target, AutomationTarget::Tempo);
    let track_idx = if is_tempo { 0 } else { track.max(0) as usize };
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let cursor_tick = s.cursor_tick;
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    if !is_tempo
        && doc
            .data
            .model
            .tracks
            .get(track_idx)
            .is_none_or(|t| t.kind == yinhe_core::TrackKind::Master)
    {
        return 0;
    }
    let interval = pr_quantize_interval(doc, cursor_tick);
    let ticks = line_ticks(t0, t1, interval);
    if ticks.is_empty() {
        return 0;
    }
    let a = t0.max(0.0).round() as u32;
    let b = t1.max(0.0).round() as u32;
    let span = f64::from(b.abs_diff(a)).max(1.0);
    let tempo_max = if is_tempo {
        tempo_max_bpm(&doc.data.model)
    } else {
        1.0
    };
    let events: Vec<AutomationEvent> = ticks
        .iter()
        .map(|&tick| {
            let f = if a == b {
                0.0
            } else {
                (f64::from(tick) - f64::from(a)) / span * if a <= b { 1.0 } else { -1.0 }
            };
            let value = (v0 + (v1 - v0) * f.clamp(0.0, 1.0)) as f32;
            let value = if is_tempo {
                (value * tempo_max).max(1.0)
            } else {
                value.clamp(0.0, 1.0)
            };
            AutomationEvent {
                id: 0,
                tick,
                value,
                shape: SegmentShape::Curve {
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                },
            }
        })
        .collect();
    let before_snapshot = doc.capture_snapshot();
    let Some(action) = doc.upsert_automation_events_batch(track_idx, target.clone(), &events)
    else {
        return 0;
    };
    doc.push_undo(action, "直线绘制自动化", before_snapshot);
    doc.edit.pianoroll_view.base.dirty = true;
    let model = doc.data.model.clone();
    refresh_snapshot(&mut s);
    drop(s);
    crate::audio::reload_notes(model);
    log::info!(
        "直线绘制自动化: kind={kind} number={number} track={track_idx} points={}",
        events.len()
    );
    1
}

/// AM 直线力度：把力度条按起止点线性插值（一次 undo）。
///
/// 力度条的"插入点"就是已有音符，密度由音符自身决定，不需要按量化补点。
pub fn velocity_line(t0: f64, v0: f64, t1: f64, v1: f64) -> i32 {
    let to_vel = |v: f64| (1.0 + 126.0 * v.clamp(0.0, 1.0)) as f32;
    let points = [(t0, to_vel(v0)), (t1, to_vel(v1))];
    velocity_stroke(&points)
}

/// 重命名音轨（AR 长按轨道菜单）。成功 1。/// 重命名音轨（AR 长按轨道菜单）。成功 1。
pub fn set_track_name(track: usize, name: String) -> i32 {
    use yinhe_editor_core::history::UndoAction;

    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let old = {
        let Some(doc) = s.document.as_ref() else {
            return 0;
        };
        let Some(t) = doc.data.model.tracks.get(track) else {
            return 0;
        };
        t.name.clone()
    };
    if old == name {
        return 0;
    }
    let Some(doc) = s.document.as_mut() else {
        return 0;
    };
    let before = doc.capture_snapshot();
    {
        let model = std::sync::Arc::make_mut(&mut doc.data.model);
        if let Some(t) = model.tracks.get_mut(track) {
            std::sync::Arc::make_mut(t).name = name.clone();
        }
        // track 0 name = 歌曲标题（与桌面 apply 一致）。
        if track == 0 {
            model.meta.name = name.clone();
        }
    }
    doc.push_undo(
        UndoAction::TrackName {
            track_idx: track,
            old,
            new: name,
        },
        "undo.track_name",
        before,
    );
    refresh_snapshot(&mut s);
    log::info!("音轨已重命名: track={track}");
    1
}

/// 撤销一步（含音频与快照刷新）。成功 1（已启动后台任务）。
pub fn undo() -> i32 {
    history_step(false)
}

/// 重做一步（含音频与快照刷新）。成功 1（已启动后台任务）。
pub fn redo() -> i32 {
    history_step(true)
}

fn history_step(redo: bool) -> i32 {
    crate::doc_job::try_start(
        if redo { "redo" } else { "undo" },
        crate::doc_job::Reload::Notes,
        move |doc, _job| {
            // 历史重放会换掉选区引用的音符：先作废后台物化/ghost 缓存。
            crate::selection_materializer::invalidate();
            if redo { doc.redo() } else { doc.undo() }
        },
        None::<fn()>,
    )
}
