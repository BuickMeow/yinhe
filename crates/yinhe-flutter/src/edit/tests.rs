use super::*;
use crate::scene::RenderScene;

fn scene(display: Option<usize>, active: Option<usize>, tracks: usize) -> RenderScene {
    RenderScene {
        num_tracks: tracks,
        display_track: display,
        active_track: active,
        ..Default::default()
    }
}

#[test]
fn conductor_view_has_no_write_track() {
    assert_eq!(write_track(&scene(Some(0), Some(0), 3)), None);
}

#[test]
fn explicit_track_writes_that_track() {
    assert_eq!(write_track(&scene(Some(2), Some(0), 3)), Some(2));
    assert_eq!(write_track(&scene(None, Some(2), 3)), Some(2));
}

#[test]
fn all_tracks_falls_back_to_first_note_track() {
    assert_eq!(write_track(&scene(None, None, 3)), Some(1));
}

#[test]
fn conductor_active_has_no_write_track_even_with_all_tracks() {
    assert_eq!(write_track(&scene(None, Some(0), 3)), None);
}

#[test]
fn selection_handles_map_corner_scale_and_gate() {
    // 选框 0..100 × 0..100，量化格 20px。
    assert_eq!(
        selection_handle_hit(0.0, 0.0, 100.0, 100.0, 100.0, 100.0, Some(20.0)),
        Some(4),
        "右下角优先重复"
    );
    assert_eq!(
        selection_handle_hit(0.0, 0.0, 100.0, 100.0, 100.0, 50.0, Some(20.0)),
        Some(3),
        "右缘中点整体变速"
    );
    assert_eq!(
        selection_handle_hit(0.0, 0.0, 100.0, 100.0, 80.0, 50.0, Some(20.0)),
        Some(5),
        "右缘内缩一格批量 gate"
    );
    assert_eq!(
        selection_handle_hit(0.0, 0.0, 100.0, 100.0, 60.0, 50.0, None),
        None,
        "AR 没有批量 gate 控制点"
    );
}

#[test]
fn pr_geom_handles_88_key_range() {
    let mut view = yinhe_types::PianoRollView::default();
    view.set_key_count(88);
    view.key_height = 10.0;
    let geom = PrGeom::new(&view);
    for key in 21..=108u8 {
        let y = geom.key_to_y(key) + geom.kh * 0.5;
        assert_eq!(geom.y_to_key(y), key, "key {key} roundtrip");
    }
}

#[test]
fn note_move_snaps_start_to_absolute_grid() {
    let mut s = RenderScene::default();
    s.pr_view.base.pixels_per_tick = 10.0;
    let kbw = s.pr_view.keyboard_width();
    let x_of = |tick: f32| kbw + tick * 10.0;
    // 起点 1（非整格）、量化 2；指针拖到 tick 6。
    // 相对位移会得到 1 + 4 = 5；绝对坐标应落在 6。
    let d = DragState {
        anchor_tick_raw: 1.0,
        start_tick: 1,
        ..Default::default()
    };
    let start = note_move_start_abs(&s, QuantizePreset::Absolute(2), &d, x_of(6.0));
    assert_eq!(start, 6);
}
