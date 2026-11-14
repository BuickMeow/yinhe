use super::controls::{SendSource, gain_at_y, gain_y, section, send_block};
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
    // 0dB（gain = 1）位于 DB_MIN..DB_MAX 的 60/66 处。
    assert!(approx(
        gain_y(1.0, TOP, BOTTOM),
        BOTTOM - 60.0 / 66.0 * 100.0
    ));
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

/// 自动高度：基础布局随总线数按发送区行高线性增长，默认总线数下
/// 与 `LayoutSettings::default().mix_height` 一致。
#[test]
fn auto_height_tracks_bus_rows() {
    let base = auto_panel_height(0);
    assert!((base - 200.0).abs() < 1e-3, "基础高度 {base}");
    assert!((auto_panel_height(1) - base).abs() < 1e-3);
    assert!((auto_panel_height(4) - (base + 3.0 * SEND_ROW_H)).abs() < 1e-3);
}

/// 实验：两个通道条共享同一 `mix_send_scroll` 滚动位置时，
/// 检查 ScrollArea 内部是否产生 ID 冲突（滚动条 / 拖拽捕获区）。
#[test]
fn send_blocks_shared_scroll_no_id_clash() {
    let ctx = egui::Context::default();
    let mut actions: Vec<MixAction> = Vec::new();
    // 两帧：滚动条显隐由上一帧内容尺寸决定。
    for _ in 0..2 {
        let output = ctx.run_ui(Default::default(), |ui| {
            ui.horizontal_top(|ui| {
                // 固定一行高：4 条总线内容溢出 → 滚动条出现（与通道条一致）。
                section(
                    ui,
                    SEND_ROW_H,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| send_block(ui, SendSource::Channel(0), &[], 4, &mut actions),
                );
                section(
                    ui,
                    SEND_ROW_H,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| send_block(ui, SendSource::Channel(1), &[], 4, &mut actions),
                );
            });
        });
        let mut clashes: Vec<String> = Vec::new();
        for cs in &output.shapes {
            if let egui::Shape::Text(t) = &cs.shape
                && t.galley.text().contains("of widget ID")
            {
                clashes.push(t.galley.text().to_owned());
            }
        }
        output.drop_without_applying_deltas();
        assert!(clashes.is_empty(), "send_block ID 冲突: {clashes:?}");
    }
}
