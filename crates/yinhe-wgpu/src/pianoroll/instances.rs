use rayon::prelude::*;
use yinhe_theme::GpuTheme;
use yinhe_types::{key_notes_in_range, NoteSource, TimeSigEvent};

use crate::grid;
use crate::vertex::{DrawInstance, NoteInstance};
use yinhe_types::PianoRollView;

/// Stack red zone threshold. When stack usage exceeds this, `stacker` will
/// allocate a new stack segment before calling the closure.
/// 32KB should be enough for a single key's note iteration.
const STACK_RED_ZONE: usize = 32 * 1024;
/// New stack segment size to allocate when the red zone is exceeded.
const STACK_SIZE: usize = 1024 * 1024; // 1MB per segment

/// Build grid line instances (layer 0).
/// Dependencies: scroll_x, pixels_per_tick, time_sig
pub fn build_grid(
    out: &mut Vec<DrawInstance>,
    w: f32,
    h: f32,
    view: &PianoRollView,
    tpb: u32,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    scroll_x_pixel: f32,
    theme: &GpuTheme,
) {
    grid::build_timeline_grid(
        out,
        w,
        h,
        &view.base,
        tpb,
        default_num,
        default_den,
        time_sig_events,
        theme.pr_measure_line,
        theme.pr_beat_line,
        Some(theme.pr_sub_beat_line),
        Some(theme.pr_tick_line),
        scroll_x_pixel,
    );
}

/// Build note instances (layer 2).
/// Dependencies: selection, track_visible, tick range (scroll_x)
///
/// Output is 16B `NoteInstance` (semantic data only: ticks, key, track, vel).
/// All pixel positions and colors are computed in the GPU vertex shader from
/// uniforms, so scroll_y and key_height changes do NOT invalidate the cache.
///
/// Padding: one screen width on each side of the visible tick range,
/// so fast scrolling doesn't flash empty space before the cache rebuilds.
///
/// Uses `stacker::maybe_grow` to dynamically allocate new stack segments
/// when the current stack is close to overflowing. This prevents
/// STATUS_STACK_BUFFER_OVERRUN on Windows when rendering millions of notes
/// at very low zoom levels.
pub fn build_notes(
    out: &mut Vec<NoteInstance>,
    w: f32,
    h: f32,
    midi: &dyn NoteSource,
    view: &PianoRollView,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
) {
    let (tick_start, tick_end) = view.visible_tick_range(w);
    let (key_lo, key_hi) = view.visible_key_range(h);
    // Only build notes whose visible interval overlaps the current viewport.
    // key_notes_in_range looks back via the max_end index, so any note that
    // starts off-screen-left but extends into view is still included — no
    // padding required, regardless of note length.
    let range_start = tick_start.max(0.0);
    let range_end = tick_end;

    // Use stacker to protect against stack overflow when processing many notes.
    // Each parallel iteration runs in a separate stack segment if needed.
    let results: Vec<Vec<NoteInstance>> = (key_lo..=key_hi)
        .into_par_iter()
        .filter_map(|key| {
            // Wrap key processing in stacker to get fresh stack segments on demand.
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = key_notes_in_range(midi.key_notes(key), range_start as u32, range_end as u32);
                if notes.is_empty() {
                    return None;
                }

                let mut local = Vec::new();

                for note in notes {
                    if note.start_tick as f64 > range_end {
                        break;
                    }
                    if (note.end_tick as f64) < range_start {
                        continue;
                    }
                    if !track_visible
                        .get(note.track as usize)
                        .copied()
                        .unwrap_or(true)
                    {
                        continue;
                    }

                    // Skip hidden notes (being dragged)
                    if hidden_notes.contains(&(note.track, note.start_tick, key)) {
                        continue;
                    }

                    // 16B NoteInstance: shader fetches color from track_colors
                    // storage buffer via track index, and computes pixel positions
                    // from uniforms. track is u16 (0..65535).
                    local.push(NoteInstance {
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        packed: NoteInstance::pack(key, note.track, note.velocity),
                        reserved: 0,
                    });
                }

                if local.is_empty() { None } else { Some(local) }
            })
        })
        .collect();

    out.extend(results.into_iter().flatten());
}

/// Build ALL note instances (no viewport culling) for GPU compute cull.
/// Upload once on MIDI load/change; the GPU cull shader handles per-frame
/// viewport culling.
///
/// Filters by `track_visible` and `hidden_notes` on the CPU so the GPU
/// buffer doesn't contain invisible notes (saving memory and draw calls).
/// When `track_visible`/`hidden_notes` change, call this again to re-upload.
///
/// Returns `(notes, per_key_offsets)` where `per_key_offsets[k]` is the
/// start index of key k's notes in the flat buffer, and `per_key_offsets[128]`
/// is the total count.
pub fn build_all_notes(
    midi: &dyn NoteSource,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
) -> (Vec<NoteInstance>, [u32; 129]) {
    let results: Vec<Vec<NoteInstance>> = (0u8..=127)
        .into_par_iter()
        .map(|key| {
            stacker::maybe_grow(STACK_RED_ZONE, STACK_SIZE, || {
                let notes = midi.key_notes(key);
                if notes.is_empty() {
                    return Vec::new();
                }

                let mut local = Vec::new();
                for note in notes {
                    if !track_visible
                        .get(note.track as usize)
                        .copied()
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    if hidden_notes.contains(&(note.track, note.start_tick, key)) {
                        continue;
                    }
                    local.push(NoteInstance {
                        start_tick: note.start_tick,
                        end_tick: note.end_tick,
                        packed: NoteInstance::pack(key, note.track, note.velocity),
                        reserved: 0,
                    });
                }

                local
            })
        })
        .collect();

    let mut offsets = [0u32; 129];
    let mut total = 0u32;
    let mut all = Vec::new();
    for (k, bucket) in results.into_iter().enumerate() {
        offsets[k] = total;
        total += bucket.len() as u32;
        all.extend(bucket);
    }
    offsets[128] = total;
    (all, offsets)
}

/// Build note instances for a single key bucket (for incremental upload).
/// Same filtering logic as `build_all_notes` but scoped to one key.
pub fn build_key_notes(
    midi: &dyn NoteSource,
    key: u8,
    hidden_notes: &std::collections::HashSet<(u16, u32, u8)>,
    track_visible: &[bool],
) -> Vec<NoteInstance> {
    let notes = midi.key_notes(key);
    let mut local = Vec::new();
    for note in notes {
        if !track_visible
            .get(note.track as usize)
            .copied()
            .unwrap_or(true)
        {
            continue;
        }
        if hidden_notes.contains(&(note.track, note.start_tick, key)) {
            continue;
        }
        local.push(NoteInstance {
            start_tick: note.start_tick,
            end_tick: note.end_tick,
            packed: NoteInstance::pack(key, note.track, note.velocity),
            reserved: 0,
        });
    }
    local
}



/// Build a single ghost note instance for the pencil tool preview (layer 4).
/// Uses the note's track color at full opacity so it appears as a solid preview
/// on top of the existing notes. Color is fetched from track_colors storage
/// buffer in the shader (same as regular notes).
pub fn build_ghost_note(
    out: &mut Vec<NoteInstance>,
    start_tick: u32,
    end_tick: u32,
    key: u8,
    track: u16,
    _theme: &GpuTheme,
) {
    out.push(NoteInstance {
        start_tick,
        end_tick,
        packed: NoteInstance::pack(key, track, 0),
        reserved: 0,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_test_helpers::make_midi;
    use yinhe_types::TimelineViewBase;

    fn make_view() -> PianoRollView {
        PianoRollView {
            base: TimelineViewBase {
                pixels_per_tick: 0.15,
                scroll_x: 0.0,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            key_height: 12.0,
        }
    }

    fn make_theme() -> GpuTheme {
        GpuTheme::default()
    }

    #[test]
    fn test_build_grid_basic() {
        let mut out = Vec::new();
        let view = make_view();
        let theme = make_theme();
        build_grid(&mut out, 800.0, 500.0, &view, 480, 4, 2, &[], 0.0, &theme);
        assert!(!out.is_empty(), "grid should produce lines");
        for inst in &out {
            // Grid lines are centered: x = tick_x - line_width/2
            // First line at tick 0 has tick_x = 60.0, line_width = 2.0 → x = 59.0
            assert!(inst.x >= 58.0, "grid line should be near keyboard boundary");
            assert!(inst.x <= 800.0, "grid line should be within viewport");
            assert_eq!(inst.h, 500.0);
        }
    }

    #[test]
    fn test_build_grid_with_time_sig_change() {
        let mut out = Vec::new();
        let view = make_view();
        let theme = make_theme();
        let sigs = vec![
            TimeSigEvent { tick: 0, numerator: 4, denominator: 2 },
            TimeSigEvent { tick: 1920, numerator: 3, denominator: 2 },
        ];
        build_grid(&mut out, 800.0, 500.0, &view, 480, 4, 2, &sigs, 0.0, &theme);
        assert!(!out.is_empty());
    }

    #[test]
    fn test_build_notes_basic() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert!(!out.is_empty(), "should produce note instances");
        let note = &out[0];
        assert_eq!(note.start_tick, 0);
        assert_eq!(note.end_tick, 480);
        // packed = key(100) | track(0) | vel(100)
        assert_eq!(note.packed & 0xFF, 100, "key");
        assert_eq!((note.packed >> 8) & 0xFFFF, 0, "track");
        assert_eq!((note.packed >> 24) & 0xFF, 100, "velocity");
    }

    #[test]
    fn test_build_notes_hidden_track() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![false];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert!(out.is_empty(), "notes on hidden track should be skipped");
    }

    #[test]
    fn test_build_notes_tag_is_track_index() {
        let mut out: Vec<NoteInstance> = Vec::new();
        // Create a note on track 2
        let midi = make_midi(vec![(100, 0, 480, 2, 100)]);
        let view = make_view();
        let track_visible = vec![true, true, true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 2, "track should be 2");
    }

    #[test]
    fn test_build_notes_tag_is_track_zero() {
        let mut out: Vec<NoteInstance> = Vec::new();
        // Create a note on track 0
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert_eq!((out[0].packed >> 8) & 0xFFFF, 0, "track should be 0");
    }

    #[test]
    fn test_build_notes_multiple_keys() {
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![
            (100, 0, 480, 0, 100),
            (104, 0, 480, 0, 80),
            (107, 0, 480, 0, 90),
        ]);
        let view = make_view();
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert_eq!(out.len(), 3, "should produce 3 note instances");
    }

    #[test]
    fn test_build_notes_long_note_crossing_left_edge() {
        // A note that starts far off-screen-left but extends into the viewport
        // must still be built (no padding; relies on the max_end look-back).
        let mut out: Vec<NoteInstance> = Vec::new();
        // Note spans tick 0..100000, key 100.
        let midi = make_midi(vec![(100, 0, 100000, 0, 100)]);
        let mut view = make_view();
        // Scroll right so the note's start is far off-screen to the left,
        // but its body still covers the viewport.
        view.base.scroll_x = 5000.0;
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert!(
            !out.is_empty(),
            "long note crossing the left edge must be included"
        );
    }

    #[test]
    fn test_build_notes_skips_fully_offscreen() {
        // A short note entirely to the left of the viewport must NOT be built.
        let mut out: Vec<NoteInstance> = Vec::new();
        let midi = make_midi(vec![(100, 0, 480, 0, 100)]);
        let mut view = make_view();
        view.base.scroll_x = 5000.0; // viewport starts well past tick 480
        let track_visible = vec![true];

        let hidden = std::collections::HashSet::new();
        build_notes(&mut out, 800.0, 500.0, &midi, &view, &hidden, &track_visible);
        assert!(
            out.is_empty(),
            "note fully off-screen-left must be culled"
        );
    }

}
