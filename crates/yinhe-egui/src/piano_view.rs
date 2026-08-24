use eframe::egui;

use yinhe_types::{AutomationLane, TimeSigEvent};

use crate::widgets::tools_panel::Tool;
pub use yinhe_editor_core::ResizeSide;
use yinhe_editor_core::audio_settings::QuickDeleteMode;
use yinhe_editor_core::quantize::QuantizePreset;
pub use yinhe_types::PencilNoteDrag;

pub mod automation_panel;
mod bg;
pub mod control_bar;
pub(crate) mod drag;
mod follow;
mod gpu;
pub(crate) mod gpu_upload;
mod interaction;
mod keyboard;
mod layout;
mod marquee;
mod overlay;
mod panels;
mod pencil;
mod perf;
mod scrollbar;
mod status;
mod tool;
mod types;
pub(crate) use follow::update_follow;
pub(crate) use layout::{Layout, compute_layout};
#[allow(unused_imports)]
pub(crate) use tool::effective_tool;
pub use types::*;
pub(crate) use types::{NotePreview, PreviewReq};

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
    last_hidden_keys: &mut crate::piano_view::gpu_upload::HiddenKeyMask, // last hidden_notes key 位图（hidden 增量重建判定）
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
    auto_ctx: Option<AutomationPanelsCtx<'_>>,
    scroll_mode: u32,
    min_border_width: f32,
    content_opacity: f32,
    note_outline: bool,
    use_gpu_cull: bool,
    tempo_lane: &AutomationLane,
    sel_rect: &mut yinhe_editor_core::edit_state::SelRectState,
    track_selected: &std::collections::HashSet<u16>,
    conductor_idx: Option<u16>,
    // 写入目标轨 = 主音轨或回退轨（无选中时第一个非 Conductor 轨），由 layout 计算。
    write_track: Option<u16>,
    // 控制栏输入（最顶部一栏：量化/音轨名称/和弦指示器；横向时在标尺上方）。
    bar: control_bar::PrBarData<'_>,
    revision: u64,
    note_revisions: &[u64; yinhe_types::KEY_COUNT],
    feedback: &mut PianoViewFeedback<'_>,
    sel_hint: Option<&crate::app::layout::SelHintInfo>,
    quick_delete_mode: QuickDeleteMode,
    default_gate: Option<u32>,
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

    let total_ticks = super::view_interaction::total_ticks_padded(
        midi.and_then(|m| m.tick_length()).unwrap_or(0),
        ppq,
    );
    let layout: Layout = compute_layout(
        view,
        rect,
        panels_natural_h,
        ui.ctx().pixels_per_point(),
        total_ticks,
    )?;
    let content_rect = layout.content_rect;
    let music_rect = layout.music_rect;
    let content_y = layout.content_y;
    let content_bottom = layout.content_bottom;
    let w = layout.w;
    let h = layout.h;
    let pw = layout.pw;
    let ph = layout.ph;
    let total_ticks = layout.total_ticks;
    let panels_total_h = layout.panels_total_h;
    // 兼容旧局部变量命名，保持后续代码无需大改。
    let kb_w = view.keyboard_width();
    let content_right_x = rect.max.x - crate::widgets::scrollbar::SCROLLBAR_W;
    let content_h = (content_bottom - content_y).max(0.0);
    let _ = content_h;
    let _ruler_band_y = rect.min.y;

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
        // 目标视口用纹理实际尺寸（超 GPU 上限时被裁剪，视口必须一致）
        let (aw, ah) = render_ctx.actual_size();
        rt.update_target(render_ctx.preview_view().clone(), aw, ah);
    }
    update_follow(view, *cursor_tick, is_playing, follow_mode, ui, &layout);

    let interaction::InteractionOutput {
        effective_tool,
        ghost_notes,
        hidden_notes,
        pencil_event,
        eraser_event,
        quick_delete_event,
    } = interaction::dispatch(
        ui,
        view,
        rect,
        content_rect,
        music_rect,
        midi,
        selected,
        track_visible,
        track_colors,
        cursor_tick,
        quantize,
        ppq,
        bar_line_data,
        total_ticks,
        sel_rect,
        track_selected,
        write_track,
        conductor_idx,
        active_tool,
        quick_delete_mode,
        feedback,
        default_gate,
    );

    interaction::update_hover_cursor(
        ui,
        view,
        content_rect,
        music_rect,
        midi,
        track_visible,
        track_selected,
        sel_rect,
        write_track,
        conductor_idx,
        effective_tool,
    );

    // ── Content interaction (zoom/pan/cursor/drag/reset) ──
    let left_zone = if view.is_vertical() { 0.0 } else { kb_w };
    crate::view_interaction::handle_input(
        ui,
        content_rect,
        view,
        cursor_tick,
        left_zone,
        Some((quantize, ppq)),
        bar_line_data,
        None,
        None,
        is_playing,
        follow_mode,
        active_tool,
    );

    interaction::handle_kb_resize(ui, view, rect, content_rect, w, h);

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

    let mut t_prepare_end = None;
    let (cull_ready, theme) = gpu::upload_and_prepare(
        pianoroll,
        render_ctx,
        render_thread,
        view,
        midi,
        midi_arc,
        &*selected,
        track_visible,
        &hidden_notes,
        track_colors,
        revision,
        note_revisions,
        last_cull_revision,
        last_cull_revision_only,
        last_hidden_hash,
        last_tv_hash,
        last_hidden_keys,
        cull_rebuild,
        &ghost_notes,
        w,
        h,
        scroll_mode,
        min_border_width,
        note_outline,
        use_gpu_cull,
        perf_on,
        &mut t_prepare_end,
    );

    // Static cache was removed — every frame rebuilds + uploads, so always paint.
    view.base.dirty = false;

    let kh = view.key_height;
    let kb_w = view.keyboard_width();
    let tpb = midi.and_then(|m| m.ticks_per_beat());
    let grid_rect = if view.is_vertical() {
        content_rect
    } else {
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x + kb_w, content_rect.min.y),
            content_rect.max,
        )
    };
    let keyboard_rect = if view.is_vertical() {
        let kb_bottom = rect.max.y - crate::widgets::scrollbar::SCROLLBAR_H;
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, kb_bottom - kb_w),
            egui::pos2(content_right_x, kb_bottom),
        )
    } else {
        egui::Rect::from_min_max(
            egui::pos2(content_rect.min.x, content_rect.min.y),
            egui::pos2(content_rect.min.x + kb_w, content_rect.max.y),
        )
    };
    let sel_action = overlay::draw_overlays(
        ui,
        &painter,
        content_rect,
        music_rect,
        rect,
        view,
        &theme,
        kh,
        kb_w,
        key_sig_events,
        content_opacity,
        midi,
        tpb,
        grid_rect,
        cull_ready,
        render_ctx,
        pianoroll,
        pw,
        ph,
        keyboard_rect,
        cursor_tick,
        effective_tool,
        sel_rect,
        quantize,
        ppq,
        bar_line_data,
        &bar,
        feedback,
        selected,
    );
    let t_paint_end = if perf_on {
        Some(std::time::Instant::now())
    } else {
        None
    };

    // ── Automation panels ──
    let panels_y = content_rect.max.y;
    let panels_status_hint = panels::show_panels(
        ui,
        view,
        rect,
        content_rect,
        panels_y,
        panels_total_h,
        auto_ctx,
        midi,
        track_visible,
        track_colors,
        tempo_lane,
        sel_rect,
        track_selected,
        active_tool,
        write_track,
        conductor_idx,
        quantize,
        ppq,
        bar_line_data,
        sel_hint,
        scroll_mode,
        min_border_width,
        revision,
        feedback,
        &theme,
        selected,
    );

    let pr_orientation = view.orientation();
    scrollbar::show_scrollbars(
        ui,
        view,
        rect,
        content_rect,
        content_bottom,
        content_y,
        w,
        h,
        total_ticks,
        pr_orientation,
    );

    // ── Perf probe + 状态栏讲解行 ──
    status::update_status(
        ui,
        rect,
        music_rect,
        content_rect,
        view,
        midi,
        follow_mode,
        t_show_start,
        t_input_end,
        t_prepare_end,
        t_paint_end,
        w as f32,
        panels_status_hint,
        sel_hint,
        sel_rect,
        bar_line_data,
        feedback,
    );

    sel_action
        .map(PianoViewEvent::SelectionAction)
        .or(quick_delete_event)
        .or(pencil_event)
        .or(eraser_event)
}
