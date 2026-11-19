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

/// C 的 SEND 聚焦模式固定一行，不随通道数增长。
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
    let ch1 = doc.track_channel_id(1).expect("track 1 channel");
    assert!(!super::strip_selected_doc(&doc, ch1));

    doc.edit.track_selected.insert(1);
    assert!(super::strip_selected_doc(&doc, ch1));
    let ch2 = doc.track_channel_id(2).expect("track 2 channel");
    assert!(!super::strip_selected_doc(&doc, ch2));
}

/// 回归：横向 ScrollArea 内点击卡片**内容区空白**（不挪系统鼠标）必须命中
/// 整卡点击层。色带只有 20px，用户点的是卡片其它位置。
#[test]
fn card_body_click_hits_inside_horizontal_scroll_area() {
    use eframe::egui::{self, pos2, vec2};

    let ctx = egui::Context::default();
    let mut clicked = 0usize;
    let mut run = |events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            egui::ScrollArea::horizontal()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for i in 0..17 {
                            let (_rect, card) = super::controls::strip_frame(
                                ui,
                                i,
                                egui::Color32::RED,
                                "A01",
                                700.0,
                                false,
                                |ui, _| {
                                    ui.label("content");
                                },
                            );
                            if card.clicked() {
                                clicked += 1;
                            }
                        }
                    });
                });
        });
        output.textures_delta.clear();
    };

    let pos = pos2(40.0, 120.0);
    let modifiers = egui::Modifiers::default();
    run(vec![]);
    run(vec![egui::Event::PointerMoved(pos)]);
    run(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers,
    }]);
    run(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    }]);

    assert_eq!(clicked, 1, "单击卡片内容区应命中整卡点击层");
}

/// 回归：电平表自带 click_and_drag（拖推子），会挡住整卡点击层；
/// 单击电平表必须补发通道选中动作。
#[test]
fn meter_click_requests_channel_selection() {
    use eframe::egui::{self, pos2, vec2};

    let ctx = egui::Context::default();
    let mut actions = Vec::new();
    let mut run = |events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                pos2(0.0, 0.0),
                vec2(1000.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            super::controls::gain_meter(
                ui,
                1.0,
                (0.0, 0.0),
                (0.0, 0.0),
                120.0,
                ChannelId::Master,
                &mut actions,
            );
        });
        output.textures_delta.clear();
    };

    let pos = pos2(10.0, 60.0);
    let modifiers = egui::Modifiers::default();
    run(vec![]);
    run(vec![egui::Event::PointerMoved(pos)]);
    run(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers,
    }]);
    run(vec![egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers,
    }]);

    assert!(
        actions.iter().any(|a| matches!(
            a,
            MixAction::SelectChannel {
                channel: ChannelId::Master
            }
        )),
        "单击电平表应请求选中 Master 通道"
    );
}

/// 回归：SEND 选中=源。选中 A01 后，A02 条的旋钮读的是 A01→A02；
/// A01 自己、Master 源、未选源都不显示旋钮；反向读取不能串。
#[test]
fn send_knob_reads_selected_source_to_target() {
    let a = ChannelId::Ap(ApChannelId::new(ApGroupId(0), 0));
    let b = ChannelId::Ap(ApChannelId::new(ApGroupId(1), 0));
    let mut mixer = MixerParams::default();
    assert!(mixer.set_channel_send(a, b, 0.5, false));

    let send = super::focus_send_for(&mixer, Some(a), b);
    assert_eq!(send.source, Some(a));
    assert_eq!(send.target, b);
    assert!((send.amount - 0.5).abs() < 1e-6);
    assert!(super::controls::send_row_visible(&send));

    let own = super::focus_send_for(&mixer, Some(a), a);
    assert!(!super::controls::send_row_visible(&own));
    let master = super::focus_send_for(&mixer, Some(ChannelId::Master), b);
    assert!(!super::controls::send_row_visible(&master));
    let none = super::focus_send_for(&mixer, None, b);
    assert!(!super::controls::send_row_visible(&none));

    let reverse = super::focus_send_for(&mixer, Some(b), a);
    assert_eq!(reverse.amount, 0.0, "A01→A02 不能读成 A02→A01");

    // Master 作为目标：选中源 A01 时，Master 条显示 A01 → Master。
    assert!(mixer.set_channel_send(a, ChannelId::Master, 0.25, false));
    let to_master = super::focus_send_for(&mixer, Some(a), ChannelId::Master);
    assert!(super::controls::send_row_visible(&to_master));
    assert!((to_master.amount - 0.25).abs() < 1e-6);
}
