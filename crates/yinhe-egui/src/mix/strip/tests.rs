use super::controls::{gain_at_y, gain_y};
use super::*;
use crate::widgets::meter::{DB_MAX, db_to_gain};

/// 电平表矩形：y 从 0（顶 = +6dB）到 100（底 = -60dB）。
const TOP: f32 = 0.0;
const BOTTOM: f32 = 100.0;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3
}

#[test]
fn gain_y_boundaries() {
    // 静音（≤ -60dB）= 最底部。
    assert!(approx(gain_y(0.0, TOP, BOTTOM), BOTTOM));
    // 三次方增益曲线：0 dB 位于约 79.4% 高度（不是 dB 线性的 90.9%）。
    assert!(approx(gain_y(1.0, TOP, BOTTOM), BOTTOM - 79.4328));
    // 超过 DB_MAX 的增益顶格。
    assert!(approx(gain_y(db_to_gain(DB_MAX), TOP, BOTTOM), TOP));
    assert!(approx(gain_y(10.0, TOP, BOTTOM), TOP));
}

#[test]
fn gain_at_y_boundaries() {
    assert_eq!(gain_at_y(BOTTOM, TOP, BOTTOM), 0.0);
    assert!(approx(gain_at_y(TOP, TOP, BOTTOM), db_to_gain(DB_MAX)));
    // y 超出矩形：夹到两端。
    assert_eq!(gain_at_y(BOTTOM + 50.0, TOP, BOTTOM), 0.0);
    assert!(approx(
        gain_at_y(TOP - 50.0, TOP, BOTTOM),
        db_to_gain(DB_MAX)
    ));
}

#[test]
fn gain_position_round_trip() {
    for gain in [0.0, 0.01, 0.1, 0.5, 1.0, 1.5, db_to_gain(DB_MAX)] {
        let y = gain_y(gain, TOP, BOTTOM);
        let back = gain_at_y(y, TOP, BOTTOM);
        assert!(approx(back, gain), "gain={gain} y={y} back={back}");
    }
}

/// C 的 SEND 聚焦模式固定一行，不再随总线数增长。
#[test]
fn auto_height_is_fixed_by_focus_send_row() {
    let base = auto_panel_height();
    assert!((base - 200.0).abs() < 1e-3, "基础高度 {base}");
}

/// 方案 C 回归：MIX 通道条的选中态与 AR 共用 `track_selected`。
#[test]
fn strip_selected_follows_track_selection() {
    use yinhe_editor_core::document::Document;

    let mut doc = Document::empty();
    // 空白工程：track 0 = Master，track 1 = A01（global 0）。
    assert_eq!(doc.data.model.tracks[1].global_channel(), 0);
    assert!(!super::strip_selected_doc(
        &doc,
        yinhe_types::MixerChannelId::Midi(0)
    ));

    doc.edit.track_selected.insert(1);
    assert!(super::strip_selected_doc(
        &doc,
        yinhe_types::MixerChannelId::Midi(0)
    ));
    assert!(!super::strip_selected_doc(
        &doc,
        yinhe_types::MixerChannelId::Midi(1)
    ));
}
