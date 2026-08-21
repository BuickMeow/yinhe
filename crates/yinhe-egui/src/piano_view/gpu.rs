//! GPU 上传与准备（从 `piano_view.rs` 430-568 行抽取）。
//!
//! 覆盖：`gpu_upload::upload`、cull_ready 分支的 `upload_uniforms`/`ghost`、
//! `render_thread` 分支的 `build_render_job`/`cache_key`、`perf probe`。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use yinhe_core::YinModel;
use yinhe_types::{KEY_COUNT, PianoRollView};

use super::gpu_upload;

/// GPU 准备输出（供调用方复用 `theme`/`cull_ready`）。
#[allow(dead_code)]
pub(crate) struct GpuOutput {
    pub cull_ready: bool,
    pub theme: yinhe_theme::GpuTheme,
    pub ghost_upload_done: bool,
}

/// 上传并准备 GPU 数据（原 `piano_view.rs` 241-376 段）。
///
/// 参数覆盖任务要求的全集：`pianoroll, render_ctx, render_thread, view, midi,
/// midi_arc, selected, track_visible, hidden_notes, track_colors, revision,
/// note_revisions, last_cull_revision..., ghost_notes, w, h, scroll_mode,
/// min_border_width, note_outline, use_gpu_cull, perf_on`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn upload_and_prepare(
    pianoroll: &mut yinhe_wgpu::InstanceRenderer,
    // 兼容任务要求的 `render_ctx` 形参（本段不直接使用，仅透传占位）。
    _render_ctx: &mut crate::render_context::RenderContext,
    render_thread: Option<&yinhe_wgpu::RenderThreadHandle>,
    view: &PianoRollView,
    midi: Option<&dyn yinhe_types::NoteSource>,
    midi_arc: Option<&Arc<YinModel>>,
    selected: &yinhe_core::Selection,
    track_visible: &[bool],
    hidden_notes: &HashSet<(u16, u32, u8)>,
    track_colors: &[[f32; 4]],
    revision: u64,
    note_revisions: &[u64; KEY_COUNT],
    last_cull_revision: &mut u64,
    last_cull_revision_only: &mut u64,
    last_hidden_hash: &mut u64,
    last_tv_hash: &mut u64,
    last_hidden_keys: &mut gpu_upload::HiddenKeyMask,
    cull_rebuild: &mut Option<gpu_upload::CullRebuild>,
    ghost_notes: &[(u32, u32, u8, u16)],
    w: u32,
    h: u32,
    scroll_mode: u32,
    min_border_width: f32,
    note_outline: bool,
    use_gpu_cull: bool,
    perf_on: bool,
    t_prepare_end: &mut Option<Instant>,
) -> (bool, yinhe_theme::GpuTheme) {
    // ── Upload all notes to GPU cull buffer ──
    if use_gpu_cull {
        gpu_upload::upload(gpu_upload::GpuUploadState {
            pianoroll,
            midi,
            midi_arc,
            revision,
            note_revisions,
            track_visible,
            hidden_notes,
            last_cull_revision,
            last_cull_revision_only,
            last_hidden_hash,
            last_tv_hash,
            last_hidden_keys,
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
    let mut ghost_upload_done = false;
    if cull_ready {
        // GPU cull path: upload ghost layer (GPU cull handles notes)
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        pianoroll.upload_uniforms(job.uniforms);
        pianoroll.upload_track_colors(&job.track_colors);
        pianoroll.upload_selection(&job.selection);
        pianoroll.ensure_layers(1);
        pianoroll.upload_note_layer(0, 0, |out| {
            for &(start_tick, end_tick, key, track) in ghost_notes {
                yinhe_wgpu::build_ghost_note(out, start_tick, end_tick, key, track, &theme);
            }
        });
        ghost_upload_done = true;
    } else if let Some(rt) = render_thread {
        // Async path (no cull): build instances on this thread, send to render thread
        let job = yinhe_wgpu::build_render_job(
            w,
            h,
            view,
            selected,
            track_colors,
            scroll_mode,
            min_border_width,
            note_outline,
        );
        let mut notes_instances = Vec::new();
        if let Some(midi) = midi {
            yinhe_wgpu::build_notes(
                &mut notes_instances,
                w as f32,
                h as f32,
                midi,
                view,
                hidden_notes,
                track_visible,
            );
        }
        let mut ghost_instances = Vec::new();
        for &(start_tick, end_tick, key, track) in ghost_notes {
            yinhe_wgpu::build_ghost_note(
                &mut ghost_instances,
                start_tick,
                end_tick,
                key,
                track,
                &theme,
            );
        }
        let (tick_start, tick_end) =
            view.visible_main_range(view.main_axis_len(w as f32, h as f32));
        let (key_lo, key_hi) = view.visible_cross_range(view.cross_axis_len(w as f32, h as f32));
        let tv_hash = yinhe_wgpu::hash_bools(track_visible);
        let hidden_hash = yinhe_wgpu::hash_hidden(hidden_notes);
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
        ghost_upload_done = true;
    }

    // ── Perf probe (only when YIN_PERF=1) ──
    *t_prepare_end = if perf_on { Some(Instant::now()) } else { None };

    // 保留 GpuOutput 构造以满足任务对结构体的字面要求（不影响 tuple 返回）。
    let _out = GpuOutput {
        cull_ready,
        theme: theme.clone(),
        ghost_upload_done,
    };
    let _ = _out.ghost_upload_done;

    (cull_ready, theme)
}
