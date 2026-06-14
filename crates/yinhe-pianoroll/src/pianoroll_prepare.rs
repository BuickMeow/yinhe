use std::collections::HashSet;

use yinhe_types::NoteSource;

use crate::PianorollRenderer;
use crate::instances;
use crate::vertex::Uniforms;
use crate::view::PianoRollView;

use yinhe_wgpu::layer_cache_key;

/// Prepare the pianoroll for rendering using the layered cache API.
///
/// Layers:
///   0 = decor (background + black-key rows)
///   1 = grid lines
///   2 = notes
///   3 = keyboard
///
/// Each layer is cached independently so that playback (scroll_x changes)
/// only invalidates the grid layer.
pub fn prepare(
    renderer: &mut PianorollRenderer,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &PianoRollView,
    selected: &HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) -> yinhe_wgpu::PrepareTimings {
    let w = width as f32;
    let h = height as f32;
    let kb_w = view.keyboard_width();
    let kh = view.key_height;
    let scroll_y = view.base.scroll_y;
    let ppu = view.base.pixels_per_tick;
    let scroll_x = view.base.scroll_x;

    let uniforms = Uniforms {
        width: w,
        height: h,
        scroll_x,
        scroll_y,
        pixels_per_tick: ppu,
        key_height: kh,
        keyboard_width: kb_w,
        _pad: 0.0,
    };

    let t = std::time::Instant::now();
    renderer.upload_uniforms(uniforms);
    renderer.ensure_layers(4);

    // Layer 0: decor (background + black-key rows)
    let decor_key = layer_cache_key(&[
        scroll_y.to_bits() as u64,
        kh.to_bits() as u64,
        h.to_bits() as u64,
    ]);
    renderer.upload_layer(0, decor_key, |out| {
        instances::build_decor(out, w, h, kb_w, kh, scroll_y);
    });

    // Layer 1: grid lines
    let mut grid_key = layer_cache_key(&[
        scroll_x.to_bits() as u64,
        ppu.to_bits() as u64,
    ]);
    if let Some(midi) = midi {
        let sig_events = midi.time_sig_events();
        // Hash the time sig events into the key
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
            instances::build_grid(out, w, h, view, tpb, def_num, def_den, sig_events);
        }
    });

    // Layer 2: notes
    let sel_hash = {
        let mut h = 0u64;
        for &(trk, tick, key) in selected.iter() {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(trk as u64);
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(tick as u64);
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(key as u64);
        }
        h
    };
    let tv_hash = {
        let mut h = 0u64;
        for &v in track_visible {
            h = h.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(v as u64);
        }
        h
    };
    let notes_key = layer_cache_key(&[
        scroll_x.to_bits() as u64,
        scroll_y.to_bits() as u64,
        ppu.to_bits() as u64,
        kh.to_bits() as u64,
        sel_hash,
        tv_hash,
    ]);
    renderer.upload_layer(2, notes_key, |out| {
        if let Some(midi) = midi {
            instances::build_notes(out, w, h, midi, view, selected, track_visible, track_colors);
        }
    });

    // Layer 3: keyboard
    let kb_key = layer_cache_key(&[
        scroll_y.to_bits() as u64,
        kh.to_bits() as u64,
        h.to_bits() as u64,
    ]);
    renderer.upload_layer(3, kb_key, |out| {
        instances::build_keyboard(out, kb_w, kh, scroll_y, h);
    });

    let dur = t.elapsed();

    yinhe_wgpu::PrepareTimings {
        dirty: true,
        static_rebuilt: true,
        build_static: dur,
        build_cursor: std::time::Duration::ZERO,
        upload: std::time::Duration::ZERO,
        instance_count: renderer.total_layer_instances(),
    }
}
