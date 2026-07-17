use super::instances;
use crate::vertex::{Uniforms, TrackColorsUniform, SelectionUniform, MAX_TRACKS, MAX_SEL_RECTS};
use yinhe_types::PianoRollView;

use crate::layer_cache_key;
use crate::DecorLayerData;

/// Render job data: uniforms + decor layers built on CPU, sent to GPU for upload.
/// Note layers are NOT included — the GPU compute cull path handles notes,
/// and ghost notes are built separately by the caller.
pub struct PianorollRenderJob {
    pub width: u32,
    pub height: u32,
    pub uniforms: Uniforms,
    pub track_colors: Box<TrackColorsUniform>,
    pub selection: SelectionUniform,
    pub decor_layers: Vec<DecorLayerData>,
    pub build_time: std::time::Duration,
}

/// Build a `PianorollRenderJob` containing decor instance data and uniforms.
///
/// This does the CPU-heavy work (building instances via rayon) without
/// touching any GPU resources. The result can be sent to a render thread
/// for async upload + draw + submit. Ghost notes are NOT included —
/// they are a transient overlay built and uploaded separately.
pub fn build_render_job(
    width: u32,
    height: u32,
    midi: Option<&dyn yinhe_types::NoteSource>,
    view: &PianoRollView,
    selected: &yinhe_core::Selection,
    track_colors: &[[f32; 3]],
    scroll_mode: u32,
    min_border_width: f32,
    note_outline: bool,
    theme: &crate::GpuTheme,
) -> PianorollRenderJob {
    let t = std::time::Instant::now();

    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let ppu = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = crate::compute_scroll_frac(scroll_x, scroll_mode);

    // Build track colors uniform — allocate on heap to avoid 1MB stack overflow
    let track_count = track_colors.len().min(MAX_TRACKS) as u32;
    let mut tc_buf: Vec<u8> = vec![0u8; std::mem::size_of::<TrackColorsUniform>()];
    let tc_uniform: &mut TrackColorsUniform = bytemuck::from_bytes_mut(&mut tc_buf);
    for (i, color) in track_colors.iter().enumerate().take(MAX_TRACKS) {
        tc_uniform.colors[i] = [color[0], color[1], color[2], 1.0];
    }

    // Build selection rects uniform
    let sel_rect_count = selected.rects.len().min(MAX_SEL_RECTS) as u32;
    let mut sel_uniform = SelectionUniform { rects: [[0; 4]; MAX_SEL_RECTS * 2] };
    for (i, rect) in selected.rects.iter().enumerate().take(MAX_SEL_RECTS) {
        let (tick_start, tick_end, key_lo, key_hi, track_lo, track_hi) = *rect;
        sel_uniform.rects[i * 2] = [tick_start, tick_end, key_lo as u32, key_hi as u32];
        sel_uniform.rects[i * 2 + 1] = [track_lo as u32, track_hi as u32, 0, 0];
    }

    let uniforms = Uniforms {
        width: w,
        height: h,
        scroll_x: scroll_x_pos,
        scroll_y,
        pixels_per_tick: ppu,
        key_height: kh,
        keyboard_width: kb_w,
        mode: 1, // PR notes: tick→pixel + compute rounding in shader
        scroll_frac,
        scroll_mode,
        min_border_width,
        track_count,
        sel_rect_count,
        note_outline: if note_outline { 1 } else { 0 },
        lane_height: 0.0, // PR unused (shader uses key_height)
        note_alpha: 1.0,  // PR notes fully opaque
    };

    // Layer 0: decor (background + black-key rows)
    let vh = view.render_hash();
    let wh = crate::hash_f32s(&[w, h]);
    let decor_key = layer_cache_key(&[vh, wh]);
    let mut decor_0 = Vec::new();
    instances::build_decor(&mut decor_0, w, h, kb_w, kh, scroll_y, theme);

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
    if let Some(midi) = midi {
        let sig_events = midi.time_sig_events();
        let sig_hash = crate::hash_time_sigs(sig_events);
        grid_key = layer_cache_key(&[vh, wh, sig_hash]);
    }
    let mut decor_1 = Vec::new();
    if let Some(midi) = midi
        && let Some(tpb) = midi.ticks_per_beat()
    {
        let (def_num, def_den) = midi.time_sig_default();
        let sig_events = midi.time_sig_events();
        instances::build_grid(&mut decor_1, w, h, view, tpb, def_num, def_den, sig_events, scroll_x_pos, theme);
    }

    let build_time = t.elapsed();

    PianorollRenderJob {
        width,
        height,
        uniforms,
        track_colors: Box::new(*tc_uniform),
        selection: sel_uniform,
        decor_layers: vec![
            DecorLayerData { instances: decor_0, cache_key: decor_key },
            DecorLayerData { instances: decor_1, cache_key: grid_key },
        ],
        build_time,
    }
}
