use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::instances;
use crate::vertex::{Uniforms, TrackColorsUniform, SelectionUniform, MAX_TRACKS, MAX_SEL_RECTS};
use yinhe_types::PianoRollView;

use yinhe_wgpu::layer_cache_key;
use yinhe_wgpu::{DecorLayerData, NoteLayerData};

/// Prepare the pianoroll for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + black-key rows)
///   1 = grid lines
///   2 = notes
///
/// Ghost notes are NOT included — they are a transient overlay handled
/// separately by the caller. Each layer is cached independently so that
/// playback (scroll_x changes) only invalidates the grid layer.
pub fn prepare(
    renderer: &mut crate::InstanceRenderer,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &yinhe_core::Selection,
    hidden_notes: &HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    scroll_mode: u32,
    min_border_width: f32,
    revision: u64,
    note_outline: bool,
) -> yinhe_wgpu::PrepareTimings {
    let job = build_render_job(
        width, height, midi, view, selected, hidden_notes, track_visible,
        track_colors, scroll_mode, min_border_width, revision, note_outline,
        &renderer.theme,
    );

    let t = std::time::Instant::now();
    renderer.upload_uniforms(job.uniforms);
    renderer.upload_track_colors(&job.track_colors);
    renderer.upload_selection(&job.selection);
    renderer.ensure_layers(job.decor_layers.len() + job.note_layers.len());

    let mut layer_idx = 0;
    for dl in &job.decor_layers {
        let cache_key = dl.cache_key;
        let instances = &dl.instances;
        renderer.upload_layer(layer_idx, cache_key, |out| {
            out.extend_from_slice(instances);
        });
        layer_idx += 1;
    }
    for nl in &job.note_layers {
        let cache_key = nl.cache_key;
        let instances = &nl.instances;
        if nl.force {
            renderer.upload_note_layer_force(layer_idx, |out| {
                out.extend_from_slice(instances);
            });
        } else {
            renderer.upload_note_layer(layer_idx, cache_key, |out| {
                out.extend_from_slice(instances);
            });
        }
        layer_idx += 1;
    }

    let dur = t.elapsed();

    yinhe_wgpu::PrepareTimings {
        build_static: job.build_time + dur,
        instance_count: renderer.total_layer_instances(),
    }
}

/// Extended render job that also carries track_colors and selection uniforms
/// (needed by the synchronous `prepare()` path).
pub struct PianorollRenderJob {
    pub width: u32,
    pub height: u32,
    pub uniforms: Uniforms,
    pub track_colors: TrackColorsUniform,
    pub selection: SelectionUniform,
    pub decor_layers: Vec<DecorLayerData>,
    pub note_layers: Vec<NoteLayerData>,
    pub build_time: std::time::Duration,
}

/// Build a `PianorollRenderJob` containing all instance data and uniforms.
///
/// This does the CPU-heavy work (building instances via rayon) without
/// touching any GPU resources. The result can be sent to a render thread
/// for async upload + draw + submit. Ghost notes are NOT included —
/// they are a transient overlay built and uploaded separately.
pub fn build_render_job(
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &yinhe_core::Selection,
    hidden_notes: &HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    scroll_mode: u32,
    min_border_width: f32,
    revision: u64,
    note_outline: bool,
    theme: &yinhe_wgpu::GpuTheme,
) -> PianorollRenderJob {
    let t = std::time::Instant::now();

    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let ppu = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = yinhe_wgpu::compute_scroll_frac(scroll_x, scroll_mode);

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
    let wh = yinhe_wgpu::hash_f32s(&[w, h]);
    let decor_key = layer_cache_key(&[vh, wh]);
    let mut decor_0 = Vec::new();
    instances::build_decor(&mut decor_0, w, h, kb_w, kh, scroll_y, theme);

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
    if let Some(midi) = midi {
        let sig_events = midi.time_sig_events();
        let sig_hash = yinhe_wgpu::hash_time_sigs(sig_events);
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

    // Layer 2: notes
    let tv_hash = yinhe_wgpu::hash_bools(track_visible);
    let hidden_hash = hidden_notes.iter().fold(0u64, |acc, &(trk, tick, key)| {
        let mut h: u64 = 0;
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(trk as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(tick as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(key as u64);
        acc ^ h
    });
    let notes_key = layer_cache_key(&[vh, wh, tv_hash, revision, hidden_hash]);
    let mut notes_2 = Vec::new();
    if let Some(midi) = midi {
        instances::build_notes(&mut notes_2, w, h, midi, view, hidden_notes, track_visible);
    }

    let build_time = t.elapsed();

    PianorollRenderJob {
        width,
        height,
        uniforms,
        track_colors: *tc_uniform,
        selection: sel_uniform,
        decor_layers: vec![
            DecorLayerData { instances: decor_0, cache_key: decor_key },
            DecorLayerData { instances: decor_1, cache_key: grid_key },
        ],
        note_layers: vec![
            NoteLayerData { instances: notes_2, cache_key: notes_key, force: false },
        ],
        build_time,
    }
}
