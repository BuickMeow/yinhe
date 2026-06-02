use yinhe_types::NoteSource;

use crate::arrangement_view::ArrangementView;
use crate::vertex::{NoteInstance, pack_props, pack_rgba};

/// Colors
const BG_COLOR: (f32, f32, f32) = (0.14, 0.14, 0.16);
const LANE_EVEN_COLOR: (f32, f32, f32) = (0.16, 0.16, 0.18);
const LANE_ODD_COLOR: (f32, f32, f32) = (0.13, 0.13, 0.15);
const MEASURE_LINE_COLOR: (f32, f32, f32, f32) = (0.30, 0.30, 0.35, 1.0);
const BEAT_LINE_COLOR: (f32, f32, f32, f32) = (0.20, 0.20, 0.23, 1.0);
const PLAYHEAD_COLOR: (f32, f32, f32, f32) = (1.0, 1.0, 1.0, 0.8);

const NOTE_ROUNDING: f32 = 0.2;

/// Safety cap to prevent GPU overload with extremely large MIDI files.
const MAX_NOTE_INSTANCES: usize = 500_000;

/// Build all instances for the arrangement view frame.
///
/// `instances` is a reusable scratch buffer — caller should retain it across frames.
pub fn build_arrangement_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    midi: Option<&dyn NoteSource>,
    view: &ArrangementView,
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
    cursor_tick: Option<f64>,
) {
    let w = width as f32;
    let h = height as f32;
    let lh = view.lane_height;
    let lb_w = view.label_width;
    let ppu = view.pixels_per_tick;
    let num_tracks = track_visible.len();

    // 1. Background quad
    instances.push(NoteInstance {
        x: lb_w,
        y: 0.0,
        w: w - lb_w,
        h,
        rgba_packed: pack_rgba(BG_COLOR.0, BG_COLOR.1, BG_COLOR.2, 1.0),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        flags: 0,
    });

    // 2. Track lane backgrounds (alternating colors)
    if num_tracks > 0 {
        let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
        for idx in trk_first..trk_last {
            if !track_visible.get(idx).copied().unwrap_or(true) {
                continue;
            }
            let y = view.lane_y(idx);
            let col = if idx % 2 == 0 { LANE_EVEN_COLOR } else { LANE_ODD_COLOR };
            instances.push(NoteInstance {
                x: lb_w,
                y,
                w: w - lb_w,
                h: lh,
                rgba_packed: pack_rgba(col.0, col.1, col.2, 1.0),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                flags: 0,
            });
        }
    }

    // 3. Grid lines + 4. Note rectangles
    if let Some(midi) = midi {
        if let Some(tpb) = midi.ticks_per_beat() {
            let (tick_start, tick_end) = view.visible_tick_range(w);

            // Grid lines
            if ppu > 0.01 {
                let ticks_per_measure = tpb * 4;
                let sub_beat_div = 4u32;
                let ticks_per_sub = (tpb / sub_beat_div).max(1);

                let start = ((tick_start / ticks_per_sub as f64).floor() as u32)
                    .saturating_mul(ticks_per_sub);
                let mut tick = start;
                while (tick as f64) <= tick_end {
                    let x = view.tick_to_x(tick as f64);
                    if x >= lb_w && x <= w {
                        let is_measure = tick % ticks_per_measure == 0;
                        let is_beat = tick % tpb == 0;
                        if is_measure {
                            instances.push(NoteInstance {
                                x,
                                y: 0.0,
                                w: 2.0,
                                h,
                                rgba_packed: pack_rgba(MEASURE_LINE_COLOR.0, MEASURE_LINE_COLOR.1, MEASURE_LINE_COLOR.2, MEASURE_LINE_COLOR.3),
                                props_packed: pack_props(0.0, 0.0),
                                velocity: 0,
                                flags: 0,
                            });
                        } else if is_beat {
                            instances.push(NoteInstance {
                                x,
                                y: 0.0,
                                w: 1.0,
                                h,
                                rgba_packed: pack_rgba(BEAT_LINE_COLOR.0, BEAT_LINE_COLOR.1, BEAT_LINE_COLOR.2, BEAT_LINE_COLOR.3),
                                props_packed: pack_props(0.0, 0.0),
                                velocity: 0,
                                flags: 0,
                            });
                        }
                    }
                    tick += ticks_per_sub;
                }
            }

            // Note rectangles — iterate all keys, draw each note in its track lane
            let tick_pad = (w - lb_w) / ppu;
            let pad_start = (tick_start - tick_pad as f64).max(0.0);
            let pad_end = tick_end + tick_pad as f64;
            let (trk_first, trk_last) = view.visible_track_range(h, num_tracks);
            let note_start_count = instances.len();

            for key in 0u8..128 {
                // Safety: stop adding notes if we hit the cap
                if instances.len() - note_start_count >= MAX_NOTE_INSTANCES {
                    break;
                }

                let notes = midi.key_notes(key);
                if notes.is_empty() {
                    continue;
                }

                // Quick skip: if the first note starts after pad_end, all do (sorted by start_tick)
                if notes.first().map_or(true, |n| n.start_tick as f64 > pad_end) {
                    continue;
                }

                for note in notes.iter() {
                    // Tick filter — notes are sorted by start_tick per key
                    if note.start_tick as f64 > pad_end {
                        break;
                    }
                    if (note.end_tick as f64) < pad_start {
                        continue;
                    }
                    // Track filter
                    let ti = note.track as usize;
                    if ti < trk_first || ti >= trk_last {
                        continue;
                    }
                    if !track_visible.get(ti).copied().unwrap_or(true) {
                        continue;
                    }

                    let nx = view.tick_to_x(note.start_tick as f64);
                    let nw = ((note.end_tick - note.start_tick) as f32 * ppu).max(2.0);
                    let ny = view.lane_y(ti);

                    // Map MIDI key (0-127) to vertical position within lane
                    // Higher keys at top of lane
                    let note_y = ny + lh - (note.key as f32 + 1.0) * (lh / 128.0);
                    let note_h = (lh / 128.0).max(1.0);

                    let color = track_colors.get(ti).copied().unwrap_or([0.5, 0.5, 0.5]);
                    let rounding = NOTE_ROUNDING * nw.min(note_h);

                    instances.push(NoteInstance {
                        x: nx,
                        y: note_y,
                        w: nw,
                        h: note_h,
                        rgba_packed: pack_rgba(color[0], color[1], color[2], 0.85),
                        props_packed: pack_props(rounding, 0.0),
                        velocity: note.velocity as u32,
                        flags: 0,
                    });
                }
            }
        }
    }

    // 5. Playhead
    if let Some(ct) = cursor_tick {
        let cx = view.tick_to_x(ct);
        if cx >= lb_w && cx <= w {
            instances.push(NoteInstance {
                x: cx,
                y: 0.0,
                w: 2.0,
                h,
                rgba_packed: pack_rgba(
                    PLAYHEAD_COLOR.0,
                    PLAYHEAD_COLOR.1,
                    PLAYHEAD_COLOR.2,
                    PLAYHEAD_COLOR.3,
                ),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                flags: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::Note;

    /// Mock MIDI data for testing.
    struct MockMidi {
        notes: [Vec<Note>; 128],
        tpb: u32,
        tick_len: u64,
    }

    impl NoteSource for MockMidi {
        fn key_notes(&self, key: u8) -> &[Note] {
            &self.notes[key as usize]
        }
        fn duration(&self) -> f64 {
            10.0
        }
        fn ticks_per_beat(&self) -> Option<u32> {
            Some(self.tpb)
        }
        fn tick_length(&self) -> Option<u64> {
            Some(self.tick_len)
        }
    }

    fn make_midi(notes: Vec<(u8, u32, u32, u16, u8)>) -> MockMidi {
        let mut key_notes: [Vec<Note>; 128] = core::array::from_fn(|_| Vec::new());
        let mut max_tick: u64 = 0;
        for (key, start_tick, end_tick, track, vel) in notes {
            let n = Note {
                key,
                start: start_tick as f64 / 480.0,
                end: end_tick as f64 / 480.0,
                start_tick,
                end_tick,
                velocity: vel,
                channel: 0,
                track,
            };
            if (end_tick as u64) > max_tick {
                max_tick = end_tick as u64;
            }
            key_notes[key as usize].push(n);
        }
        MockMidi {
            notes: key_notes,
            tpb: 480,
            tick_len: max_tick,
        }
    }

    #[test]
    fn test_basic_note_instances() {
        let mock = make_midi(vec![
            (60, 0, 480, 0, 100),   // Track 0, key 60
            (60, 480, 960, 0, 100), // Track 0, second note
            (64, 240, 720, 1, 80),  // Track 1, key 64
        ]);

        let view = ArrangementView::default();
        let track_visible = vec![true; 2];
        let track_colors = [[0.3, 0.6, 0.9], [0.9, 0.3, 0.3]];
        let mut instances = Vec::new();
        let w = 1200u32;
        let h = 400u32;

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            w,
            h,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 100, "build took too long: {:?}", elapsed);
        assert!(!instances.is_empty(), "should have generated instances");

        // Count note instances (velocity > 0 => actual note rectangle)
        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert_eq!(note_count, 3, "should have 3 note instances");

        // Verify note positions are reasonable
        for inst in &instances {
            if inst.velocity > 0 {
                assert!(inst.x >= view.label_width, "note x should be >= label_width");
                assert!(inst.w > 0.0, "note width should be positive");
            }
        }
    }

    #[test]
    fn test_all_keys_performance() {
        // Simulate a dense MIDI: 1 note on each of the 128 keys for track 0
        let mut notes = Vec::with_capacity(128);
        for key in 0..128u8 {
            notes.push((key, key as u32 * 10, key as u32 * 10 + 120, 0, 90));
        }
        let mock = make_midi(notes);

        let view = ArrangementView::default();
        let track_visible = vec![true; 1];
        let track_colors = [[0.3, 0.6, 0.9]];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            1200,
            400,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        // Should be very fast for 128 notes
        assert!(elapsed.as_millis() < 100, "128-key build took: {:?}", elapsed);
        assert!(instances.len() > 128, "should have many instances");
    }

    #[test]
    fn test_instance_cap() {
        // Create enough notes to overflow the cap
        let mut notes = Vec::new();
        let total_notes = MAX_NOTE_INSTANCES + 1000;
        for i in 0..total_notes {
            let key = (i % 128) as u8;
            // Spread across ticks to avoid early-skip optimization
            let tick = (i as u32) * 5;
            notes.push((
                key,
                tick,
                tick + 120,
                (i % 16) as u16,
                (50 + (i % 100) as u8),
            ));
        }
        let mock = make_midi(notes);

        let view = ArrangementView::default();
        let track_visible = vec![true; 16];
        let track_colors = [[0.5f32; 3]; 16];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            1200,
            800,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        // Even with overflow, should complete quickly
        assert!(elapsed.as_millis() < 500, "overflow build took: {:?}", elapsed);

        // The number of note instances should be at most MAX_NOTE_INSTANCES
        // (only count note instances, not bg/grid/playhead)
        let note_count = instances.iter().filter(|i| i.velocity > 0).count();
        assert!(note_count <= MAX_NOTE_INSTANCES, "exceeded cap: {} > {}", note_count, MAX_NOTE_INSTANCES);
    }

    #[test]
    fn test_grid_lines_dont_hang() {
        // Wide viewport should generate many grid lines
        let mock = make_midi(vec![(60, 0, 480, 0, 100)]);
        let view = ArrangementView {
            pixels_per_tick: 10.0, // Extreme zoom — lots of grid lines
            ..Default::default()
        };
        let track_visible = vec![true; 1];
        let track_colors = [[0.3, 0.6, 0.9]];
        let mut instances = Vec::new();

        let start = std::time::Instant::now();
        build_arrangement_instances(
            &mut instances,
            2000,
            400,
            Some(&mock as &dyn NoteSource),
            &view,
            &track_visible,
            &track_colors,
            None,
        );
        let elapsed = start.elapsed();
        assert!(elapsed.as_millis() < 200, "extreme zoom build took: {:?}", elapsed);
    }
}
