use eframe::egui;
use rust_i18n::t;

use yinhe_types::time_format::format_tick_bar_beat_with_time_sig;
use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::widgets::selection_actions::SelectionAction;
use crate::widgets::tools_panel::Tool;
pub use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::PencilNoteDrag;

pub mod automation_panel;
mod bg;
pub(crate) mod drag;
pub(crate) mod gpu_upload;
mod keyboard;
mod marquee;
mod pencil;
mod perf;
mod quantize_button;

/// Events emitted by the piano-roll view for the caller to act on.
pub enum PianoViewEvent {
    SelectionAction(SelectionAction),
    AddNote {
        track: u16,
        note: yinhe_core::NoteEvent,
    },
    EraserDelete {
        t_start: u32,
        t_end: u32,
        key_lo: u8,
        key_hi: u8,
        track_lo: u16,
        track_hi: u16,
    },
    QuantizePreset(QuantizePreset),
}

/// Automation panel 上下文（all-or-nothing：要么全 Some 要么全 None）。
/// 合并 5 个 auto_* 参数，减少 piano_view::show 的参数数量。
pub struct AutomationPanelsCtx<'a> {
    pub panels: &'a mut Vec<yinhe_types::AutomationPanelView>,
    pub renderers: &'a mut Vec<(
        yinhe_wgpu::InstanceRenderer,
        crate::render_context::RenderContext,
    )>,
    pub lanes: &'a [yinhe_types::AutomationLane],
    /// 渲染用 lanes：所有 PR 可见音轨的 lanes（与音符显示逻辑一致）。
    /// `lanes` 仅为 editing_track 的编辑目标，渲染不受其限制。
    pub render_lanes: &'a [&'a yinhe_types::AutomationLane],
    pub show: &'a mut bool,
    pub wgpu_state: &'a std::sync::Arc<eframe::egui_wgpu::RenderState>,
}

/// 音符听觉预览请求（UI 交互 → App → AudioCommand）。
/// 预览音从目标音轨的通道发出，通道状态按目标位置（target_tick）的自动化。
pub(crate) enum PreviewReq {
    /// 播放/重触发一个音符预览。`duration_ticks == 0` 表示持续音（直到 `Stop`）。
    Note(NotePreview),
    /// 停止持续音预览。
    Stop,
}

/// 单个音符的预览参数。
pub(crate) struct NotePreview {
    pub track: u16,
    pub key: u8,
    /// `None` = 用该音轨最近修改力度（default_velocity）。
    pub velocity: Option<u8>,
    /// 目标位置 tick：自动化状态采样点（音符起点）。
    pub target_tick: u32,
    /// 预览时长（tick），0 = 持续音（配合 `PreviewReq::Stop`）。
    pub duration_ticks: u32,
}

/// piano_view 给外部的反馈通道（合并多个 &mut 出参）。
pub struct PianoViewFeedback<'a> {
    pub auto_edit_events: &'a mut Vec<crate::piano_view::automation_panel::AutomationEdit>,
    pub info_content: &'a mut Option<crate::right_panel::InfoContent>,
    pub right_tab: &'a mut Option<crate::right_panel::RightTab>,
    pub automation_drag_ghost: &'a mut Option<(u32, f32)>,
    pub note_drag_delta: &'a mut Option<(i64, i32, bool)>,
    pub pencil_note_drag: &'a mut Option<PencilNoteDrag>,
    /// 选框边缘拖动伸缩：(side, delta_ticks)。dt 按量化对齐。
    pub note_resize_delta: &'a mut Option<(ResizeSide, i64)>,
    pub velocity_edits: &'a mut Vec<yinhe_types::VelocityEdit>,
    /// 音符听觉预览请求（铅笔新建/拖拽、选框拖拽触发）。
    pub preview_reqs: &'a mut Vec<PreviewReq>,
    /// 状态栏讲解行：钢琴卷帘悬停提示（位置 + 音高）。
    pub status_hint: &'a mut Option<String>,
}

/// Height of the time ruler band at the top of the pianoroll view.
use crate::theme;
const RULER_H: f32 = theme::RULER_H;

/// Display the pianoroll texture with zoom/pan interaction.
///
/// When `auto_*` parameters are `Some`, automation panels are rendered between
/// the pianoroll content and the horizontal scrollbar. The AUTO toggle and
/// +/- buttons live inside the scrollbar's left blank area (same width as the
/// piano keyboard).
///
/// Returns an optional event for the caller to handle (selection action or
/// note-add request).
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    available: egui::Vec2,
    pianoroll: &mut yinhe_wgpu::InstanceRenderer,
    render_ctx: &mut super::render_context::RenderContext,
    render_thread: Option<&yinhe_wgpu::RenderThreadHandle>,
    view: &mut yinhe_types::PianoRollView,
    last_cull_revision: &mut u64, // revision ^ hidden_hash — triggers all_notes re-upload
    last_cull_revision_only: &mut u64, // last revision for incremental detection
    last_hidden_hash: &mut u64,   // last hidden_hash for incremental detection
    last_tv_hash: &mut u64,       // last track_visible hash (track_mask 变化检测)
    cull_rebuild: &mut Option<crate::piano_view::gpu_upload::CullRebuild>, // 后台重建状态机
    midi: Option<&dyn yinhe_types::NoteSource>,
    midi_arc: Option<&std::sync::Arc<yinhe_core::YinModel>>,
    selected: &mut yinhe_core::Selection,
    track_visible: &[bool],
    track_colors: &[[f32; 4]],
    cursor_tick: &mut Option<f64>,
    is_playing: bool,
    quantize: QuantizePreset,
    ppq: u32,
    bar_line_data: Option<(u32, u8, u8, &[TimeSigEvent])>,
    key_sig_events: &[yinhe_types::KeySigEvent],
    last_cursor_tick: &mut Option<f64>,
    follow_mode: &mut super::view_interaction::FollowMode,
    active_tool: &Tool,
    // Automation panel data (all-or-nothing)
    mut auto_ctx: Option<AutomationPanelsCtx<'_>>,
    scroll_mode: u32,
    min_border_width: f32,
    note_outline: bool,
    use_gpu_cull: bool,
    tempo_lane: &AutomationLane,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    editing_track: Option<u16>,
    revision: u64,
    note_revisions: &[u64; 128],
    feedback: &mut PianoViewFeedback<'_>,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
) -> Option<PianoViewEvent> {
    // Sense::hover() — no drag ownership. All drag is handled by dedicated
    // ui.interact calls below, each inside its own push_id scope.
    let (resp, painter) = ui.allocate_painter(available, egui::Sense::hover());
    let rect = resp.rect;

    // Compute automation panel natural total height.
    // First panel has no leading handle; subsequent panels have SPLIT_H above them.
    let panels_natural_h: f32 = match &auto_ctx {
        Some(ctx) if *ctx.show && !ctx.panels.is_empty() => {
            ctx.panels.iter().map(|p| p.panel_height).sum::<f32>()
                + (ctx.panels.len() as f32 * automation_panel::SPLIT_H)
        }
        _ => 0.0,
    };

    // Cap panels area to prevent overflow when too many panels.
    // Reserve at least 35% of available height for the pianoroll content;
    // excess panels become scrollable.
    let avail_h = rect.height() - RULER_H - crate::widgets::scrollbar::SCROLLBAR_H;
    let panels_max_h = (avail_h * 0.65).max(0.0);
    let panels_total_h = panels_natural_h.min(panels_max_h);

    // 内容右边界：让出 SCROLLBAR_W 给垂直滚动条
    let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;

    // Layout: ruler | pianoroll content | automation panels | scrollbar
    let ruler_band_y = rect.min.y;
    let content_y = rect.min.y + RULER_H;
    let content_h = (avail_h - panels_total_h).max(0.0);
    let content_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x, content_y),
        egui::pos2(content_right_x, content_y + content_h),
    );
    let kb_w = view.keyboard_width();
    let music_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + kb_w, content_y),
        egui::pos2(content_right_x, content_y + content_h),
    );
    let ppp = ui.ctx().pixels_per_point();
    let w = content_rect.width() as u32;
    let h = content_rect.height() as u32;
    let pw = (w as f32 * ppp) as u32;
    let ph = (h as f32 * ppp) as u32;

    if w == 0 || h == 0 {
        return None;
    }

    // ── Perf probe (only when YIN_PERF=1) ──
    let perf_on = yinhe_memtrace::perf_probe::enabled();
    let t_show_start = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Resize render target if needed — texture_id may change after this
    render_ctx.ensure_size(pw, ph);

    // Update render thread's target view if texture was recreated
    if let Some(rt) = render_thread {
        rt.update_target(render_ctx.preview_view().clone(), pw, ph);
    }

    // Clamp scroll — add some extra space beyond the last note
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // Auto-follow: scroll based on follow mode (playback only).
    // Never auto-follow when paused, so the user can freely scroll around.
    if let Some(ct) = *cursor_tick
        && is_playing
        && *follow_mode != super::view_interaction::FollowMode::None
        && let Some(new_scroll_x) = super::view_interaction::compute_follow_scroll(
            ct,
            view.base.pixels_per_tick,
            w as f32,
            view.keyboard_width(),
            *follow_mode,
            1.0,
        )
    {
        view.base.scroll_x = new_scroll_x;
        view.clamp_scroll(w as f32, h as f32, total_ticks);
    }

    // ── Selection drag (Select tool only) ──
    // Update state BEFORE handle_input to avoid egui pointer-capture conflicts.
    let mut sel_action = None;
    let mut pencil_event: Option<PianoViewEvent> = None;
    let mut eraser_event: Option<PianoViewEvent> = None;
    let mut ghost_notes: Vec<(u32, u32, u8, u16)> = Vec::new();
    let mut hidden_notes: std::collections::HashSet<(u16, u32, u8)> =
        std::collections::HashSet::new();
    if *active_tool == Tool::Select || *active_tool == Tool::SelectVertical {
        let vertical = *active_tool == Tool::SelectVertical;
        let (sel_ghosts, sel_hidden, sel_previews, sel_note_event, sel_pencil_drag) =
            drag::sel_drag_frame(
                ui,
                content_rect,
                music_rect,
                view,
                midi,
                selected,
                quantize,
                ppq,
                bar_line_data,
                total_ticks,
                cursor_tick,
                feedback.note_drag_delta,
                feedback.note_resize_delta,
                sel_rect,
                track_colors,
                track_visible,
                track_selected,
                editing_track,
                conductor_idx,
                vertical,
            );
        ghost_notes = sel_ghosts;
        hidden_notes = sel_hidden.into_iter().collect();
        feedback.preview_reqs.extend(sel_previews);
        // 双击写音符（选择工具）：与铅笔一致，目标轨 = editing_track。
        if let Some((note, track)) = sel_note_event {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
        // 单音符边缘伸缩（选择工具，不用先选中）：复用铅笔的提交通道。
        *feedback.pencil_note_drag = sel_pencil_drag;
    } else if *active_tool == Tool::Pencil {
        let (note_event, ghost, hidden, pencil_drag, preview) = pencil::pencil_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            editing_track,
            track_visible,
            conductor_idx,
            midi,
            track_colors,
            total_ticks,
        );
        ghost_notes = ghost;
        hidden_notes.extend(hidden);
        *feedback.pencil_note_drag = pencil_drag;
        if let Some(p) = preview {
            feedback.preview_reqs.push(p);
        }
        if let Some(note) = note_event
            && let Some(track) =
                pencil::valid_pencil_track(editing_track, track_visible, conductor_idx)
        {
            pencil_event = Some(PianoViewEvent::AddNote { track, note });
        }
    } else if *active_tool == Tool::Eraser {
        eraser_event = marquee::eraser_drag_frame(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            total_ticks,
            track_selected,
            editing_track,
        );
    }

    // ── Hover cursor: show Move/ResizeWest/ResizeEast when over selection rect ──
    if (*active_tool == Tool::Select || *active_tool == Tool::SelectVertical)
        && !crate::view_interaction::pointer_over_popup(ui.ctx())
        && let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && music_rect.contains(pos)
    {
        let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
        let eff_rects = sel_rect.effective_rects();
        // 音符 hit-test 优先（不用先选中，与铅笔一致）：
        // 边缘 → 伸缩光标；中部 → 移动光标。
        if let Some((mode, _, _, _, _)) = drag::hit_test_note(
            midi,
            view,
            local,
            track_visible,
            track_selected,
            editing_track,
        ) {
            use crate::piano_view::pencil::HitMode;
            match mode {
                HitMode::ResizeLeft => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest),
                HitMode::ResizeRight => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast),
                HitMode::Move => ui.ctx().set_cursor_icon(egui::CursorIcon::Move),
            }
        } else if let Some((side, _, _)) =
            drag::hit_test_sel_edge(&eff_rects, &view.base, view.key_height, local)
        {
            match side {
                yinhe_editor_core::ResizeSide::Left => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeWest)
                }
                yinhe_editor_core::ResizeSide::Right => {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeEast)
                }
            }
        } else {
            let in_sel_rect = eff_rects.iter().any(|&(t_start, t_end, key_lo, key_hi)| {
                let pixel_rect = crate::selection::drag::music_sel_to_pixel_rect(
                    &view.base,
                    view.key_height,
                    t_start,
                    t_end,
                    key_lo,
                    key_hi,
                );
                pixel_rect.contains(local)
            });
            if in_sel_rect {
                // 垂直选框工具：只能水平拖动 → 左右双向指针；
                // 普通选框工具：四向移动指针。
                let icon = if *active_tool == Tool::SelectVertical {
                    egui::CursorIcon::ResizeHorizontal
                } else {
                    egui::CursorIcon::Move
                };
                ui.ctx().set_cursor_icon(icon);
            }
        }
    }

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    // 传 content_rect（含键盘列）+ left_zone_width=kb_w，让 handle_input 统一处理
    // 键盘区垂直缩放与卷帘区平移/水平缩放。x_to_tick 内部减 left_panel_width，
    // 所以传入相对 content 的 x 才正确（之前传 music_rect 导致 kb_w 被减两次）。
    crate::view_interaction::handle_input(
        ui,
        content_rect,
        view,
        cursor_tick,
        kb_w,
        Some((quantize, ppq)),
        bar_line_data,
        None,
        is_playing,
        follow_mode,
        active_tool,
    );

    // ── Keyboard resize handle ──
    ui.push_id("kb_handle", |ui| {
        let handle_x = rect.min.x + view.keyboard_width();
        let handle_rect = egui::Rect::from_min_max(
            egui::pos2(handle_x - 2.0, rect.min.y),
            egui::pos2(handle_x + 2.0, content_rect.max.y),
        );
        let handle_resp = ui.interact(handle_rect, ui.id(), egui::Sense::click_and_drag());
        if handle_resp.hovered() || handle_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if handle_resp.dragged() {
            let delta = handle_resp.drag_delta().x;
            let old_kb = view.keyboard_width();
            let new_kb = (old_kb + delta).clamp(
                crate::theme::MIN_KEYBOARD_WIDTH,
                rect.width() * crate::theme::MAX_KEYBOARD_RATIO,
            );

            let old_sb_w = w as f32 - old_kb;
            let new_sb_w = w as f32 - new_kb;
            if old_sb_w > 0.0 && new_sb_w > 0.0 {
                let start_tick = view.base.scroll_x / view.base.pixels_per_tick;
                let new_start_tick = start_tick * old_sb_w / new_sb_w;
                view.base.scroll_x = new_start_tick * view.base.pixels_per_tick;
            }

            view.base.left_panel_width = new_kb;
            view.base.dirty = true;
            ui.ctx().request_repaint();
        }
    });

    // ── Clamp scroll after all interactions ──
    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.map(|m| m.tick_length().unwrap_or(0)).unwrap_or(0),
        ppq,
    );
    view.clamp_scroll(w as f32, h as f32, total_ticks);

    // ── Dirty detection ──
    // cursor_tick no longer affects rendering at all — the cursor is drawn
    // by egui directly on top of the wgpu texture, outside the cache.
    // app_eframe already calls request_repaint while audio is playing.
    *last_cursor_tick = *cursor_tick;

    // Perf probe: capture input phase duration (everything up to prepare).
    let t_input_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Upload all notes to GPU cull buffer ──
    // Only when GPU cull mode is enabled. CPU build mode skips this entirely.
    // 增量 per-key 上传 → 失败回退全量上传。详见 gpu_upload 模块。
    if use_gpu_cull {
        gpu_upload::upload(gpu_upload::GpuUploadState {
            pianoroll,
            midi,
            midi_arc,
            revision,
            note_revisions,
            track_visible,
            hidden_notes: &hidden_notes,
            last_cull_revision,
            last_cull_revision_only,
            last_hidden_hash,
            last_tv_hash,
            rebuild: cull_rebuild,
        });
    }

    // Prepare GPU data (ghost notes are handled separately as a transient overlay)
    let theme = pianoroll.theme().clone();
    let cull_ready = use_gpu_cull && pianoroll.cull_ready();
    tracing::debug!(
        "[cull-frame] cull_ready={cull_ready} scroll_x={} scroll_y={} ppu={} kh={} w={w} h={h}",
        view.base.scroll_x,
        view.base.scroll_y,
        view.base.pixels_per_tick,
        view.key_height,
    );
    if cull_ready {
        // GPU cull path: upload ghost layer (GPU cull handles notes)
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            &*selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        pianoroll.upload_uniforms(job.uniforms);
        pianoroll.upload_track_colors(&job.track_colors);
        pianoroll.upload_selection(&job.selection);
        // Grid 已迁移到 egui，wgpu 只剩 ghost note overlay 一层。
        pianoroll.ensure_layers(1);
        // Ghost note overlay: built and uploaded independently.
        // Always upload (even when empty) to clear stale ghost data from the previous frame.
        pianoroll.upload_note_layer(0, 0, |out| {
            for &(start_tick, end_tick, key, track) in &ghost_notes {
                yinhe_wgpu::build_ghost_note(out, start_tick, end_tick, key, track, &theme);
            }
        });
    } else if let Some(rt) = render_thread {
        // Async path (no cull): build instances on this thread, send to render thread
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            &*selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        // Build note instances + ghost overlay as note layers for the render thread.
        let mut notes_instances = Vec::new();
        if let Some(midi) = midi {
            yinhe_wgpu::build_notes(
                &mut notes_instances,
                w as f32,
                h as f32,
                midi,
                view,
                &hidden_notes,
                track_visible,
            );
        }
        let mut ghost_instances = Vec::new();
        for &(start_tick, end_tick, key, track) in &ghost_notes {
            yinhe_wgpu::build_ghost_note(
                &mut ghost_instances,
                start_tick,
                end_tick,
                key,
                track,
                &theme,
            );
        }
        // Cache key for notes: includes viewport tick/key range, revision,
        // hidden_notes, and track_visible — anything that affects which notes
        // are built. When none of these change (e.g. steady state, no scroll/
        // edit), the render thread skips GPU upload entirely.
        let (tick_start, tick_end) = view.visible_tick_range(w as f32);
        let (key_lo, key_hi) = view.visible_key_range(h as f32);
        let tv_hash = yinhe_wgpu::hash_bools(track_visible);
        let hidden_hash = yinhe_wgpu::hash_hidden(&hidden_notes);
        let notes_cache_key = yinhe_wgpu::layer_cache_key(&[
            tick_start.to_bits(),
            tick_end.to_bits(),
            key_lo as u64,
            key_hi as u64,
            tv_hash,
            revision,
            hidden_hash,
        ]);
        let note_layers = vec![
            yinhe_wgpu::NoteLayerData {
                instances: notes_instances,
                cache_key: notes_cache_key,
                force: false,
            },
            yinhe_wgpu::NoteLayerData {
                instances: ghost_instances,
                cache_key: 0,
                force: true,
            },
        ];
        rt.send_job(yinhe_wgpu::RenderJob {
            width: job.width,
            height: job.height,
            uniforms: job.uniforms,
            track_colors: job.track_colors,
            selection: job.selection,
            note_layers,
        });
    }

    let t_prepare_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;

    // ── Background (drawn by egui before wgpu texture) ──
    let theme = pianoroll.theme().clone();
    let (r, g, b) = theme.pr_bg;
    painter.rect_filled(
        content_rect,
        0.0,
        egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8),
    );

    // ── Scale background + 八度横线（调号驱动的调内/调外/根音条带）──
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let h_f32 = h as f32;
    let bottom = 128.0 * kh - scroll_y;
    let kb_w = view.keyboard_width();
    bg::paint(&painter, content_rect, kb_w, kh, view, key_sig_events);

    // ── Grid lines (drawn by egui before wgpu texture) ──
    // 替代原 wgpu grid layer。与 time_ruler 共用 MIN_SPACING 阈值，保证"有线就有标签"。
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let grid_rect = egui::Rect::from_min_max(
            egui::pos2(
                content_rect.min.x + view.keyboard_width(),
                content_rect.min.y,
            ),
            content_rect.max,
        );
        crate::widgets::grid_lines::paint_grid_lines(
            &painter,
            grid_rect,
            &view.base,
            tpb,
            def_num,
            def_den,
            sig_events,
            &crate::widgets::grid_lines::GridColors::pianoroll(),
        );
    }

    // Paint wgpu content into the content_rect (notes only — grid moved to egui)
    if cull_ready {
        // GPU cull path: draw directly (no render thread needed — cull makes GPU work fast)
        render_ctx.paint(
            pianoroll,
            pw,
            ph,
            "pianoroll_frame",
            &painter,
            content_rect,
            true,
        );
        // 诊断：每小节打印 CPU 构建数 vs GPU 显示数（YIN_CULL_DIAG=1 时生效）
        pianoroll.cull_diag_bar(view, midi, w as f32, h as f32, &hidden_notes, track_visible);
    } else {
        // Render thread handles GPU work — just display the latest texture
        render_ctx.paint_texture_only(pw, ph, &painter, content_rect);
    }

    let t_paint_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Keyboard (drawn by egui on top of the wgpu texture) ──
    keyboard::paint(&painter, content_rect, kb_w, kh, bottom, h_f32, &theme);

    // ── Playback cursor (drawn by egui on top of the wgpu texture) ──
    // Decoupled from the wgpu pipeline so cursor movement during playback
    // does NOT invalidate the static instance cache.
    if let Some(ct) = *cursor_tick {
        let kb_w = view.keyboard_width();
        let cx_local = view.tick_to_x(ct);
        if cx_local >= kb_w && cx_local <= w as f32 {
            let cx = content_rect.min.x + cx_local;
            painter.line_segment(
                [
                    egui::pos2(cx, content_rect.min.y),
                    egui::pos2(cx, content_rect.max.y),
                ],
                egui::Stroke::new(crate::theme::CURSOR_WIDTH, crate::theme::CURSOR_COLOR),
            );
        }
    }

    // ── Draw selection box on TOP of GPU content ──
    // State was already updated by sel_drag_frame above; this just draws the box
    // after the GPU paint so it's not covered by the texture.
    if *active_tool == Tool::Select || *active_tool == Tool::SelectVertical {
        let vertical = *active_tool == Tool::SelectVertical;

        // Draw active drag box (if any)
        marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "sel_drag",
            egui::Color32::WHITE,
            egui::Color32::WHITE,
            vertical,
        );

        // Draw persisted selection rects (remains after mouse release).
        // Compute pixel rect from music coordinates each frame so it follows
        // scroll/zoom. 多选框时遍历所有 rects。
        let eff_rects = sel_rect.effective_rects();
        let persisted_pixel_rects: Vec<egui::Rect> = eff_rects
            .iter()
            .map(|&(t_start, t_end, key_lo, key_hi)| {
                crate::selection::drag::music_sel_to_pixel_rect(
                    &view.base,
                    view.key_height,
                    t_start,
                    t_end,
                    key_lo,
                    key_hi,
                )
            })
            .collect();
        {
            let kb_w = music_rect.min.x - content_rect.min.x;
            let music_rect_local = egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(music_rect.width(), music_rect.height()),
            );
            for &rect in &persisted_pixel_rects {
                let shifted = egui::Rect::from_min_max(
                    egui::pos2(rect.min.x - kb_w, rect.min.y),
                    egui::pos2(rect.max.x - kb_w, rect.max.y),
                );
                if shifted.intersects(music_rect_local) {
                    crate::selection::draw::draw(
                        ui.painter(),
                        music_rect,
                        shifted,
                        egui::Color32::WHITE,
                        egui::Color32::WHITE,
                    );
                }
            }
        }

        // Show floating action bar next to the latest persisted selection rect
        if let Some(action) = crate::widgets::selection_actions::show(
            ui,
            music_rect,
            persisted_pixel_rects.last().copied(),
        ) {
            sel_action = Some(action);
        }
    } else if *active_tool == Tool::Eraser {
        // Draw eraser marquee box in red
        marquee::draw_marquee_box(
            ui,
            content_rect,
            music_rect,
            view,
            quantize,
            ppq,
            bar_line_data,
            "eraser_drag",
            egui::Color32::RED,
            egui::Color32::RED,
            false,
        );
    }

    // ── Time ruler ──
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let ruler_rect = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + view.keyboard_width(), ruler_band_y),
            egui::pos2(content_right_x, ruler_band_y + RULER_H),
        );
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        let ruler_jumped = crate::widgets::time_ruler::interactive_ruler(
            ui,
            ruler_rect,
            view,
            tpb,
            def_num,
            def_den,
            sig_events,
            |tick| crate::view_interaction::snap_tick(tick, quantize, ppq, bar_line_data),
            "piano_ruler",
            cursor_tick,
        );
        // 点击/拖动时间标尺跳转位置时，取消已选择的选框（含框选与全选）。
        if ruler_jumped {
            selected.clear();
            sel_rect.clear();
        }
    }

    // ── Automation panels ──
    let panels_y = content_rect.max.y;
    // 自动化面板的状态栏提示（鼠标在面板 grid_area 内时由 show_panels 写入）
    let mut panels_status_hint: Option<String> = None;
    if let Some(ctx) = auto_ctx.as_mut() {
        let kb_w = view.keyboard_width();
        let combo_w = kb_w * theme::AUTO_PANEL_COMBO_WIDTH_RATIO;

        // automation 编辑上下文：Pencil/Curve/Select/SelectVertical 工具时启用。
        // active_track 由 editing_track 决定（与 pencil 一致），但 Conductor 除外：
        // Conductor 不能作为非 Tempo 自动化编辑目标（Tempo 编辑不依赖 active_track，
        // 见 dispatch_edit_interaction）。只需可见即可
        // （editing_track 已常驻 PR 显示，不再要求 track_selected）。
        let active_track = editing_track
            .filter(|&t| track_visible.get(t as usize).copied().unwrap_or(false))
            .filter(|&t| Some(t) != conductor_idx);
        let edit_ctx = if matches!(
            *active_tool,
            Tool::Pencil | Tool::Curve | Tool::Select | Tool::SelectVertical
        ) {
            Some(automation_panel::AutomationEditCtx {
                active_tool: *active_tool,
                active_track,
                quantize,
                ppq,
                bar_line_data,
            })
        } else {
            None
        };

        let mut panels_state = automation_panel::PanelsState {
            panels: ctx.panels,
            renderers: ctx.renderers,
            wgpu_state: ctx.wgpu_state,
            show_panels: ctx.show,
        };
        let panels_data = automation_panel::PanelsData {
            automation_lanes: ctx.lanes,
            render_lanes: ctx.render_lanes,
            tempo_lane,
            midi,
            track_visible,
            track_colors,
        };
        let panels_layout = automation_panel::PanelsLayout {
            combo_width: combo_w,
            content_rect_right: rect.max.x,
            content_top_y: panels_y,
            panels_visible_h: panels_total_h,
        };
        let panels_cfg = automation_panel::PanelsCfg {
            pianoroll_scroll_x: view.base.scroll_x,
            pianoroll_ppt: view.base.pixels_per_tick,
            scroll_mode,
            min_border_width,
            revision,
            bar_line_data,
            sel_hint,
            editing_is_conductor: editing_track == conductor_idx,
        };
        let mut panels_edit = automation_panel::PanelsEdit {
            selected,
            info_content: feedback.info_content,
            right_tab: feedback.right_tab,
        };
        let (_h, auto_edits, velocity_edits, auto_feedback, auto_drag_info) =
            automation_panel::show_panels(
                ui,
                &mut panels_state,
                &panels_data,
                panels_layout,
                panels_cfg,
                &mut panels_edit,
                edit_ctx.as_ref(),
            );
        panels_status_hint = auto_feedback.status_hint.clone();
        for edit in auto_edits {
            feedback.auto_edit_events.push(edit);
        }
        feedback.velocity_edits.extend(velocity_edits);

        // 应用 automation 面板的 pianoroll 联动反馈（水平滚动/缩放）
        if auto_feedback.scroll_x_delta != 0.0 {
            view.base.scroll_x -= auto_feedback.scroll_x_delta;
            view.base.dirty = true;
        }
        if (auto_feedback.zoom_factor - 1.0).abs() > 0.001 {
            view.zoom_around_x(auto_feedback.zoom_center_x, auto_feedback.zoom_factor);
        }

        // 存储 ghost drag info 供信息面板实时显示
        *feedback.automation_drag_ghost = auto_drag_info;

        if midi.is_some() {
            let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
            let sb_left_blank = egui::Rect::from_min_max(
                egui::pos2(rect.min.x, sb_y),
                egui::pos2(
                    rect.min.x + kb_w,
                    sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
                ),
            );
            ui.painter()
                .rect_filled(sb_left_blank, 0.0, theme::SCROLLBAR_BG);
            ui.scope_builder(egui::UiBuilder::new().max_rect(sb_left_blank), |ui| {
                ui.horizontal_centered(|ui| {
                    let mut count = ctx.panels.len();
                    automation_panel::show_toggle_buttons(ui, ctx.show, &mut count);
                    while ctx.panels.len() < count {
                        ctx.panels.push(yinhe_types::AutomationPanelView::default());
                    }
                    while ctx.panels.len() > count {
                        ctx.panels.pop();
                    }
                });
            });
        }
    }

    // ── Horizontal scrollbar ──
    let kb_w = view.keyboard_width();
    let sb_y = rect.min.y + rect.height() - crate::widgets::scrollbar::SCROLLBAR_H;
    let sb_rect = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + kb_w, sb_y),
        egui::pos2(
            content_right_x,
            sb_y + crate::widgets::scrollbar::SCROLLBAR_H,
        ),
    );

    // 水平滚动条：thumb 拖 = 平移（x）+ 垂直位移 → 垂直缩放（key 行高）
    let sb_drag_dy = ui
        .push_id("piano_scrollbar", |ui| {
            crate::widgets::scrollbar::show(
                ui,
                sb_rect,
                w as f32 - kb_w,
                &mut view.base.scroll_x,
                &mut view.base.pixels_per_tick,
                total_ticks,
                &mut view.base.dirty,
            )
        })
        .inner;
    if sb_drag_dy != 0.0 {
        let factor = 1.0 + sb_drag_dy * 0.005;
        let anchor_y = sb_rect.center().y - content_rect.min.y;
        view.zoom_around_y(anchor_y, factor, content_rect.height());
        ui.ctx().request_repaint();
    }

    // ── Vertical scrollbar ──
    // PR 像素空间：num_cells = 128，cell_size = key_height。
    // 滚动条范围 = PR 内容区 [content_y, content_y + content_h]，不超过 PR/AM 分割线。
    // 相对缩放：最小 = 128 键一屏（cell_min），最大 = 12 键一屏（cell_max），随窗口变化。
    {
        let vsb_rect = egui::Rect::from_min_max(
            egui::pos2(content_right_x, content_y),
            egui::pos2(rect.max.x, content_y + content_h),
        );
        let cell_min = content_rect.height() / 128.0;
        let cell_max = content_rect.height() / 12.0;
        // 垂直滚动条：thumb 拖 = 平移（y）+ 水平位移 → 水平缩放（tick 宽度）
        let vsb_drag_dx = ui
            .push_id("piano_vscroll", |ui| {
                crate::widgets::scrollbar::show_vertical(
                    ui,
                    vsb_rect,
                    content_rect.height(),
                    &mut view.base.scroll_y,
                    &mut view.key_height,
                    128,
                    cell_min,
                    cell_max,
                    &mut view.base.dirty,
                )
            })
            .inner;
        if vsb_drag_dx != 0.0 {
            let factor = 1.0 + vsb_drag_dx * 0.005;
            let anchor_x = vsb_rect.center().x - content_rect.min.x;
            view.zoom_around_x(anchor_x, factor);
            ui.ctx().request_repaint();
        }

        // ── 滚动条滚轮缩放：垂直滚动条上滚轮 = 水平缩放；水平滚动条上滚轮 = 垂直缩放 ──
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            let scroll_y = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_y.abs() > 0.5 {
                let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
                if vsb_rect.contains(pos) {
                    // 垂直滚动条 → 水平缩放（锚定滚动条中心 x）
                    let anchor_x = vsb_rect.center().x - content_rect.min.x;
                    view.zoom_around_x(anchor_x, factor);
                    ui.ctx().request_repaint();
                } else if sb_rect.contains(pos) {
                    // 水平滚动条 → 垂直缩放（锚定滚动条中心 y）
                    let anchor_y = sb_rect.center().y - content_rect.min.y;
                    view.zoom_around_y(anchor_y, factor, content_rect.height());
                    ui.ctx().request_repaint();
                }
            }
        }
    }

    // ── Perf probe: submit per-frame sample ──
    perf::submit(perf::PerfCtx {
        t_show_start,
        t_input_end,
        t_prepare_end,
        t_paint_end,
        follow_mode,
        midi,
        view,
        width: w as f32,
    });

    // ── PR quantize button in the top-left corner (left of ruler, above keyboard) ──
    let pr_quantize_event = quantize_button::show(
        ui,
        quantize_button::QuantizeBtnCtx {
            rect_min_x: rect.min.x,
            ruler_band_y,
            kb_w,
            ppq,
            quantize,
        },
    );

    // ── 状态栏讲解行：钢琴卷帘悬停提示（位置 + 音高）──
    // 自动化面板的提示（grid_area 内）优先于 PR 内容区提示；
    // 鼠标在视图内但不在任何可讲解区域（标尺/滚动条/面板空白）→ 清空。
    if let Some(pos) = ui.input(|i| i.pointer.hover_pos())
        && rect.contains(pos)
    {
        let hint = if let Some(h) = panels_status_hint {
            Some(h)
        } else if music_rect.contains(pos) {
            let local = egui::pos2(pos.x - content_rect.min.x, pos.y - content_rect.min.y);
            let tick = view.x_to_tick(local.x).max(0.0);
            let key = view.y_to_key(local.y);
            // 本视图有选框 → 讲解行显示选框统计（参考 info panel）
            let sel_text = if !sel_rect.effective_rects().is_empty()
                && let Some(sh) = sel_hint
            {
                Some(t!("hint.sel_notes", n = sh.count, span = &sh.span).to_string())
            } else {
                None
            };
            if let Some(s) = sel_text {
                Some(s)
            } else {
                let pos_str = match bar_line_data {
                    Some((ppq, num, den, events)) => {
                        format_tick_bar_beat_with_time_sig(tick, ppq, events, num, den)
                    }
                    None => format!("{}", tick as u32),
                };
                Some(format!("{} {}", pos_str, key))
            }
        } else if content_rect.contains(pos) {
            // 键盘列：只显示音高数字
            let local_y = pos.y - content_rect.min.y;
            Some(format!("{}", view.y_to_key(local_y)))
        } else {
            None
        };
        *feedback.status_hint = hint;
    }

    sel_action
        .map(PianoViewEvent::SelectionAction)
        .or(pencil_event)
        .or(eraser_event)
        .or(pr_quantize_event)
}
