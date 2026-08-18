use std::collections::HashSet;

use eframe::egui;

use yinhe_types::{ArRow, ArRowLayout, ArrangementView};
use yinhe_wgpu::layer_cache_key;
use yinhe_wgpu::{InstanceRenderer, MAX_TRACKS, Uniforms};
use yinhe_wgpu::{build_arr_notes, build_ghost_notes};

use crate::piano_view::drag::{GhostNote, HiddenNote};
use crate::render_context::RenderContext;
use crate::widgets::tools_panel::Tool;

/// Display the arrangement view texture with zoom/pan interaction.
///
/// Uses the layered cache API: decor (layer 0), grid (layer 1), notes (layer 2),
/// ghost notes (layer 3, no cache).  The playhead cursor is drawn by egui on top
/// of the wgpu texture.
///
/// `arr_drag_delta` is set on mouse release after dragging an existing selection
/// (moving notes + automation events in the selected track/time range).
/// `(delta_ticks, delta_tracks)` — ticks are horizontal, tracks are vertical.
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    renderer: &mut InstanceRenderer,
    render_ctx: &mut RenderContext,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: super::ArrangeData<'_>,
    edit: &mut super::ArrangeEdit<'_>,
    cfg: &mut super::ArrangeViewCfg<'_>,
) {
    let _arrange_total_start = if yinhe_memtrace::perf_probe::enabled() {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // 纹理为全宽（含轨道面板列），left_panel_width = 面板宽 + 分屏条宽；
    // 所有绘制 clip 到音乐区，轨道面板/分屏条仍由 arrange.rs 的 egui 层绘制。
    // 必须在 allocate_painter 之前设置：painter 的 clip_rect 在分配时快照。
    let full_rect = ui.max_rect();
    let lp = view.base.left_panel_width;
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(full_rect.min.x + lp, full_rect.min.y),
        full_rect.max,
    );
    ui.set_clip_rect(ui.clip_rect().intersect(music_rect));

    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;
    let ppp = ui.ctx().pixels_per_point();
    let w = rect.width() as u32;
    let h = rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return;
    }

    render_ctx.ensure_size(pw, ph);

    view.clamp_scroll(
        w as f32,
        h as f32,
        data.total_ticks,
        row_layout.total_rows(),
    );

    if let Some(ct) = *edit.cursor_tick
        && cfg.is_playing
        && *cfg.follow_mode != crate::view_interaction::FollowMode::None
        && let Some(new_scroll_x) = crate::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            w as f32,
            0.0,
            *cfg.follow_mode,
            0.01,
            view.base.scroll_x,
        )
    {
        view.base.scroll_x = new_scroll_x;
        view.clamp_scroll(
            w as f32,
            h as f32,
            data.total_ticks,
            row_layout.total_rows(),
        );
    }

    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = match cfg.scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        }
    };

    // Build track colors — dynamic Vec, no fixed 1MB allocation.
    let track_count = data.track_colors.len().min(MAX_TRACKS) as u32;
    let tc_colors: Vec<[f32; 4]> = data.track_colors.iter().take(MAX_TRACKS).copied().collect();

    let uniforms = Uniforms {
        width: w as f32,
        height: h as f32,
        scroll_x: scroll_x_pos,
        scroll_y: view.base.scroll_y,
        pixels_per_tick: view.base.pixels_per_tick,
        key_height: 0.0, // AR unused (shader uses lane_height)
        keyboard_width: view.base.left_panel_width,
        mode: 2, // AR notes: shader computes pixel_y from lane_height + scroll_y
        scroll_frac,
        scroll_mode: cfg.scroll_mode,
        min_border_width: cfg.min_border_width,
        track_count,       // AR notes now use data.track_colors uniform for coloring
        sel_rect_count: 0, // unused in AR mode
        note_outline: 1,   // AR mode: outline always on
        lane_height: view.lane_height(), // AR: per-track lane height
        value_zoom: 0.0,   // AR unused (automation panel only)
        value_scroll: 0.0, // AR unused (automation panel only)
    };

    view.base.dirty = false;

    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&tc_colors);
    // Grid 已迁移到 egui（widgets::grid_lines），wgpu 只剩 notes + ghost notes 两层。
    renderer.ensure_layers(2);

    // ── Select tool dispatch (BEFORE layer building to get ghost notes) ──
    // Like PR's sel_drag_frame, this returns ghost_notes/hidden_notes generated
    // from the CURRENT frame's mouse position, enabling zero-delay ghost preview.
    let (mut ghost_notes, hidden_notes, drag_rect) =
        if *cfg.active_tool == Tool::Select || *cfg.active_tool == Tool::SelectVertical {
            let vertical = *cfg.active_tool == Tool::SelectVertical;
            sel_drag_frame_arrange(
                ui, rect, music_rect, view, row_layout, &data, edit, vertical,
            )
        } else {
            (Vec::new(), HashSet::new(), None)
        };

    let vh = view.render_hash();
    let wh = {
        let mut hash: u64 = 0;
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(w as u64);
        hash = hash.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(h as u64);
        hash
    };

    // tv_hash still needed for notes layer cache key
    let tv_hash = {
        let mut h = 0u64;
        for &v in data.track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };

    // Grid lines 已迁移到 egui（见下方 paint_grid_lines 调用），wgpu 不再构建 grid layer。

    // 行几何：行高 / 滚动 / 可见行范围（含展开的自动化 lane 子行）。
    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;
    let first_row = ((scroll_y / lh).floor().max(0.0) as usize).min(row_layout.total_rows());
    let last_row =
        (((scroll_y + h as f32) / lh).ceil().max(0.0) as usize).min(row_layout.total_rows());
    let track_range = row_layout.visible_track_range(scroll_y, h as f32, lh);
    // 每轨主行 y 偏移表：展开自动化 lane 后行布局不再均匀，shader 查表定位。
    renderer.upload_track_offsets(&row_layout.track_offsets(lh));
    // 布局 hash：折叠/展开会改变可见音轨范围与行位置，音符层与 AM 层都依赖它。
    let offsets_hash = row_layout.track_offsets(lh).iter().fold(0u64, |acc, &o| {
        acc.wrapping_mul(0x9e3779b97f4a7c15)
            .wrapping_add(o.to_bits() as u64)
    });

    // Layer 0: notes (16B NoteInstance — shader computes pixel positions from uniforms)
    // key 含 offsets_hash：布局变化（chevron 折叠/展开）时强制重建，
    // 否则可见音轨范围变了而 layer 0 缓存不失效，会残留旧行位置的音符。
    let notes_key = layer_cache_key(&[
        vh,
        wh,
        tv_hash,
        offsets_hash,
        cfg.revision,
        hidden_notes.len() as u64,
    ]);
    renderer.upload_note_layer(0, notes_key, |out| {
        if let Some(midi) = data.midi {
            build_arr_notes(
                out,
                w as f32,
                midi,
                view,
                track_range,
                data.track_visible,
                &hidden_notes,
            );
        }
    });

    // Layer 1: ghost notes (no cache — rebuilt every frame during drag)
    renderer.upload_note_layer(1, 0, |out| {
        build_ghost_notes(
            out,
            &mut ghost_notes,
            w as f32,
            view,
            track_range,
            data.track_visible,
        );
    });

    // ── 自动化 lane 交互（展开的音轨子行 + Conductor 主行 Tempo 直显）──
    // 先于曲线渲染层调用，本帧 ghost 当帧可见；edits 由 arrange.rs 应用到 Document。
    let am_rows =
        super::am_lanes::visible_am_rows(row_layout, first_row, last_row, data.conductor_track_idx);
    let mut am_ghost: Option<(yinhe_wgpu::AutomationGhost, f32, f32, f32)> = None;
    let mut am_marquee: Option<egui::Rect> = None;
    if !am_rows.is_empty() {
        let am_ctx = crate::piano_view::automation_panel::AutomationEditCtx {
            active_tool: *cfg.active_tool,
            active_track: None, // AR 无 editing_track；lane 交互自带 track
            quantize: data.quantize,
            ppq: data.ppq,
            bar_line_data: data.bar_line_data,
        };
        let mut io = super::am_lanes::AmLanesIo {
            tracks: data.tracks,
            tempo_lane: data.tempo_lane,
            track_colors: data.track_colors,
            selected: &mut *edit.selected,
            info_content: &mut *edit.info_content,
            right_tab: &mut *edit.right_tab,
            am_views: &mut *edit.arr_am_views,
            edits: &mut *edit.am_edits,
        };
        let out =
            super::am_lanes::interact_all(ui, &am_rows, view, rect, music_rect, &am_ctx, &mut io);
        am_ghost = out.ghost;
        am_marquee = out.marquee;
    }

    // ── 自动化曲线渲染层（layer 2 数据 + layer 3 ghost，画在共享走带纹理上）──
    {
        let show_anchors = matches!(
            *cfg.active_tool,
            Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
        );
        let mut am_render: Vec<yinhe_wgpu::ArrAutomationLane> = Vec::new();
        let mut am_highlights: Vec<Box<[u32]>> = Vec::new();
        for r in &am_rows {
            let (lane, track) = match r.sub {
                Some(sub) => {
                    match data
                        .tracks
                        .get(r.track)
                        .and_then(|t| t.automation_lanes.get(sub))
                    {
                        Some(l) => (l, r.track as u16),
                        None => continue,
                    }
                }
                None => (data.tempo_lane, r.track as u16),
            };
            let key = (track, lane.target.clone());
            let sel_rects = edit
                .arr_am_views
                .get(&key)
                .map(|v| v.anchor_sel_rects.as_slice())
                .unwrap_or(&[]);
            am_highlights.push(super::am_lanes::lane_highlight_ticks(
                lane,
                track,
                sel_rects,
                edit.info_content,
            ));
            am_render.push(yinhe_wgpu::ArrAutomationLane {
                lane,
                y_top: r.row as f32 * lh - scroll_y,
                height: lh,
                max_val: super::am_lanes::lane_max_val(lane),
                highlight_ticks: &[],
            });
        }
        // 二阶段回填：highlights 定稿后再借给 render lanes。
        for (i, l) in am_render.iter_mut().enumerate() {
            l.highlight_ticks = &am_highlights[i];
        }
        // 缓存 key：布局 hash（上面已算）+ revision（任何编辑都 bump）+ 高亮锚点。
        let hl_hash = am_highlights.iter().fold(0u64, |acc, hl| {
            hl.iter()
                .fold(acc, |a, &tk| a.wrapping_mul(31).wrapping_add(tk as u64))
        });
        let am_key = layer_cache_key(&[
            vh,
            wh,
            tv_hash,
            offsets_hash,
            show_anchors as u64,
            cfg.revision,
            hl_hash,
        ]);
        yinhe_wgpu::prepare_arr_automation(
            renderer,
            w as f32,
            h as f32,
            &view.base,
            &am_render,
            data.track_visible,
            data.track_colors,
            show_anchors,
            am_ghost,
            am_key,
        );
    }

    let content_changed = true;

    // ── Track lanes ──
    // 普通行 = app_bg（打底一层，不透明）；着色行（偶数行号）叠更黑条纹。
    // 按全局行号奇偶（AM 子行也参与，展开奇数条 lane 会错位后续音轨斑纹）。
    painter.rect_filled(rect, 0.0, crate::theme::app_bg());
    let lb_w = view.base.left_panel_width;
    for row in first_row..last_row {
        let Some(hit) = row_layout.row_hit(row) else {
            continue;
        };
        let track = hit.track();
        if !data.track_visible.get(track).copied().unwrap_or(true) {
            continue;
        }
        if row % 2 != 0 {
            continue; // 奇数行 = 普通行（app_bg）
        }
        let y = rect.min.y + row as f32 * lh - scroll_y;
        let col = crate::theme::stripe_bg();
        painter.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.min.x + lb_w, y),
                egui::vec2(w as f32 - lb_w, lh),
            ),
            0.0,
            col,
        );
    }

    // ── Grid lines (drawn by egui before wgpu texture) ──
    // 替代原 wgpu grid layer。与 time_ruler 共用 MIN_SPACING 阈值，保证"有线就有标签"。
    if let Some(midi) = data.midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let grid_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + view.base.left_panel_width, rect.min.y),
            rect.max,
        );
        crate::widgets::grid_lines::paint_grid_lines(
            &painter,
            grid_rect,
            &view.base,
            tpb,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::arrangement(),
        );
    }

    render_ctx.paint(
        renderer,
        pw,
        ph,
        "arrangement_frame",
        &painter,
        rect,
        content_changed,
    );

    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    if let Some(ct) = *edit.cursor_tick {
        let lb_w = view.base.left_panel_width;
        let cx_local = view.tick_to_x(ct);
        if cx_local >= lb_w && cx_local <= w as f32 {
            let cx = rect.min.x + cx_local;
            painter.line_segment(
                [egui::pos2(cx, rect.min.y), egui::pos2(cx, rect.max.y)],
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::contrast_fg()),
            );
        }
    }

    // ── Draw drag selection rect (move-drag offset or marquee) on top of GPU texture ──
    if let Some(dr) = drag_rect {
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            dr,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }

    // ── Eraser tool dispatch (after GPU texture, before eraser marquee drawing) ──
    if *cfg.active_tool == Tool::Eraser {
        eraser_drag_frame_arrange(ui, rect, music_rect, view, row_layout, &data, edit);
    }

    // Draw persisted selection rects (remains after mouse release, 支持多选框).
    // y 范围用行布局：覆盖展开的 AM 子行（选区内 automation 事件也随拖拽移动）。
    for &(t_start, t_end, track_lo, track_hi) in edit.arr_sel_rect.iter() {
        let view_sy = row_layout.track_y(track_lo, lh) - scroll_y;
        let view_ey =
            row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh) - scroll_y;
        let view_sx = view.tick_to_x(t_start);
        let view_ex = view.tick_to_x(t_end);
        let snapped = egui::Rect::from_min_max(
            egui::pos2(view_sx.min(view_ex), view_sy.min(view_ey)),
            egui::pos2(view_sx.max(view_ex), view_sy.max(view_ey)),
        );
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            snapped,
            crate::theme::contrast_fg(),
            crate::theme::contrast_fg(),
        );
    }

    // Draw eraser marquee box in red (active during drag)
    if *cfg.active_tool == Tool::Eraser {
        let drag_id = ui.id().with("eraser_drag_arr");
        let drag: Option<((f64, f32), egui::Pos2)> =
            ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);
        if let Some((start_music, end)) = drag {
            let start_pixel = egui::pos2(
                view.tick_to_x(start_music.0),
                start_music.1 * view.lane_height() - view.base.scroll_y,
            );
            if (end - start_pixel).length() >= 3.0
                && let Some(b) =
                    arrange_snapped_bounds(start_pixel, end, view, row_layout, &data, false)
            {
                let snapped = egui::Rect::from_min_max(
                    egui::pos2(b.view_sx.min(b.view_ex), b.view_sy.min(b.view_ey)),
                    egui::pos2(b.view_sx.max(b.view_ex), b.view_sy.max(b.view_ey)),
                );
                crate::selection::draw::draw(
                    ui.painter(),
                    rect,
                    snapped,
                    crate::theme::danger_text_bright(),
                    crate::theme::danger_text_bright(),
                );
            }
        }
    }

    // AM lane 的 Select/Eraser 框选矩形（draw 接受 content 局部坐标，需平移）
    if let Some(mr) = am_marquee {
        let col = if *cfg.active_tool == Tool::Eraser {
            crate::theme::danger_text_bright()
        } else {
            crate::theme::contrast_fg()
        };
        crate::selection::draw::draw(
            ui.painter(),
            rect,
            mr.translate(-rect.min.to_vec2()),
            col,
            col,
        );
    }

    crate::view_interaction::handle_input(
        ui,
        rect,
        view,
        edit.cursor_tick,
        0.0,
        Some((data.quantize, data.ppq)),
        data.bar_line_data,
        None,
        // 全宽 rect 含轨道面板列，交互命中限制在音乐区
        Some(music_rect),
        cfg.is_playing,
        cfg.follow_mode,
        cfg.active_tool,
    );

    // Clamp scroll after input
    view.clamp_scroll(
        w as f32,
        h as f32,
        data.total_ticks,
        row_layout.total_rows(),
    );

    if let Some(t0) = _arrange_total_start {
        yinhe_memtrace::perf_probe::record_arrange_total(t0.elapsed());
    }
}

// ── Arrangement selection drag ──

/// Returns `(ghost_notes, hidden_notes, drag_rect)` for move-drag preview.
///
/// `ghost_notes`: `(start_tick, end_tick, key, track)` — preview notes at new positions.
/// `hidden_notes`: `(track, start_tick, key)` — original notes to hide during drag.
/// `drag_rect`: the selection rect to draw on top of the GPU texture (move-drag offset
///   rect or marquee rect). `None` on release (arr_sel_rect takes over).
#[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
fn sel_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    hit_rect: egui::Rect,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: &super::ArrangeData<'_>,
    edit: &mut super::ArrangeEdit<'_>,
    vertical: bool,
) -> (Vec<GhostNote>, HashSet<HiddenNote>, Option<egui::Rect>) {
    let mut ghost_notes: Vec<GhostNote> = Vec::new();
    let mut hidden_notes: HashSet<HiddenNote> = HashSet::new();
    let mut drag_rect: Option<egui::Rect> = None;

    let sel_id = ui.id().with("sel_drag_arr");
    // 拖框起始点存音乐坐标 (start_tick, start_track_y)，免疫任何滚动
    // （触摸板滚动、自动滚动、中键拖拽都不会改变音乐坐标）
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(sel_id)).unwrap_or(None);

    // Move-drag state: ((origin_tick, origin_track_f), (current_tick, current_track_f), alt)
    // Stores both tick (horizontal) and track-float (vertical) music coordinates.
    // alt = 按住 Option 拖拽：复制而非移动（press 时锁定）。
    type ArrMoveDrag = ((f64, f32), (f64, f32), bool);
    let move_drag_id = ui.id().with("arr_move_drag");
    let mut move_drag: Option<ArrMoveDrag> = ui
        .data_mut(|d| d.get_persisted(move_drag_id))
        .unwrap_or(None);
    // 拖拽开始时保存原选框快照（多选框，所以是 Vec）
    let move_orig_id = ui.id().with("arr_move_orig_sel");
    let mut move_orig_sel: Vec<(f64, f64, usize, usize)> = ui
        .data_mut(|d| d.get_persisted(move_orig_id))
        .unwrap_or_default();

    let pointer = ui.input(|i| i.pointer.clone());
    let cmd = ui.input(|i| i.modifiers.command || i.modifiers.ctrl);
    // shift 或 cmd/ctrl 都表示累加模式（多选框）
    let additive = cmd || ui.input(|i| i.modifiers.shift);

    // Clear stale drag states (e.g. lost window focus mid-drag)
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }
    if move_drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        move_drag = None;
        move_orig_sel.clear();
    }

    // 弹窗打开时跳过所有 pointer 处理，避免点击穿透
    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(sel_id, drag));
        ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
        ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));
        return (ghost_notes, hidden_notes, drag_rect);
    }

    // ── Check if mouse is inside any existing selection rect (for Move cursor + drag) ──
    let inside_sel_rect = edit
        .arr_sel_rect
        .iter()
        .any(|&(t_start, t_end, track_lo, track_hi)| {
            pointer.hover_pos().is_some_and(|pos| {
                let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
                let lh = view.lane_height();
                let scroll_y = view.base.scroll_y;
                let sy = row_layout.track_y(track_lo, lh) - scroll_y;
                let ey = row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh)
                    - scroll_y;
                let sx = view.tick_to_x(t_start);
                let ex = view.tick_to_x(t_end);
                let rect = egui::Rect::from_min_max(
                    egui::pos2(sx.min(ex), sy.min(ey)),
                    egui::pos2(sx.max(ex), sy.max(ey)),
                );
                rect.contains(local)
            })
        });

    // Show Move cursor when hovering over the selection rect (only when not currently dragging)
    if inside_sel_rect && move_drag.is_none() && drag.is_none() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
    }

    // 命中 AM 子行或 Conductor 主行（Tempo 直显）→ 交给自动化 lane 交互，
    // 此处不起音符框选/移动拖拽。
    let on_am_row = pointer.hover_pos().is_some_and(|pos| {
        hit_rect.contains(pos)
            && match row_layout.hit_at_music_y(
                pos.y - content_rect.min.y + view.base.scroll_y,
                view.lane_height(),
            ) {
                Some(ArRow::Automation(..)) => true,
                Some(ArRow::Track(t)) => data.conductor_track_idx == Some(t as u16),
                None => false,
            }
    });

    // ── Primary press handling ──
    if pointer.primary_pressed()
        && !on_am_row
        && let Some(pos) = pointer.hover_pos()
        && hit_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let click_tick = view.x_to_tick(local.x);
        // 行命中 → 音轨索引（展开后行号与音轨不再一一对应，AM 行归到所属音轨）
        let click_track_f = row_layout
            .hit_at_music_y(local.y + view.base.scroll_y, view.lane_height())
            .map(|h| h.track() as f32)
            .unwrap_or(0.0);

        if inside_sel_rect && !additive {
            // Start move-drag of existing selection
            // 保存原选框快照并清空（拖拽中由 move_orig_sel 计算偏移后的选框显示）
            move_orig_sel = edit.arr_sel_rect.clone();
            edit.arr_sel_rect.clear();
            let origin = (click_tick, click_track_f);
            // press 时锁定 alt（复制模式），拖拽中切换不影响本次操作。
            let alt = ui.input(|i| i.modifiers.alt);
            move_drag = Some((origin, origin, alt));
            drag = None;
        } else {
            // Start new selection marquee
            let start_track_y = (local.y + view.base.scroll_y) / view.lane_height();
            drag = Some(((click_tick, start_track_y), local));
            // 累加模式下保留已有选框，否则清空
            if !additive {
                edit.arr_sel_rect.clear();
                edit.selected.clear();
            }
        }
    }

    // ── Move-drag: update current position ──
    if let Some((origin, _, alt)) = move_drag
        && pointer.primary_down()
        && !pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let current_tick = view.x_to_tick(local.x);
        let current_track_f = row_layout
            .hit_at_music_y(local.y + view.base.scroll_y, view.lane_height())
            .map(|h| h.track() as f32)
            .unwrap_or(0.0);
        move_drag = Some((origin, (current_tick, current_track_f), alt));

        // Auto-scroll when dragging near the edge
        let lh = view.lane_height();
        let full_w = content_rect.width();
        crate::selection::drag::auto_scroll_on_drag(
            ui,
            &mut view.base,
            hit_rect,
            pos,
            |base, _, h| {
                base.clamp_scroll_x(full_w, data.total_ticks);
                let max_scroll_y = (row_layout.total_rows() as f32 * lh - h).max(0.0);
                base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
            },
        );
    }

    // ── Generate ghost notes + offset sel_rect from current move_drag (BEFORE release) ──
    // Ghost notes must be generated before release clears move_drag, so the ghost
    // stays visible on the release frame (preventing flicker before model update).
    if let Some(((origin_t, origin_tr), (current_t, current_tr), alt)) = move_drag
        && !move_orig_sel.is_empty()
    {
        let snapped_origin = crate::view_interaction::snap_tick(
            origin_t,
            data.quantize,
            data.ppq,
            data.bar_line_data,
        );
        let snapped_current = crate::view_interaction::snap_tick(
            current_t,
            data.quantize,
            data.ppq,
            data.bar_line_data,
        );
        let dt = (snapped_current - snapped_origin).round() as i64;
        // 垂直选框工具：只能水平移动，dtr 强制为 0
        let dtr = if vertical {
            0
        } else {
            (current_tr - origin_tr).round() as i32
        };

        // 拖拽中：把 edit.arr_sel_rect 设为所有偏移后的选框，由 show() 统一绘制
        *edit.arr_sel_rect = move_orig_sel
            .iter()
            .map(|&(t_start, t_end, track_lo, track_hi)| {
                (
                    t_start + dt as f64,
                    t_end + dt as f64,
                    track_lo.saturating_add_signed(dtr as isize),
                    track_hi.saturating_add_signed(dtr as isize),
                )
            })
            .collect();

        // Generate ghost notes at new positions + hide originals
        if dt != 0 || dtr != 0 {
            let max_track = (data.num_tracks as i32 - 1).max(0) as u16;

            // 与 PR 共用的选中音符收集（edit.track_selected 传空集合 = 不过滤轨道）。
            // edit.selected.rects 在拖拽中保持原快照，与 move_orig_sel 语义一致。
            // AR 无 editing_track 概念，传 None。
            let notes = crate::selection::drag::collect_selected_notes(
                edit.selected,
                data.midi,
                data.track_visible,
                &HashSet::new(),
                None,
            );
            for note in notes {
                let new_tick = (note.start_tick as i64 + dt).max(0) as u32;
                let length = note.end_tick - note.start_tick;
                let new_track = (note.track as i32 + dtr).max(0).min(max_track as i32) as u16;
                ghost_notes.push((new_tick, new_tick + length, note.key, new_track));
                // Alt（复制模式）：原音符保留可见，不隐藏。
                if !alt {
                    hidden_notes.insert((note.track, note.start_tick, note.key));
                }
            }
        }
    }

    // ── Move-drag: release handling ──
    if move_drag.is_some() && pointer.primary_released() {
        if let Some(((origin_t, origin_tr), (current_t, current_tr), alt)) = move_drag {
            let snapped_origin = crate::view_interaction::snap_tick(
                origin_t,
                data.quantize,
                data.ppq,
                data.bar_line_data,
            );
            let snapped_current = crate::view_interaction::snap_tick(
                current_t,
                data.quantize,
                data.ppq,
                data.bar_line_data,
            );
            let delta_ticks = (snapped_current - snapped_origin).round() as i64;
            // 垂直选框工具：只能水平移动，delta_tracks 强制为 0
            let delta_tracks = if vertical {
                0
            } else {
                (current_tr - origin_tr).round() as i32
            };

            let has_moved = delta_ticks != 0 || delta_tracks != 0;

            if has_moved {
                *edit.arr_drag_delta = Some((delta_ticks, delta_tracks, alt));

                // 多选框：对所有原选框应用偏移
                *edit.arr_sel_rect = move_orig_sel
                    .iter()
                    .map(|&(t_start, t_end, track_lo, track_hi)| {
                        (
                            t_start + delta_ticks as f64,
                            t_end + delta_ticks as f64,
                            track_lo.saturating_add_signed(delta_tracks as isize),
                            track_hi.saturating_add_signed(delta_tracks as isize),
                        )
                    })
                    .collect();
                view.base.dirty = true;
            } else {
                *edit.arr_sel_rect = move_orig_sel.clone();
            }
        }
        move_drag = None;
        move_orig_sel.clear();
        drag_rect = None; // edit.arr_sel_rect takes over on release
    }

    // ── Selection marquee drag handling ──
    if let Some((start_music, _)) = drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            let clamped = pos.clamp(hit_rect.min, hit_rect.max);
            let local = egui::pos2(
                clamped.x - content_rect.min.x,
                clamped.y - content_rect.min.y,
            );
            drag = Some((start_music, local));

            let lh = view.lane_height();
            let full_w = content_rect.width();
            crate::selection::drag::auto_scroll_on_drag(
                ui,
                &mut view.base,
                hit_rect,
                pos,
                |base, _, h| {
                    base.clamp_scroll_x(full_w, data.total_ticks);
                    let max_scroll_y = (row_layout.total_rows() as f32 * lh - h).max(0.0);
                    base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
                },
            );
        }

        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height() - view.base.scroll_y,
        );

        // Compute marquee drag_rect (BEFORE release, same pattern as move-drag)
        if let Some((_, end)) = drag
            && (end - start_pixel).length() >= 3.0
            && let Some(b) =
                arrange_snapped_bounds(start_pixel, end, view, row_layout, data, vertical)
        {
            drag_rect = Some(egui::Rect::from_min_max(
                egui::pos2(b.view_sx.min(b.view_ex), b.view_sy.min(b.view_ey)),
                egui::pos2(b.view_sx.max(b.view_ex), b.view_sy.max(b.view_ey)),
            ));
        }

        if pointer.primary_released() {
            if let (Some(_midi_ref), Some((_, end))) = (data.midi, drag) {
                let drag_dist = (end - start_pixel).length();

                if drag_dist < 3.0 {
                    let tick = view.x_to_tick(start_pixel.x);
                    let snapped = crate::view_interaction::snap_tick(
                        tick,
                        data.quantize,
                        data.ppq,
                        data.bar_line_data,
                    );
                    edit.selected.clear();
                    edit.arr_sel_rect.clear();
                    *edit.cursor_tick = Some(snapped.max(0.0));

                    // 点击时同时选中对应音轨
                    let track_arr_idx = start_music.1.floor() as usize;
                    if track_arr_idx < data.num_tracks {
                        let track_idx = data.track_info[track_arr_idx].index;
                        edit.track_selected.clear();
                        edit.track_selected.insert(track_idx);
                        *edit.selection_anchor = Some(track_idx);
                        *edit.info_content = Some(crate::right_panel::InfoContent::Track);
                    }
                } else {
                    if let Some(b) =
                        arrange_snapped_bounds(start_pixel, end, view, row_layout, data, vertical)
                    {
                        // shift 或 cmd/ctrl 累加模式：保留已有选框；否则清空
                        if !additive {
                            edit.selected.clear();
                            edit.arr_sel_rect.clear();
                        }
                        edit.selected.add_rect_track(
                            b.t_start as u32,
                            b.t_end as u32,
                            0,
                            127,
                            b.track_lo as u16,
                            b.track_hi as u16,
                        );
                        edit.arr_sel_rect
                            .push((b.t_start, b.t_end, b.track_lo, b.track_hi));
                    } else if !additive {
                        // 选框完全在空白区域：清空选区
                        edit.selected.clear();
                        edit.arr_sel_rect.clear();
                    }
                }
                view.base.dirty = true;
            }
            drag = None;
            drag_rect = None; // edit.arr_sel_rect takes over on release
        }
    }

    ui.data_mut(|d| d.insert_persisted(sel_id, drag));
    ui.data_mut(|d| d.insert_persisted(move_drag_id, move_drag));
    ui.data_mut(|d| d.insert_persisted(move_orig_id, move_orig_sel));

    (ghost_notes, hidden_notes, drag_rect)
}

// ── Arrangement eraser tool ──

/// Eraser-tool marquee drag for the arrangement view.
/// On release, sets `arr_eraser_rect` which triggers deletion in the caller.
fn eraser_drag_frame_arrange(
    ui: &mut egui::Ui,
    content_rect: egui::Rect,
    hit_rect: egui::Rect,
    view: &mut ArrangementView,
    row_layout: &ArRowLayout,
    data: &super::ArrangeData<'_>,
    edit: &mut super::ArrangeEdit<'_>,
) {
    let drag_id = ui.id().with("eraser_drag_arr");
    let mut drag: Option<((f64, f32), egui::Pos2)> =
        ui.data_mut(|d| d.get_persisted(drag_id)).unwrap_or(None);

    let pointer = ui.input(|i| i.pointer.clone());

    // Clear stale drag state
    if drag.is_some() && !pointer.primary_down() && !pointer.primary_released() {
        drag = None;
    }

    if crate::view_interaction::pointer_over_popup(ui.ctx()) {
        ui.data_mut(|d| d.insert_persisted(drag_id, drag));
        return;
    }

    // Press → start drag (store music coordinates: (tick, 行号浮点))
    // 命中 AM 子行 / Conductor 主行 → 交给自动化 lane 的橡皮擦交互。
    if pointer.primary_pressed()
        && let Some(pos) = pointer.hover_pos()
        && hit_rect.contains(pos)
        && !match row_layout.hit_at_music_y(
            pos.y - content_rect.min.y + view.base.scroll_y,
            view.lane_height(),
        ) {
            Some(ArRow::Automation(..)) => true,
            Some(ArRow::Track(t)) => data.conductor_track_idx == Some(t as u16),
            None => false,
        }
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let start_tick = view.x_to_tick(local.x);
        let start_track_f = (local.y + view.base.scroll_y) / view.lane_height();
        drag = Some(((start_tick, start_track_f), local));
        *edit.arr_eraser_rect = None;
    }

    // Move → update with auto-scroll
    if let Some((start_music, _)) = drag {
        if pointer.primary_down()
            && !pointer.primary_pressed()
            && let Some(pos) = pointer.hover_pos()
        {
            let clamped = pos.clamp(hit_rect.min, hit_rect.max);
            let local = egui::pos2(
                clamped.x - content_rect.min.x,
                clamped.y - content_rect.min.y,
            );
            drag = Some((start_music, local));

            let lh = view.lane_height();
            let full_w = content_rect.width();
            crate::selection::drag::auto_scroll_on_drag(
                ui,
                &mut view.base,
                hit_rect,
                pos,
                |base, _, h| {
                    base.clamp_scroll_x(full_w, data.total_ticks);
                    let max_scroll_y = (row_layout.total_rows() as f32 * lh - h).max(0.0);
                    base.scroll_y = base.scroll_y.clamp(0.0, max_scroll_y);
                },
            );
        }

        let start_pixel = egui::pos2(
            view.tick_to_x(start_music.0),
            start_music.1 * view.lane_height() - view.base.scroll_y,
        );

        // Release → compute snapped bounds, set eraser rect
        if pointer.primary_released() {
            if let Some((_, end)) = drag {
                if (end - start_pixel).length() >= 3.0
                    && let Some(b) =
                        arrange_snapped_bounds(start_pixel, end, view, row_layout, data, false)
                {
                    *edit.arr_eraser_rect = Some((b.t_start, b.t_end, b.track_lo, b.track_hi));
                }
                view.base.dirty = true;
            }
            drag = None;
        }
    }

    ui.data_mut(|d| d.insert_persisted(drag_id, drag));
}

/// 吸附后的选框边界：view 局部坐标 + tick/track 范围。
struct ArrSnappedBounds {
    view_sx: f32,
    view_ex: f32,
    view_sy: f32,
    view_ey: f32,
    t_start: f64,
    t_end: f64,
    track_lo: usize,
    track_hi: usize,
}

/// Compute snapped selection bounds for arrangement.
/// 行号（均匀行空间）→ 音轨索引用 ArRowLayout 换算：AM 子行归到所属音轨。
fn arrange_snapped_bounds(
    start: egui::Pos2,
    end: egui::Pos2,
    view: &ArrangementView,
    row_layout: &ArRowLayout,
    data: &super::ArrangeData<'_>,
    vertical: bool,
) -> Option<ArrSnappedBounds> {
    let sx = start.x.min(end.x);
    let ex = start.x.max(end.x);

    let tick_s = view.x_to_tick(sx);
    let tick_e = view.x_to_tick(ex);
    let snapped_s =
        crate::view_interaction::snap_tick(tick_s, data.quantize, data.ppq, data.bar_line_data);
    let snapped_e =
        crate::view_interaction::snap_tick(tick_e, data.quantize, data.ppq, data.bar_line_data);
    let t_start = snapped_s.min(snapped_e);
    let mut t_end = snapped_s.max(snapped_e);

    // Ensure minimum width of one quantise grid interval
    let interval = data.quantize.tick_interval(data.ppq) as f64;
    if t_end <= t_start {
        t_end = t_start + interval.max(1.0);
    }

    if data.num_tracks == 0 {
        return None;
    }

    let lh = view.lane_height();
    let scroll_y = view.base.scroll_y;

    // 垂直全选模式：track 范围固定 0..num_tracks-1，忽略鼠标 y
    let (track_lo, track_hi, view_sy, view_ey) = if vertical {
        let th = data.num_tracks - 1;
        (0, th, 0.0, data.num_tracks as f32 * lh - scroll_y)
    } else {
        let sy = start.y.min(end.y);
        let ey = start.y.max(end.y);
        let row_lo = ((scroll_y + sy) / lh).floor().max(0.0) as usize;
        let row_hi = ((scroll_y + ey) / lh).floor().max(0.0) as usize;
        // 边界判断：选框必须与实际内容区域有重叠，否则不纳入选择范围。
        if row_lo >= row_layout.total_rows() {
            return None;
        }
        // 行 → 音轨：AM 子行归到所属音轨（选框自然覆盖该轨全部展开行）。
        let track_lo = row_layout.row_hit(row_lo).map(|h| h.track()).unwrap_or(0);
        let track_hi = row_layout
            .row_hit(row_hi.min(row_layout.total_rows().saturating_sub(1)))
            .map(|h| h.track())
            .unwrap_or(0);
        let view_sy = row_layout.track_y(track_lo, lh) - scroll_y;
        let view_ey =
            row_layout.track_y(track_hi, lh) + row_layout.track_height(track_hi, lh) - scroll_y;
        (track_lo, track_hi, view_sy, view_ey)
    };

    let view_sx = view.tick_to_x(t_start);
    let view_ex = view.tick_to_x(t_end);

    Some(ArrSnappedBounds {
        view_sx,
        view_ex,
        view_sy,
        view_ey,
        t_start,
        t_end,
        track_lo,
        track_hi,
    })
}
