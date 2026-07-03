use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::InstanceRenderer;
use crate::instances;
use crate::vertex::{Uniforms, TrackColorsUniform, SelectionUniform, MAX_TRACKS, MAX_SEL_RECTS};
use crate::view::PianoRollView;

use yinhe_wgpu::layer_cache_key;

/// Prepare the pianoroll for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + black-key rows)
///   1 = grid lines
///   2 = notes
///   3 = keyboard
///   4 = pencil ghost note (optional, rebuilt every frame)
///
/// Each layer is cached independently so that playback (scroll_x changes)
/// only invalidates the grid layer.
pub fn prepare(
    renderer: &mut InstanceRenderer,
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
    midi_version: u64,
    ghost_notes: &[(f64, f64, u8, [f32; 3])], // (start_tick, end_tick, key, color) for pencil preview
    note_selection_highlight: bool,
) -> yinhe_wgpu::PrepareTimings {
    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let ppu = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;
    let (scroll_x_pos, scroll_frac) = match scroll_mode {
        0 => (scroll_x, 0.0),
        _ => {
            let f = scroll_x.floor();
            (f, scroll_x - f)
        },
    };

    // Build track colors uniform
    let track_count = track_colors.len().min(MAX_TRACKS) as u32;
    let mut tc_uniform = TrackColorsUniform { colors: [[0.0; 4]; MAX_TRACKS] };
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
        note_selection_highlight: if note_selection_highlight { 1 } else { 0 },
    };

    let t = std::time::Instant::now();
    let theme = renderer.theme.clone();
    renderer.upload_uniforms(uniforms);
    renderer.upload_track_colors(&tc_uniform);
    renderer.upload_selection(&sel_uniform);
    renderer.ensure_layers(5);

    // Layer 0: decor (background + black-key rows)
    let vh = view.render_hash();
    let wh = yinhe_wgpu::hash_f32s(&[w, h]);
    let decor_key = layer_cache_key(&[vh, wh]);
    renderer.upload_layer(0, decor_key, |out| {
        instances::build_decor(out, w, h, kb_w, kh, scroll_y, &theme);
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[vh, wh]);
    if let Some(midi) = midi {
        let sig_events = midi.time_sig_events();
        let mut sig_hash = 0u64;
        for ev in sig_events {
            sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.tick as u64);
            sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.numerator as u64);
            sig_hash = sig_hash.wrapping_mul(31).wrapping_add(ev.denominator as u64);
        }
        grid_key = layer_cache_key(&[grid_key, sig_hash]);
    }
    renderer.upload_layer(1, grid_key, |out| {
        if let Some(midi) = midi
            && let Some(tpb) = midi.ticks_per_beat()
        {
            let (def_num, def_den) = midi.time_sig_default();
            let sig_events = midi.time_sig_events();
            instances::build_grid(out, w, h, view, tpb, def_num, def_den, sig_events, scroll_x_pos, &theme);
        }
    });

    // Layer 2: notes
    // Notes layer no longer depends on selection or track_colors (handled in shader)
    // Only depends on: viewport, track_visible, hidden_notes, midi_version
    // 顺序哈希：位置敏感，[true,false] ≠ [false,true]
    // 之前用 XOR 导致只算奇偶校验位，切换轨道时哈希不变，缓存不失效
    let tv_hash = yinhe_wgpu::hash_bools(track_visible);
    let hidden_hash = hidden_notes.iter().fold(0u64, |acc, &(trk, tick, key)| {
        let mut h: u64 = 0;
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(trk as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(tick as u64);
        h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(key as u64);
        acc ^ h
    });
    let notes_key = layer_cache_key(&[vh, wh, tv_hash, midi_version, hidden_hash]);
    renderer.upload_layer(2, notes_key, |out| {
        if let Some(midi) = midi {
            instances::build_notes(out, w, h, midi, view, selected, hidden_notes, track_visible, track_colors, &theme);
        }
    });

    // Layer 3: keyboard
    let kb_key = layer_cache_key(&[vh, wh]);
    renderer.upload_layer(3, kb_key, |out| {
        instances::build_keyboard(out, kb_w, kh, scroll_y, h, &theme);
    });

    // Layer 4: ghost notes (no cache — rebuilt every frame)
    if !ghost_notes.is_empty() {
        renderer.upload_layer_force(4, |out| {
            for &(start_tick, end_tick, key, color) in ghost_notes {
                instances::build_ghost_note(out, start_tick, end_tick, key, color, &theme);
            }
        });
    } else {
        // Clear the ghost layer when there's no preview
        renderer.upload_layer_force(4, |_| {});
    }

    let dur = t.elapsed();

    yinhe_wgpu::PrepareTimings {
        build_static: dur,
        instance_count: renderer.total_layer_instances(),
    }
}
