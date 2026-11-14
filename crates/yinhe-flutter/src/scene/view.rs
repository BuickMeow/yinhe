use super::*;

/// 覆盖层修订号（Dart 仅在变化时重取 JSON）。
pub fn overlay_revision() -> u32 {
    scene().lock().map(|s| s.overlay_rev).unwrap_or(0)
}

/// 覆盖层 JSON：`{"box":[x0,y0,x1,y1]|null,"cells":[...],"sel":[...]}`。
///
/// 坐标均为画布内容逻辑坐标（Flutter 侧直接绘制）。
pub fn overlay_json() -> String {
    let Ok(s) = scene().lock() else {
        return "{}".into();
    };
    let mut out = String::from("{\"box\":");
    match s.overlay_box {
        Some(b) => out.push_str(&format!("[{},{},{},{}]", b[0], b[1], b[2], b[3])),
        None => out.push_str("null"),
    }
    out.push_str(",\"cells\":[");
    for (i, c) in s.overlay_cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("[{},{},{},{}]", c[0], c[1], c[2], c[3]));
    }
    out.push_str("],\"sel\":[");
    let mut first = true;
    // 锚点拖动中的第一条矩形用视觉缓动位置；其余矩形仍按文档目标绘制。
    let animated = s.sel_target.map(|t| {
        [
            t[0] + s.sel_anim[0],
            t[1] + s.sel_anim[1],
            t[2] + s.sel_anim[2],
            t[3] + s.sel_anim[3],
        ]
    });
    if let Some(doc) = s.document.as_ref().filter(|_| !s.riff_preview) {
        match s.view_mode {
            ViewMode::Pianoroll => {
                let view = &s.pr_view;
                for (t0, t1, klo, khi) in doc.edit.sel_rect.effective_rects() {
                    if !first {
                        out.push(',');
                    }
                    let (x0, y0, x1, y1) = if first {
                        if let Some(a) = animated {
                            (a[0], a[1], a[2], a[3])
                        } else {
                            (
                                view.tick_to_x(t0),
                                view.key_to_y(khi),
                                view.tick_to_x(t1),
                                view.key_to_y(klo) + view.key_height,
                            )
                        }
                    } else {
                        (
                            view.tick_to_x(t0),
                            view.key_to_y(khi),
                            view.tick_to_x(t1),
                            view.key_to_y(klo) + view.key_height,
                        )
                    };
                    first = false;
                    out.push_str(&format!("[{x0},{y0},{x1},{y1}]"));
                }
            }
            ViewMode::Arrangement => {
                let view = &s.arr_view;
                let lane_h = view.lane_height();
                for (t0, t1, tr_lo, tr_hi) in &doc.edit.arr_sel_rect {
                    if !first {
                        out.push(',');
                    }
                    let (x0, y0, x1, y1) = if first {
                        if let Some(a) = animated {
                            (a[0], a[1], a[2], a[3])
                        } else {
                            (
                                view.base.tick_to_x(*t0),
                                *tr_lo as f32 * lane_h - view.base.scroll_y,
                                view.base.tick_to_x(*t1),
                                (*tr_hi as f32 + 1.0) * lane_h - view.base.scroll_y,
                            )
                        }
                    } else {
                        (
                            view.base.tick_to_x(*t0),
                            *tr_lo as f32 * lane_h - view.base.scroll_y,
                            view.base.tick_to_x(*t1),
                            (*tr_hi as f32 + 1.0) * lane_h - view.base.scroll_y,
                        )
                    };
                    first = false;
                    out.push_str(&format!("[{x0},{y0},{x1},{y1}]"));
                }
            }
        }
    }
    out.push_str("]}");
    out
}

/// 播放光标在当前视图下的 x（逻辑像素；-1 = 无文档）。
///
/// 返回**最近一帧渲染时**算出的位置，而不重新读一次播放时钟：同一帧里
/// 视口跟随已经按某个 tick 滚动，若这里再取一个更晚的 tick，播放头会
/// 比画面领先一小段，渲染耗时抖动时表现为单帧左右抽动。
pub fn playhead_x() -> f32 {
    let Ok(s) = scene().lock() else {
        return -1.0;
    };
    s.last_playhead_x
}

/// 设置播放光标（点击跳转）：吸附到量化网格并 seek 音频。
pub fn set_play_cursor(tick: f64) {
    let snapped = {
        let Ok(s) = scene().lock() else {
            return;
        };
        let Some(doc) = s.document.as_ref() else {
            return;
        };
        let q = match s.view_mode {
            ViewMode::Pianoroll => doc.edit.quantize_pianoroll,
            ViewMode::Arrangement => doc.edit.quantize_arrange,
        };
        let ppu = match s.view_mode {
            ViewMode::Pianoroll => s.pr_view.base.pixels_per_tick,
            ViewMode::Arrangement => s.arr_view.base.pixels_per_tick,
        };
        let (num, den_pow) = doc
            .data
            .model
            .tempo_map
            .time_sig_at_tick(tick.max(0.0) as u32);
        let q = q.resolved_with_time_sig(
            ppu,
            s.ppq,
            num,
            den_pow,
            matches!(s.view_mode, ViewMode::Pianoroll),
        );
        SnapCtx::new(&s)
            .round(q, tick.max(0.0))
            .clamp(0.0, (s.total_ticks - 1.0).max(0.0))
    };
    if let Ok(mut s) = scene().lock() {
        s.cursor_tick = snapped;
    }
    crate::audio::seek(snapped);
}

/// AR 视口定位到某个 tick / 第一条选中轨（导入 MIDI 后调用）。
pub fn focus_arr_tick(tick: f64) {
    let (lw, lh) = crate::logical_size();
    let Ok(mut s) = scene().lock() else {
        return;
    };
    let selected_track = s
        .document
        .as_ref()
        .and_then(|d| d.edit.arr_sel_rect.first().map(|r| r.2))
        .unwrap_or(0);
    let total_ticks = s.total_ticks;
    let num_tracks = s.num_tracks;
    let view = &mut s.arr_view;
    let content_w = (lw - view.base.left_panel_width).max(1.0);
    let ppt = view.base.pixels_per_tick.max(0.001);
    let max_scroll = (total_ticks as f32 * ppt - content_w).max(0.0);
    view.base.scroll_x = (tick as f32 * ppt - content_w * 0.25)
        .max(0.0)
        .min(max_scroll);
    let row_h = view.lane_height();
    let max_y = (num_tracks as f32 * row_h - lh).max(0.0);
    view.base.scroll_y = (selected_track as f32 * row_h - lh * 0.25)
        .max(0.0)
        .min(max_y);
    view.base.track_panel_scroll_y = view.base.scroll_y;
    view.base.follow_target = None;
    view.base.dirty = true;
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
}

/// 事件列表跳转：光标精确落在 tick（不吸附），并把 PR 视口水平居中。
///
/// 与标尺点击的 `set_play_cursor` 不同：事件跳转必须落在事件本身的 tick，
/// 不能被量化档位吸走。
pub fn jump_to_event(tick: f64) {
    let (lw, lh) = crate::logical_size();
    let mut seek_to = tick.max(0.0);
    if let Ok(mut s) = scene().lock() {
        let max_tick = (s.total_ticks - 1.0).max(0.0);
        seek_to = seek_to.min(max_tick);
        s.cursor_tick = seek_to;
        let total_ticks = s.total_ticks;
        let view = &mut s.pr_view;
        let main_size = view.main_axis_len(lw, lh);
        let ppt = view.base.pixels_per_tick.max(0.001);
        *view.main_scroll() = seek_to as f32 * ppt - main_size * 0.5;
        view.clamp_main_scroll(main_size, total_ticks);
        view.base.follow_target = None;
        view.base.dirty = true;
        s.overlay_rev = s.overlay_rev.wrapping_add(1);
    }
    crate::audio::seek(seek_to);
}

/// 停止播放时把光标归零（与走带行为一致）。
pub fn reset_cursor() {
    if let Ok(mut s) = scene().lock() {
        s.cursor_tick = 0.0;
    }
}

/// 原样写入非播放态光标（暂停时把音频侧暂停点同步给绘制端）。
///
/// 不吸附、不触发 seek：暂停点就是用户听到的位置。
pub fn set_cursor_exact(tick: f64) {
    if !tick.is_finite() {
        return;
    }
    if let Ok(mut s) = scene().lock() {
        s.cursor_tick = tick.max(0.0);
    }
}

// ── 最近文件（主页列表，持久化到私有目录 recent.txt）──

/// 轨道面板 JSON：`[{"index","name","port","channel","color":[r,g,b,a],"muted","soloed"}]`。
pub fn tracks_json() -> String {
    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let mut out = String::from("[");
    for (i, t) in s.tracks.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"index\":{},\"uuid\":\"{}\",\"name\":\"{}\",\"port\":{},\"channel\":{},\"color\":[{},{},{},{}],\"muted\":{},\"soloed\":{},\"ib\":{},\"ip\":{},\"master\":{}}}",
            i,
            json_escape(&t.uuid),
            json_escape(&t.name),
            t.port,
            t.channel,
            t.color[0],
            t.color[1],
            t.color[2],
            t.color[3],
            t.muted,
            t.soloed,
            t.instrument_bank,
            t.instrument_program,
            t.master
        ));
    }
    out.push(']');
    out
}

/// 光标（或指定 tick）的 `bar.beat.tick` 文本，拍号变化时按段连续计数。
pub fn bar_beat_text(tick: f64) -> String {
    let Ok(s) = scene().lock() else {
        return String::new();
    };
    let Some(doc) = s.document.as_ref() else {
        return String::new();
    };
    bar_beat_text_for_model(&doc.data.model, tick.max(0.0))
}

/// 按模型拍号算 `bar.beat.tick`（不锁 scene，事件列表批量用）。
pub(super) fn bar_beat_text_for_model(model: &YinModel, tick: f64) -> String {
    let (num, den) = model.tempo_map.time_sig_at_tick(tick as u32);
    format_tick_bar_beat_with_time_sig(
        tick,
        model.meta.ppq,
        &model.tempo_map.time_sig_events,
        num,
        den,
    )
}

/// tick 所在拍号段的小节长 / 拍长（tick）：`(measure_ticks, beat_ticks)`。
pub fn bar_beat_ticks(at_tick: f64) -> (u32, u32) {
    let Ok(s) = scene().lock() else {
        return (0, 0);
    };
    let Some(doc) = s.document.as_ref() else {
        return (0, 0);
    };
    let model = &doc.data.model;
    let (num, den) = model.tempo_map.time_sig_at_tick(at_tick.max(0.0) as u32);
    let measure = measure_ticks(model.meta.ppq, num, den);
    (measure, (measure / u32::from(num.max(1))).max(1))
}

/// 时长文本（从 0 开始，不是绝对位置）：`小节.拍.tick`。
pub fn bar_beat_duration_text(ticks: f64, at_tick: f64) -> String {
    let (measure, beat) = bar_beat_ticks(at_tick);
    if ticks <= 0.0 || measure == 0 || beat == 0 {
        return "0.0.000".into();
    }
    let t = ticks.floor() as u64;
    let bars = t / u64::from(measure);
    let rem = t % u64::from(measure);
    let beats = rem / u64::from(beat);
    let in_beat = rem % u64::from(beat);
    format!("{bars}.{beats}.{in_beat:03}")
}

/// 解析时长输入：`4`=4 小节，`4.2`=4 小节 2 拍，`4.2.120`=再 120 tick。
///
/// 无效或 <= 0 返回 -1（调用方据此提示错误）。
pub fn parse_bar_beat_duration(text: &str, at_tick: f64) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return -1.0;
    }
    let (measure, beat) = bar_beat_ticks(at_tick);
    if measure == 0 || beat == 0 {
        return -1.0;
    }
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() == 1 {
        let Ok(bars) = text.parse::<f64>() else {
            return -1.0;
        };
        if !bars.is_finite() || bars <= 0.0 {
            return -1.0;
        }
        return (bars * f64::from(measure)).round();
    }
    if parts.len() > 3 {
        return -1.0;
    }
    let bars: i64 = parts[0].parse().unwrap_or(-1);
    let beats: i64 = parts[1].parse().unwrap_or(-1);
    let ticks: i64 = if parts.len() == 3 {
        parts[2].parse().unwrap_or(-1)
    } else {
        0
    };
    if bars < 0 || beats < 0 || ticks < 0 {
        return -1.0;
    }
    (bars * i64::from(measure) + beats * i64::from(beat) + ticks) as f64
}

/// 时间标尺标签：Dart 只负责画，拍号分段/小节号/密度选择全部在这里算。
///
/// 返回 JSON：`[{"x":..,"text":"..","level":0..3}]`，`x` 是画布逻辑像素。
/// level：0=小节、1=拍、2=拍内 tick、3=子拍。
pub fn ruler_labels_json() -> String {
    const MIN_LABEL_SPACING: f32 = 38.0;
    const SUB_BEAT_DIV: u32 = 4;

    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let (default_num, default_den) = model.tempo_map.time_sig_default;
    let segments =
        build_time_sig_segments(&model.tempo_map.time_sig_events, default_num, default_den);
    let bar_offsets = compute_bar_offsets(ppq, &segments);

    let (ppu, scroll_x, kb_w) = match s.view_mode {
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
    let width = crate::logical_size().0;
    if ppu <= 0.0 || width <= 0.0 {
        return "[]".into();
    }

    let start_tick = (((-kb_w + scroll_x) / ppu).max(0.0)) as f64;
    let end_tick = (((width - kb_w + scroll_x) / ppu).max(0.0)) as f64;
    let end_i = end_tick.floor() as i64;
    let mut labels: Vec<serde_json::Value> = Vec::new();

    for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
        if seg_start as f64 > end_tick {
            break;
        }
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |s| s.0);
        let mticks = measure_ticks(ppq, num, den);
        let bticks = (mticks / u32::from(num.max(1))).max(1);
        let sub_ticks = (bticks / SUB_BEAT_DIV).clamp(1, mticks);
        let divisor = compute_measure_divisor(mticks as f32 * ppu, MIN_LABEL_SPACING);
        let merged = mticks * divisor;
        let show_beat = bticks as f32 * ppu >= MIN_LABEL_SPACING;
        let show_sub = sub_ticks as f32 * ppu >= MIN_LABEL_SPACING;
        let show_tick = ppu >= MIN_LABEL_SPACING;
        let step = if show_beat || show_sub {
            sub_ticks
        } else {
            merged
        } as i64;

        let first = if seg_start as f64 > start_tick {
            seg_start as i64
        } else {
            start_tick.floor() as i64
        };
        let mut tick = seg_start as i64 + ((first - seg_start as i64) / step) * step;
        while tick <= end_i && tick < seg_end as i64 {
            let x = tick as f32 * ppu - scroll_x + kb_w;
            if x >= kb_w && x <= width + 40.0 {
                let local = (tick - seg_start as i64) as u32;
                let is_measure = local.is_multiple_of(merged);
                let is_beat = !is_measure && (local % mticks).is_multiple_of(bticks);
                let bar = bar_offsets[i] + local / mticks + 1;
                let beat = (local % mticks) / bticks + 1;
                let (text, level) = if is_measure {
                    (bar.to_string(), 0)
                } else if is_beat && show_beat {
                    (format!("{bar}.{beat}"), 1)
                } else if show_sub {
                    if show_tick {
                        (format!("{bar}.{beat}.{:03}", local % bticks), 2)
                    } else {
                        (format!("{bar}.{beat}.{}", (local % bticks) / sub_ticks), 3)
                    }
                } else {
                    (String::new(), 4)
                };
                if !text.is_empty() {
                    labels.push(serde_json::json!({"x": x, "text": text, "level": level}));
                }
            }
            tick += step;
        }
    }
    serde_json::Value::Array(labels).to_string()
}

/// 时间网格线：Dart 只负责画，拍号分段/密度/层级全部在这里算。
///
/// 返回 JSON：`[{"tick":..,"level":0..3}]`。level：0=小节、1=拍、2=细分、3=tick。
/// x 坐标由 Dart 用各自视口的 `ppu/scroll/leftInset` 换算（PR/AR/AM 同一条数据）。
pub fn time_grid_json() -> String {
    const MIN_SPACING: f32 = 38.0;

    let Ok(s) = scene().lock() else {
        return "[]".into();
    };
    let Some(doc) = s.document.as_ref() else {
        return "[]".into();
    };
    let model = &doc.data.model;
    let ppq = model.meta.ppq;
    let (default_num, default_den) = model.tempo_map.time_sig_default;
    let segments =
        build_time_sig_segments(&model.tempo_map.time_sig_events, default_num, default_den);

    let (ppu, scroll_x, left_inset) = match s.view_mode {
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
    let width = crate::logical_size().0;
    if ppu <= 0.0 || width <= 0.0 || ppq == 0 {
        return "[]".into();
    }

    let start_tick = (((scroll_x - left_inset) / ppu).max(0.0)) as f64;
    let end_tick = (((scroll_x - left_inset + width) / ppu).max(0.0)) as f64;
    let end_i = end_tick.floor() as i64;
    let sub_ticks = (ppq / 4).max(1);
    // AR 只画小节/拍线；PR/AM 才有细分和 tick 层。Auto 量化用同一开关。
    let allow_sub = matches!(s.view_mode, ViewMode::Pianoroll);
    let mut lines: Vec<serde_json::Value> = Vec::new();

    for (i, &(seg_start, num, den)) in segments.iter().enumerate() {
        if seg_start as f64 > end_tick {
            break;
        }
        let seg_end = segments.get(i + 1).map_or(u32::MAX, |s| s.0);
        let mticks = measure_ticks(ppq, num, den);
        let bticks = (mticks / u32::from(num.max(1))).max(1);
        let divisor = compute_measure_divisor(mticks as f32 * ppu, MIN_SPACING);
        let merged = divisor > 1;
        let show_beat = !merged;
        let show_sub = allow_sub && show_beat && bticks as f32 * ppu >= MIN_SPACING;
        let show_tick = allow_sub && show_sub && ppu >= 10.0;
        // 与 Auto 量化共用的唯一来源：当前实际画出的最细网格步长。
        let step = yinhe_editor_core::quantize::visible_grid_step(
            ppu, ppq, num, den, allow_sub, allow_sub,
        );

        let first = if seg_start as f64 > start_tick {
            seg_start as i64
        } else {
            start_tick.floor() as i64
        };
        let mut tick = seg_start as i64 + ((first - seg_start as i64) / step as i64) * step as i64;
        while tick <= end_i && tick < seg_end as i64 {
            let local = (tick - seg_start as i64) as u32;
            let is_measure = local.is_multiple_of(mticks);
            let beat_local = local % mticks;
            let is_beat = beat_local.is_multiple_of(bticks) && beat_local > 0;
            let is_sub = local.is_multiple_of(sub_ticks);
            let level = if is_measure {
                Some(0)
            } else if show_beat && is_beat {
                Some(1)
            } else if show_sub && is_sub {
                Some(2)
            } else if show_tick || show_sub {
                Some(3)
            } else {
                None
            };
            if let Some(level) = level {
                lines.push(serde_json::json!({"tick": tick, "level": level}));
            }
            tick += step as i64;
        }
    }
    serde_json::Value::Array(lines).to_string()
}

/// 视图快照（Dart 文本浮层用），固定 13 个 f32：
/// `[mode, ppu, scroll_x, scroll_y, key_height, left_panel_w, lane_height,
///   total_ticks, ppq, key0_y, time_sig_num, time_sig_den]`
pub fn view_snapshot() -> [f32; 15] {
    let Ok(mut s) = scene().lock() else {
        return [0.0; 15];
    };
    crate::selection_materializer::apply_pending(&mut s);
    let ppq = s.ppq as f32;
    // 顶栏 BPM/拍号 = 播放光标处的实时值（原生事件优先，否则工程默认），
    // 与桌面时间码 `bpm_at_time` / `time_sig_at_tick` 同规则。
    let (sig_num, sig_den, bpm) = match s.document.as_ref() {
        Some(doc) => {
            let model = &doc.data.model;
            let tick = crate::audio::playback_tick_for(model).max(0.0);
            let (num, den) = model.tempo_map.time_sig_at_tick(tick as u32);
            (num, den, model.tempo_map.bpm_at_tick(tick as u64))
        }
        None => (4, 2, 120.0),
    };
    match s.view_mode {
        ViewMode::Pianoroll => [
            1.0,
            s.pr_view.base.pixels_per_tick,
            s.pr_view.base.scroll_x,
            s.pr_view.base.scroll_y,
            s.pr_view.key_height,
            s.pr_view.keyboard_width(),
            0.0,
            s.total_ticks as f32,
            ppq,
            s.pr_view.key_to_y(0),
            f32::from(sig_num),
            f32::from(sig_den),
            bpm,
            f32::from(s.pr_view.layout.start),
            f32::from(s.pr_view.layout.count),
        ],
        ViewMode::Arrangement => [
            0.0,
            s.arr_view.base.pixels_per_tick,
            s.arr_view.base.scroll_x,
            s.arr_view.base.scroll_y,
            0.0,
            s.arr_view.base.left_panel_width,
            s.arr_view.lane_height(),
            s.total_ticks as f32,
            ppq,
            0.0,
            f32::from(sig_num),
            f32::from(sig_den),
            bpm,
            0.0,
            0.0,
        ],
    }
}

/// 设置 PR 音域键数（88/128/256）。成功 1。
pub fn set_pr_key_count(count: u16) -> i32 {
    if !matches!(count, 88 | 128 | 256) {
        return 0;
    }
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.pr_view.set_key_count(count);
    refresh_snapshot(&mut s);
    1
}

/// 滚轮/触控板：`ctrl=true` 缩放（键盘/面板区 → 副轴，其余 → 主轴），否则平移。
pub fn scroll_at(x: f32, y: f32, dx: f32, dy: f32, ctrl: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let s = &mut *s;
    let (lw, lh) = crate::logical_size();
    let total = s.total_ticks;
    let num_tracks = s.num_tracks;
    match s.view_mode {
        ViewMode::Pianoroll => {
            let view = &mut s.pr_view;
            if ctrl {
                let factor = (-dy * 0.0025).exp();
                if x < view.keyboard_width() {
                    let cross = view.cross_axis_len(lw, lh);
                    view.zoom_cross_around(y, factor, cross);
                } else {
                    view.zoom_main_around(x - view.keyboard_width(), factor);
                }
            } else {
                *view.main_scroll() -= dx + dy;
            }
            s.pr_view.clamp_scroll(lw, lh, total);
        }
        ViewMode::Arrangement => {
            let view = &mut s.arr_view;
            if ctrl {
                let factor = (-dy * 0.0025).exp();
                if x < view.base.left_panel_width {
                    view.zoom_lane_height(y, factor);
                } else {
                    view.base.zoom_around_x(x, factor);
                }
            } else {
                view.base.scroll_x -= dx + dy;
            }
            s.arr_view.clamp_scroll(lw, lh, total, num_tracks + 1);
        }
    }
    // 视图变化后选框需按内容坐标重算屏幕位置（与桌面逐帧换算一致）。
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    1
}

/// Flutter 视口手势模式。
pub const GESTURE_PAN: i32 = 0;

pub const GESTURE_PR_TIME: i32 = 1;

pub const GESTURE_PR_PITCH: i32 = 2;

pub const GESTURE_AR_TIME: i32 = 3;

pub const GESTURE_AR_LANE: i32 = 4;

/// 播放跟随（当前只做居中模式）。
///
/// 不做“边缘轮询再跳”：目标位置来自播放头，随播放匀速前进，视口用帧率无关的
/// 指数插值跟随，视觉上是连续匀速移动；用户手动平移/缩放后暂停，播放头连续
/// 离开视口 `FOLLOW_RESUME_SECS` 秒后自动恢复。
pub(crate) fn apply_follow(s: &mut RenderScene, w: f32, h: f32, tick: f64) {
    let _ = h;
    let Some(_doc) = s.document.as_ref() else {
        return;
    };
    let (ppu, left, content_w, scroll) = match s.view_mode {
        ViewMode::Pianoroll => {
            let v = &s.pr_view;
            (
                v.base.pixels_per_tick,
                v.keyboard_width(),
                (w - v.keyboard_width()).max(1.0),
                v.base.scroll_x,
            )
        }
        ViewMode::Arrangement => {
            let v = &s.arr_view;
            (
                v.base.pixels_per_tick,
                v.base.left_panel_width,
                (w - v.base.left_panel_width).max(1.0),
                v.base.scroll_x,
            )
        }
    };
    if ppu <= 0.0 || w < 1.0 || content_w < 1.0 {
        return;
    }
    let now = std::time::Instant::now();
    let dt = s
        .follow_last_at
        .map(|t| (now - t).as_secs_f32())
        .unwrap_or(0.2)
        .clamp(0.0, 0.1);
    s.follow_last_at = Some(now);

    let cursor_x = tick as f32 * ppu - scroll + left;
    let out = cursor_x < left - 1.0 || cursor_x > w + 1.0;
    if s.follow_paused {
        if !out {
            s.follow_out_since = None;
            return;
        }
        let since = *s.follow_out_since.get_or_insert(now);
        if now.duration_since(since).as_secs_f32() < FOLLOW_RESUME_SECS {
            return;
        }
        s.follow_paused = false;
        s.follow_out_since = None;
        s.follow_anim_until = Some(now + FOLLOW_ANIM);
    }
    let target = (tick as f32 * ppu - content_w * 0.5).max(0.0);
    // 起点/大幅跳转（例如切到另一个视图）先做固定 0.3s 平滑吸附；
    // 稳定跟随时每帧精确居中，避免“平滑系数 + 匀速目标”的速度相关滞后。
    if s.follow_anim_until.is_none() && (scroll - target).abs() > content_w * 0.5 {
        s.follow_anim_until = Some(now + FOLLOW_ANIM);
    }
    let catching = s.follow_anim_until.is_some_and(|until| now < until);
    let next = if catching {
        yinhe_editor_core::follow::follow_interpolate(scroll, target, dt, FOLLOW_SMOOTH_TAU)
    } else {
        s.follow_anim_until = None;
        target
    };
    match s.view_mode {
        ViewMode::Pianoroll => s.pr_view.base.scroll_x = next,
        ViewMode::Arrangement => s.arr_view.base.scroll_x = next,
    }
}

/// 恢复跟随的判定时长：播放头连续离开视口这么久才重新居中。
const FOLLOW_RESUME_SECS: f32 = 2.0;

/// 跟随平滑时间常数（秒）；越小越贴近播放头，越大越柔和。
const FOLLOW_SMOOTH_TAU: f32 = 0.05;

/// 恢复/起手时的固定吸附时长。
const FOLLOW_ANIM: std::time::Duration = std::time::Duration::from_millis(300);

/// 应用一次 Flutter 视口手势：先平移，再围绕当前焦点缩放，最后统一 clamp。
///
/// `pan_x/pan_y` 为逻辑像素；`scale` 为本次增量倍率（1=不缩放）。
/// PR/AR 的轴向由 Flutter 在起手时锁定，这里不再按当前中点切换。
pub fn viewport_gesture(
    mode: i32,
    pan_x: f32,
    pan_y: f32,
    scale: f32,
    focus_x: f32,
    focus_y: f32,
) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let (lw, lh) = crate::logical_size();
    let total = s.total_ticks;
    let num_tracks = s.num_tracks;
    let do_pan = (pan_x != 0.0 || pan_y != 0.0) && pan_x.is_finite() && pan_y.is_finite();
    let do_zoom = scale.is_finite()
        && scale > 0.0
        && (scale - 1.0).abs() > 1e-6
        && focus_x.is_finite()
        && focus_y.is_finite();
    match s.view_mode {
        ViewMode::Pianoroll => {
            let mut key_accum = s.pr_key_zoom_accum;
            let view = &mut s.pr_view;
            if do_pan {
                *view.main_scroll() -= pan_x;
                *view.cross_scroll() -= pan_y;
            }
            if do_zoom {
                match mode {
                    GESTURE_PR_TIME => {
                        let kb_w = view.keyboard_width();
                        view.zoom_main_around(focus_x - kb_w, scale);
                    }
                    GESTURE_PR_PITCH => {
                        let cross = view.cross_axis_len(lw, lh);
                        view.zoom_cross_discrete(focus_y, scale, cross, &mut key_accum);
                    }
                    _ => {}
                }
            }
            view.clamp_scroll(lw, lh, total);
            s.pr_key_zoom_accum = key_accum;
        }
        ViewMode::Arrangement => {
            let view = &mut s.arr_view;
            if do_pan {
                view.base.scroll_x -= pan_x;
                view.base.scroll_y -= pan_y;
            }
            if do_zoom {
                match mode {
                    GESTURE_AR_TIME => view.base.zoom_around_x(focus_x, scale),
                    GESTURE_AR_LANE => view.zoom_lane_height(focus_y, scale),
                    _ => {}
                }
            }
            view.clamp_scroll(lw, lh, total, num_tracks + 1);
        }
    }
    s.overlay_rev = s.overlay_rev.wrapping_add(1);
    1
}

/// 按逻辑像素平移视口（鼠标右键拖拽等）。
pub fn pan_by(dx: f32, dy: f32) -> i32 {
    viewport_gesture(GESTURE_PAN, dx, dy, 1.0, 0.0, 0.0)
}

/// 设置 AR 左侧轨道面板宽度（折叠/展开两档）。成功 1。
pub fn set_arr_panel_width(width: f32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let w = width.clamp(60.0, 400.0);
    if (s.arr_view.base.left_panel_width - w).abs() < 0.5 {
        return 1;
    }
    s.arr_view.base.left_panel_width = w;
    1
}

/// 设置“显示轨道”过滤（-1 = 全部）。PR/AR 音符只画该轨。成功 1。
pub fn set_display_track(track: i32) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    let next = if track < 0 {
        None
    } else {
        Some(track as usize)
    };
    // 切换显示轨后旧选区可能落在看不见的轨上（移动/删除会改到隐藏音符）；
    // 这里直接清空，让用户在新视图里重新框选。
    if s.display_track != next {
        s.display_track = next;
        if let Some(doc) = s.document.as_mut() {
            doc.edit.selected.clear();
            doc.edit.sel_rect.clear();
            doc.edit.arr_sel_rect.clear();
            doc.edit.chan_ar_sel.clear();
        }
        crate::selection_materializer::invalidate();
        s.overlay_rev = s.overlay_rev.wrapping_add(1);
    }
    log::info!("显示轨道: {track}");
    1
}

/// 设置 PR 其他轨道是否去色（0/1）。仅影响钢琴卷帘，不影响 AR。
pub fn set_gray_other_tracks(on: bool) -> i32 {
    let Ok(mut s) = scene().lock() else {
        return 0;
    };
    s.gray_other_tracks = on;
    log::info!("其他轨道去色: {on}");
    1
}

/// 歌曲标题（meta.name；空串 = 未设置）。
pub fn song_title() -> String {
    let Ok(s) = scene().lock() else {
        return String::new();
    };
    let Some(doc) = s.document.as_ref() else {
        return String::new();
    };
    doc.data.model.meta.name.clone()
}

/// tick → 秒（时间码预显示用；tempo 事件优先）。
pub fn seconds_at_tick(tick: f64) -> f64 {
    scene()
        .lock()
        .ok()
        .and_then(|s| s.model.clone())
        .map(|m| m.tempo_map.tick_to_seconds(tick.max(0.0) as u64))
        .unwrap_or(0.0)
}
