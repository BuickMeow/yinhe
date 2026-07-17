use yinhe_types::view_base::TimelineViewBase;
use yinhe_types::{ArrangementView, PianoRollView};
use yinhe_editor_core::follow::{FollowMode, compute_follow_scroll, total_ticks_padded};
use yinhe_editor_core::quantize::QuantizePreset;

fn make_base() -> TimelineViewBase {
    TimelineViewBase {
        pixels_per_tick: 0.1,
        scroll_x: 0.0,
        scroll_y: 0.0,
        left_panel_width: 100.0,
        dirty: false,
        track_panel_row_height: 40.0,
        track_panel_scroll_y: 0.0,
    }
}

// ── TimelineViewBase ──

#[test]
fn tick_to_x_at_origin() {
    let base = make_base();
    assert!((base.tick_to_x(0.0) - 100.0).abs() < 1e-6);
}

#[test]
fn tick_to_x_with_scroll() {
    let mut base = make_base();
    base.scroll_x = 100.0;
    let x = base.tick_to_x(0.0);
    assert!((x - 0.0).abs() < 1e-6, "tick_to_x(0) with scroll=100 should be 0, got {x}");
}

#[test]
fn tick_to_x_x_to_tick_roundtrip() {
    let base = make_base();
    let tick = 1234.5;
    let x = base.tick_to_x(tick);
    let back = base.x_to_tick(x);
    assert!((back - tick).abs() < 0.01, "roundtrip failed: {tick} -> {x} -> {back}");
}

#[test]
fn visible_tick_range() {
    let base = make_base();
    let (start, end) = base.visible_tick_range(1100.0);
    assert!((start - 0.0).abs() < 1.0);
    assert!((end - 10000.0).abs() < 1.0);
}

#[test]
fn zoom_around_x_preserves_tick() {
    let mut base = make_base();
    let pointer_x = 300.0;
    let tick_before = base.x_to_tick(pointer_x);
    base.zoom_around_x(pointer_x, 2.0);
    let tick_after = base.x_to_tick(pointer_x);
    assert!((tick_after - tick_before).abs() < 1.0,
        "zoom should preserve tick at pointer: {tick_before} vs {tick_after}");
}

#[test]
fn clamp_scroll_x() {
    let mut base = make_base();
    base.scroll_x = 99999.0;
    base.clamp_scroll_x(1000.0, 10000.0);
    // max = 10000*0.1 - (1000-100) = 1000 - 900 = 100
    assert!(base.scroll_x <= 100.0 + 0.01);
}

// ── PianoRollView ──

#[test]
fn pianoroll_tick_to_x_roundtrip() {
    let view = PianoRollView::default();
    let tick = 500.0;
    let x = view.tick_to_x(tick);
    let back = view.x_to_tick(x);
    assert!((back - tick).abs() < 0.01);
}

#[test]
fn pianoroll_key_to_y_roundtrip() {
    let view = PianoRollView::default();
    let key = 60u8;
    let y = view.key_to_y(key);
    let back = view.y_to_key(y);
    assert_eq!(back, key);
}

#[test]
fn pianoroll_visible_tick_range() {
    let view = PianoRollView::default();
    let (start, end) = view.visible_tick_range(800.0);
    assert!(start < end);
}

#[test]
fn pianoroll_visible_key_range() {
    let view = PianoRollView::default();
    let (lo, hi) = view.visible_key_range(600.0);
    assert!(lo <= hi);
    assert!(lo >= 0);
    assert!(hi <= 127);
}

#[test]
fn pianoroll_keyboard_width() {
    let view = PianoRollView::default();
    assert!(view.keyboard_width() > 0.0);
}

#[test]
fn pianoroll_zoom_around_x() {
    let mut view = PianoRollView::default();
    let ppt_before = view.base.pixels_per_tick;
    view.zoom_around_x(400.0, 2.0);
    assert!(view.base.pixels_per_tick > ppt_before);
}

#[test]
fn pianoroll_zoom_around_y() {
    let mut view = PianoRollView::default();
    let kh_before = view.key_height;
    view.zoom_around_y(300.0, 2.0, 600.0);
    assert!(view.key_height > kh_before);
}

// ── ArrangementView ──

#[test]
fn arrangement_tick_to_x_roundtrip() {
    let view = ArrangementView::default();
    let tick = 500.0;
    let x = view.tick_to_x(tick);
    let back = view.x_to_tick(x);
    assert!((back - tick).abs() < 0.01);
}

#[test]
fn arrangement_lane_y() {
    let view = ArrangementView::default();
    let y0 = view.lane_y(0);
    let y1 = view.lane_y(1);
    assert!(y1 > y0, "lane 1 should be below lane 0");
}

#[test]
fn arrangement_visible_tick_range() {
    let view = ArrangementView::default();
    let (start, end) = view.visible_tick_range(800.0);
    assert!(start < end);
}

#[test]
fn arrangement_zoom_around_x() {
    let mut view = ArrangementView::default();
    let ppt_before = view.base.pixels_per_tick;
    view.zoom_around_x(400.0, 2.0);
    assert!(view.base.pixels_per_tick > ppt_before);
}

// ── FollowMode ──

#[test]
fn follow_mode_next_cycles() {
    assert_eq!(FollowMode::None.next(), FollowMode::Page);
    assert_eq!(FollowMode::Page.next(), FollowMode::Continuous);
    assert_eq!(FollowMode::Continuous.next(), FollowMode::None);
}

#[test]
fn follow_mode_tooltip_not_empty() {
    assert!(!FollowMode::None.tooltip().is_empty());
    assert!(!FollowMode::Page.tooltip().is_empty());
    assert!(!FollowMode::Continuous.tooltip().is_empty());
}

#[test]
fn compute_follow_scroll_none_mode() {
    assert_eq!(
        compute_follow_scroll(100.0, 1.0, 800.0, 0.0, FollowMode::None, 1.0),
        None
    );
}

#[test]
fn compute_follow_scroll_continuous_mode() {
    let result = compute_follow_scroll(100.0, 2.0, 800.0, 0.0, FollowMode::Continuous, 50.0);
    assert_eq!(result, Some(150.0));
}

#[test]
fn compute_follow_scroll_page_mode_scrolls_at_edge() {
    let result = compute_follow_scroll(900.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0);
    assert!(result.is_some(), "should scroll when near right edge");
}

#[test]
fn compute_follow_scroll_page_mode_no_scroll_in_center() {
    assert_eq!(
        compute_follow_scroll(400.0, 1.0, 800.0, 0.0, FollowMode::Page, 1.0),
        None
    );
}

#[test]
fn total_ticks_padded_positive() {
    assert!((total_ticks_padded(1000) - 1200.0).abs() < 0.01);
}

#[test]
fn total_ticks_padded_zero() {
    assert_eq!(total_ticks_padded(0), 0.0);
}

// ── QuantizePreset ──

#[test]
fn quantize_tick_interval_quarter() {
    let interval = QuantizePreset::Fraction(1, 4).tick_interval(480);
    assert_eq!(interval, 480);
}

#[test]
fn quantize_tick_interval_eighth() {
    let interval = QuantizePreset::Fraction(1, 8).tick_interval(480);
    assert_eq!(interval, 240);
}

#[test]
fn quantize_tick_interval_sixteenth() {
    let interval = QuantizePreset::Fraction(1, 16).tick_interval(480);
    assert_eq!(interval, 120);
}

#[test]
fn quantize_snap_tick() {
    let snapped = QuantizePreset::Fraction(1, 4).snap_tick(500.0, 480);
    assert_eq!(snapped, 480.0);
}

#[test]
fn quantize_snap_tick_at_boundary() {
    let snapped = QuantizePreset::Fraction(1, 4).snap_tick(480.0, 480);
    assert_eq!(snapped, 480.0);
}

#[test]
fn quantize_label_not_empty() {
    assert!(!QuantizePreset::Fraction(1, 4).label().is_empty());
    assert!(!QuantizePreset::Fraction(1, 16).label().is_empty());
}

#[test]
fn quantize_label_not_empty() {
    assert!(!QuantizePreset::Fraction(1, 4).label().is_empty());
}

// ── Time formatting (yinhe-types) ──

#[test]
fn format_time_zero() {
    assert_eq!(yinhe_types::time_format::format_time(0.0), "0:00.000");
}

#[test]
fn format_time_seconds() {
    assert_eq!(yinhe_types::time_format::format_time(65.123), "1:05.123");
}

#[test]
fn format_bpm() {
    assert_eq!(yinhe_types::time_format::format_bpm(120.0), "120.00");
}

#[test]
fn format_time_sig_4_4() {
    assert_eq!(yinhe_types::time_format::format_time_sig(4, 2), "4/4");
}

#[test]
fn format_tick_bar_beat() {
    assert_eq!(yinhe_types::time_format::format_tick_bar_beat(0.0, 480, 4), "1.1.000");
    assert_eq!(yinhe_types::time_format::format_tick_bar_beat(480.0, 480, 4), "1.2.000");
    assert_eq!(yinhe_types::time_format::format_tick_bar_beat(1920.0, 480, 4), "2.1.000");
}
