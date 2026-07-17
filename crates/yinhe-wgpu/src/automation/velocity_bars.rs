use yinhe_types::AutomationPanelView;
use rayon::prelude::*;
use yinhe_theme::GpuTheme;
use yinhe_types::{key_notes_in_range, NoteSource, TRACK_PALETTE};
use crate::vertex::{DrawInstance, pack_props, pack_rgba};

/// Stack red zone threshold for stacker.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size for stacker.
const STACK_SIZE: usize = 1024 * 1024;

/// Build velocity bars from NoteSource (layer 2, replaces data bars for Velocity).
///
/// `display_mode`: 0=柱状(2px竖条), 1=矩形(填充), 2=空心矩形(边框)
///
/// Uses `stacker::maybe_grow` to prevent stack overflow when processing
/// many notes at very low zoom levels.
pub fn build_velocity_bars(
    out: &mut Vec<DrawInstance>,
    w: f32,
    _h: f32,
    midi: &dyn NoteSource,
    view: &AutomationPanelView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    display_mode: u32,
    _theme: &GpuTheme,
) {
    let ppu = view.base.pixels_per_tick;
    let (tick_start, tick_end) = view.base.visible_tick_range(w);
    let pad_start = tick_start.max(0.0) as u32;
    let pad_end = tick_end.max(0.0) as u32;
    let x_offset = view.base.left_panel_width - view.base.scroll_x;

    struct VelBar {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        color: [f32; 3],
        vel: u32,
        duration: u32,
        start_tick: u32,
    }

    let mut bars: Vec<VelBar> = (0u8..128)
        .into_par_iter()
        .flat_map_iter(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let mut local_bars: Vec<VelBar> = Vec::new();
                let notes = key_notes_in_range(midi.key_notes(key), pad_start, pad_end);
                for note in notes {
                    if note.start_tick as f64 > pad_end as f64 {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start as f64 {
                        continue;
                    }
                    let trk_idx = note.track as usize;
                    if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                        continue;
                    }

                    let color = track_colors
                        .get(trk_idx)
                        .copied()
                        .unwrap_or_else(|| TRACK_PALETTE[trk_idx % TRACK_PALETTE.len()]);

                    let y = view.value_to_y(note.velocity as f32, 127.0);
                    let vel_h = view.value_to_y(0.0, 127.0) - y;

                    match display_mode {
                        0 => {
                            let bar_x = x_offset + note.start_tick as f32 * ppu;
                            if bar_x + 2.0 < 0.0 || bar_x > w {
                                continue;
                            }
                            local_bars.push(VelBar {
                                x: bar_x,
                                y,
                                w: 2.0,
                                h: vel_h,
                                color,
                                vel: note.velocity as u32,
                                duration: note.end_tick - note.start_tick,
                                start_tick: note.start_tick,
                            });
                        }
                        _ => {
                            let raw_x = x_offset + note.start_tick as f32 * ppu;
                            let raw_end = x_offset + note.end_tick as f32 * ppu;
                            let nx = raw_x;
                            let nw = (raw_end - raw_x).max(2.0);
                            if nx + nw < 0.0 || nx > w {
                                continue;
                            }
                            local_bars.push(VelBar {
                                x: nx,
                                y,
                                w: nw,
                                h: vel_h,
                                color,
                                vel: note.velocity as u32,
                                duration: note.end_tick - note.start_tick,
                                start_tick: note.start_tick,
                            });
                        }
                    }
                }
                local_bars
            })
        })
        .collect();

    // Sort: shorter notes on top (later in draw order), then later-starting,
    // then softer.  This ensures overlapping bars don't fully hide short notes.
    bars.sort_by(|a, b| {
        a.duration.cmp(&b.duration)
            .then(b.start_tick.cmp(&a.start_tick))
            .then(a.vel.cmp(&b.vel))
    });

    let alpha = if display_mode == 1 { 1.0 } else { 0.85 };
    let border = if display_mode == 2 { 1.0 } else { 0.0 };
    let fill_alpha = if display_mode == 2 { 0.0 } else { alpha };

    for bar in &bars {
        out.push(DrawInstance {
            x: bar.x,
            y: bar.y,
            w: bar.w,
            h: bar.h,
            rgba_packed: pack_rgba(bar.color[0], bar.color[1], bar.color[2], fill_alpha),
            props_packed: pack_props(0.0, border),
            velocity: bar.vel,
            tag: 0,
        });
    }
}
