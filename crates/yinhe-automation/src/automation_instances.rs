use yinhe_types::{AutomationEvent, AutomationLane, TRACK_PALETTE, TimeSigEvent};

use crate::AutomationPanelView;
use crate::grid;
use crate::{NoteInstance, pack_props, pack_rgba};

/// Bar width in pixels for automation events.
const BAR_WIDTH: f32 = 2.0;
/// Corner radius for automation bars (flat).
const BAR_ROUNDING: f32 = 0.0;
/// Border width for automation bars.
const BAR_BORDER: f32 = 0.0;
/// Color for the center/default reference line.
const CENTER_LINE_COLOR: (f32, f32, f32) = (0.30, 0.30, 0.35);
/// Center line alpha.
const CENTER_LINE_ALPHA: f32 = 0.6;

/// Build all instances for one automation panel: background, grid, data bars, reference line.
///
/// **Coordinate system:** The wgpu texture covers the full panel width (from
/// `x=0`). The combo/dropdown area is overlaid on top in egui, so this texture
/// fills the entire visible area. Grid lines start at `left_panel_width` (the
/// combo area width), and data bars are offset by `left_panel_width` so they
/// align with the pianoroll content above.
pub fn build_automation_instances(
    instances: &mut Vec<NoteInstance>,
    width: u32,
    height: u32,
    view: &AutomationPanelView,
    lane: Option<&AutomationLane>,
    tpb: Option<u32>,
    default_num: u8,
    default_den: u8,
    time_sig_events: &[TimeSigEvent],
    track_visible: &[bool],
    track_colors: &[[f32; 3]],
) {
    let w = width as f32;
    let h = height as f32;
    let ppu = view.base.pixels_per_tick;

    // 1. Background — fill the entire texture (grid area only)
    instances.push(NoteInstance {
        x: 0.0,
        y: 0.0,
        w,
        h,
        rgba_packed: pack_rgba(
            grid::PR_BG_COLOR.0,
            grid::PR_BG_COLOR.1,
            grid::PR_BG_COLOR.2,
            1.0,
        ),
        props_packed: pack_props(0.0, 0.0),
        velocity: 0,
        tag: 0,
    });

    // 2. Grid lines (vertical only — reuse the shared timeline grid builder)
    // Grid lines start at left_panel_width (= combo width) so they don't
    // render beneath the combo overlay; the background quad still fills
    // the full texture so the entire panel has the grid background color.
    if let Some(tpb) = tpb {
        grid::build_timeline_grid(
            instances,
            w,
            h,
            &view.base,
            tpb,
            default_num,
            default_den,
            time_sig_events,
            grid::PR_MEASURE_LINE_COLOR,
            grid::PR_BEAT_LINE_COLOR,
            Some(grid::PR_SUB_BEAT_LINE_COLOR),
        );
    }

    // 3. Reference / center line
    if let Some(lane) = lane {
        let target = &lane.target;
        let max_val = target.max_value() as f32;
        if max_val > 0.0 && target.has_center_line() {
            let center_val = target.default_value() as f32;
            let y_center = h - (center_val / max_val) * h;
            // Draw a thin horizontal line across the panel
            instances.push(NoteInstance {
                x: 0.0,
                y: y_center - 0.5,
                w,
                h: 1.0,
                rgba_packed: pack_rgba(
                    CENTER_LINE_COLOR.0,
                    CENTER_LINE_COLOR.1,
                    CENTER_LINE_COLOR.2,
                    CENTER_LINE_ALPHA,
                ),
                props_packed: pack_props(0.0, 0.0),
                velocity: 0,
                tag: 0,
            });
        }

        // 4. Data bars — visible tick range only.
        // Events are discrete vertical bars; off-screen bars are also rejected
        // by the per-bar x check below. No extra tick padding needed.
        let (tick_start, tick_end) = view.base.visible_tick_range(w);
        let pad_start = tick_start.max(0.0) as u32;
        let pad_end = tick_end.max(0.0) as u32;

        let events = lane.events_in_range(pad_start, pad_end);
        let x_offset = view.base.left_panel_width - view.base.scroll_x;

        // ── Collect unique (tick, value) bars, keeping first occurrence ──
        // Events are sorted by tick. Within each tick, we track seen values
        // so duplicate values (even across tracks) only produce one bar.
        struct Bar<'a> {
            evt: &'a AutomationEvent,
            bar_x: f32,
            bar_h: f32,
            color: [f32; 3],
        }

        let mut bars: Vec<Bar> = Vec::new();
        let mut last_tick = u32::MAX;
        let mut seen_values: Vec<u16> = Vec::new();

        for evt in events {
            // Skip events from hidden tracks
            let trk_idx = evt.track as usize;
            if !track_visible.get(trk_idx).copied().unwrap_or(true) {
                continue;
            }

            // New tick → reset seen values
            if evt.tick != last_tick {
                last_tick = evt.tick;
                seen_values.clear();
            }

            // Skip duplicate values at the same tick
            if seen_values.contains(&evt.value) {
                continue;
            }
            seen_values.push(evt.value);

            let val = evt.value as f32;
            let max_val = max_val.max(1.0);
            let bar_h = ((val + 1.0) / (max_val + 1.0)) * h;
            let bar_x = x_offset + evt.tick as f32 * ppu;

            // Skip if completely off-screen
            if bar_x + BAR_WIDTH < 0.0 || bar_x > w {
                continue;
            }

            let trk = evt.track as usize;
            let color = track_colors
                .get(trk)
                .copied()
                .unwrap_or_else(|| TRACK_PALETTE[trk % TRACK_PALETTE.len()]);

            bars.push(Bar {
                evt,
                bar_x,
                bar_h,
                color,
            });
        }

        // ── Sort by value descending so higher values draw first (bottom) ──
        // Lower values draw on top, so at the same tick the bottom portion
        // shows the lower value's track color while the top shows the higher.
        bars.sort_by(|a, b| b.evt.value.cmp(&a.evt.value));

        for bar in &bars {
            instances.push(NoteInstance {
                x: bar.bar_x,
                y: h - bar.bar_h,
                w: BAR_WIDTH,
                h: bar.bar_h,
                rgba_packed: pack_rgba(bar.color[0], bar.color[1], bar.color[2], 0.85),
                props_packed: pack_props(BAR_ROUNDING, BAR_BORDER),
                velocity: bar.evt.value as u32,
                tag: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_types::{
        AutomationEvent, AutomationLane, AutomationTarget, TimeSigEvent, TimelineViewBase,
    };

    /// Helper: build a minimal AutomationPanelView with predictable scroll state.
    fn make_view(pixels_per_tick: f32, scroll_x: f32, panel_height: f32) -> AutomationPanelView {
        AutomationPanelView {
            base: TimelineViewBase {
                pixels_per_tick,
                scroll_x,
                scroll_y: 0.0,
                left_panel_width: 60.0,
                dirty: true,
                track_panel_row_height: 40.0,
                track_panel_scroll_y: 0.0,
            },
            panel_height,
            selected_target: AutomationTarget::CC { controller: 7 },
            lane_index: 0,
            dirty: true,
        }
    }

    /// Helper: build an AutomationLane with events at given ticks on channel 0, track 0.
    fn make_lane(target: AutomationTarget, ticks: &[(u32, u16)]) -> AutomationLane {
        AutomationLane {
            target,
            events: ticks
                .iter()
                .map(|&(tick, value)| AutomationEvent {
                    tick,
                    value,
                    channel: 0,
                    track: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn test_background_always_first() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, None, 4, 2, &[], &[], &[]);

        assert!(!instances.is_empty());
        let bg = &instances[0];
        assert_eq!(bg.x, 0.0);
        assert_eq!(bg.y, 0.0);
        assert_eq!(bg.w, 800.0);
        assert_eq!(bg.h, 100.0);
    }

    #[test]
    fn test_no_grid_when_tpb_is_none() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, None, 4, 2, &[], &[], &[]);

        assert_eq!(instances.len(), 1, "no TPB means no grid lines");
    }

    #[test]
    fn test_grid_lines_when_tpb_provided() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);

        build_automation_instances(&mut instances, 800, 100, &view, None, Some(480), 4, 2, &[], &[], &[]);

        assert!(instances.len() > 1);
        let grid_line = &instances[1];
        assert!(grid_line.x >= 0.0);
        assert!(grid_line.w < 5.0, "grid lines should be thin");
    }

    #[test]
    fn test_center_line_appears_for_pitch_bend() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::PitchBend,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        let center = &instances[1];
        let expected_center_y = 100.0 - (8192.0_f32 / 16383.0_f32) * 100.0 - 0.5;
        assert!((center.y - expected_center_y).abs() < 0.001);
        assert_eq!(center.w, 800.0);
        assert_eq!(center.h, 1.0);
    }

    #[test]
    fn test_center_line_appears_for_fine_tune() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::FineTune,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        let center = &instances[1];
        assert!((center.y - (100.0 - 50.0 - 0.5)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_no_center_line_for_velocity() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::Velocity,
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_no_center_line_for_cc() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_data_bars_positioned_correctly() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 64), (200, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 3);

        // Bars are sorted by value descending: val=127 first, then val=64.
        // Bar at tick=200, value=127 (full height, drawn first = bottom)
        let bar0 = &instances[1];
        assert!((bar0.x - 260.0).abs() < 0.001);
        assert!((bar0.y - 0.0).abs() < 0.001);
        assert_eq!(bar0.w, 2.0);

        // Bar at tick=100, value=64, max=127
        // New mapping: bar_h = ((val+1)/(max_val+1)) * h
        // x = 60 - 0 + 100*1.0 = 160
        let bar1 = &instances[2];
        assert!((bar1.x - 160.0).abs() < 0.001);
        let expected_y1 = 100.0 - ((64.0 + 1.0) / (127.0 + 1.0)) * 100.0;
        assert!((bar1.y - expected_y1).abs() < 0.001);
        assert_eq!(bar1.w, 2.0);
    }

    #[test]
    fn test_off_screen_events_are_skipped() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 1000.0, 100.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 64), (200, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1, "off-screen events should be skipped");
    }

    #[test]
    fn test_scrolled_events_appear_at_correct_x() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 50.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(100, 64)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        // x = 60 - 50 + 100 = 110
        let bar = &instances[1];
        assert!((bar.x - 110.0).abs() < 0.001);
    }

    #[test]
    fn test_data_bar_height_scales_with_value() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 200.0);
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[(100, 0), (200, 64), (300, 127)],
        );

        build_automation_instances(
            &mut instances,
            800,
            200,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 4);

        // Bars are sorted by value descending: val=127, 64, 0.
        // value=127: full height
        assert!((instances[1].h - 200.0).abs() < 0.001);
        assert!((instances[1].y - 0.0).abs() < 0.001);

        // value=64: mid height
        assert!((instances[2].h - 101.5625).abs() < 0.001);
        assert!((instances[2].y - 98.4375).abs() < 0.001);

        // value=0: min height = (1/128)*200 = 1.5625
        assert!((instances[3].h - 1.5625).abs() < 0.001);
        assert!((instances[3].y - 198.4375).abs() < 0.001);
    }

    #[test]
    fn test_track_colors_from_palette() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                AutomationEvent {
                    tick: 100,
                    value: 64,
                    channel: 0,
                    track: 0,
                },
                AutomationEvent {
                    tick: 200,
                    value: 64,
                    channel: 0,
                    track: 1,
                },
            ],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 3);
        assert_ne!(instances[1].rgba_packed, instances[2].rgba_packed);
        assert_eq!(instances[1].velocity, 64);
        assert_eq!(instances[2].velocity, 64);
    }

    #[test]
    fn test_empty_lane_produces_no_data_bars() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![],
        };

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn test_zero_dimensions_produces_valid_instances() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 0.0);

        build_automation_instances(&mut instances, 0, 0, &view, None, None, 4, 2, &[], &[], &[]);

        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].w, 0.0);
        assert_eq!(instances[0].h, 0.0);
    }

    #[test]
    fn test_max_value_bar_full_height() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(AutomationTarget::PitchBend, &[(100, 16383)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        // Background + center line + 1 bar
        assert_eq!(instances.len(), 3);
        let bar = &instances[2];
        assert!((bar.y - 0.0).abs() < 0.001);
        assert!((bar.h - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_event_partially_off_screen_left_edge() {
        let mut instances = Vec::new();
        // scroll_x = 0, event at tick 0 with left_panel_width=60
        // event x = 60 - 0 + 0 = 60 (just past left edge of visible area?)
        // Actually: bar_x = x_offset + tick * ppu = 60 - 0 + 0 = 60
        // visible area x range is 0..800, so x=60 is on-screen
        let view = make_view(1.0, 0.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(0, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 2);
        assert!((instances[1].x - 60.0).abs() < 0.001);
    }

    #[test]
    fn test_negative_bar_x_skipped() {
        let mut instances = Vec::new();
        // scroll_x = 100, left_panel_width = 60
        // event at tick 10: x = 60 - 100 + 10 = -30  (< 0, off-screen to left)
        let view = make_view(1.0, 100.0, 100.0);
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(10, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(instances.len(), 1, "bar with negative x should be skipped");
    }

    #[test]
    fn test_bar_beyond_right_edge_skipped() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 100.0);
        // event at tick 1000: x = 60 + 1000 = 1060 > 800 (width) → off-screen
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[(1000, 100)]);

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[], &[], &[]);

        assert_eq!(
            instances.len(),
            1,
            "bar beyond right edge should be skipped"
        );
    }

    #[test]
    fn test_grid_with_time_signature_events() {
        let mut instances = Vec::new();
        let view = make_view(1.0, 0.0, 80.0);
        let ts_events = [TimeSigEvent {
            tick: 0,
            numerator: 6,
            denominator: 2,
        }];

        build_automation_instances(
            &mut instances,
            800,
            100,
            &view,
            None,
            Some(480),
            4,
            2,
            &ts_events, &[], &[]);

        // Background + grid lines (no lane)
        assert!(instances.len() > 1);
    }

    #[test]
    fn test_track_visibility_filters_bars() {
        // Lane has 2 events, one on each track. With track_visible=[true,false],
        // only the track-0 event should produce a bar.
        let view = make_view(1.0, 0.0, 100.0);
        let lane = AutomationLane {
            target: AutomationTarget::CC { controller: 7 },
            events: vec![
                yinhe_types::AutomationEvent {
                    tick: 100,
                    value: 64,
                    channel: 0,
                    track: 0,
                },
                yinhe_types::AutomationEvent {
                    tick: 200,
                    value: 80,
                    channel: 1,
                    track: 1,
                },
            ],
        };

        // Both visible → both bars rendered.
        let mut both = Vec::new();
        build_automation_instances(
            &mut both,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[],
            &[true, true],
            &[],
        );

        // Only track 0 visible → only one bar.
        let mut only0 = Vec::new();
        build_automation_instances(
            &mut only0,
            800,
            100,
            &view,
            Some(&lane),
            None,
            4,
            2,
            &[],
            &[true, false],
            &[],
        );

        // The difference must be exactly one instance (the hidden bar).
        assert_eq!(both.len(), only0.len() + 1);
    }
}
