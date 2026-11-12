//! 底部按钮区几何回归：按钮必须与内容区**右下角同时贴齐**。
//!
//! 曾经的现象：未保存弹窗的按钮右侧贴齐、下方却空出一大截（或反之）。
//! 根因是 `bottom_up` 布局里按钮行被顶到预留区顶部，行底没落在内容区底缘。
//!
//! 这里测的是「按钮到内容区（Frame 内边距之内）右下角的距离」：
//! 两者都应为 0——外层的 `inner_margin` 已经统一决定了窗口边距，
//! 按钮只要贴住内容区右下角，四边留白就必然一致。

use eframe::egui;

/// 返回 `(主按钮 rect, 内容区可用矩形)`。
fn measure(size: egui::Vec2, margin: f32) -> (egui::Rect, egui::Rect) {
    let button = std::rc::Rc::new(std::cell::RefCell::new(egui::Rect::NOTHING));
    let inner = std::rc::Rc::new(std::cell::RefCell::new(egui::Rect::NOTHING));
    let b_out = button.clone();
    let i_out = inner.clone();

    let mut harness = egui_kittest::Harness::builder()
        .with_size(size)
        .build_ui(move |ui| {
            egui::Frame::new()
                .inner_margin(egui::Margin::same(margin as i8))
                .show(ui, |ui| {
                    *i_out.borrow_mut() = ui.available_rect_before_wrap();
                    let btn_zone = crate::chrome::dialog_buttons::btn_zone_h(ui.ctx());
                    crate::chrome::dialog::content_with_bottom_buttons(
                        ui,
                        btn_zone,
                        |ui| {
                            ui.label("message");
                        },
                        |ui| {
                            use crate::chrome::dialog_buttons::{DialogButton, dialog_button_row};
                            let h = crate::scaling::scaled_font(ui.ctx(), 30.0);
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), h),
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    // 与 dialog_button_row 相同的顺序：主按钮最靠右。
                                    let resp = crate::chrome::dialog_buttons::dialog_button(
                                        ui,
                                        &DialogButton::primary("Save"),
                                    );
                                    *b_out.borrow_mut() = resp.rect;
                                    ui.add_space(8.0);
                                    let _ = crate::chrome::dialog_buttons::dialog_button(
                                        ui,
                                        &DialogButton::secondary("Cancel"),
                                    );
                                },
                            );
                            let _ = dialog_button_row;
                        },
                    );
                });
        });
    harness.step();
    (*button.borrow(), *inner.borrow())
}

/// 按钮到内容区右下角的距离都必须为 0：贴右下角。
#[test]
fn bottom_buttons_flush_with_content_corner() {
    for (w, h) in [(340.0, 130.0), (480.0, 300.0), (760.0, 620.0)] {
        let (button, inner) = measure(egui::vec2(w, h), 12.0);
        assert!(button.is_finite(), "{w}x{h}: 未取到按钮 rect");
        let right = inner.max.x - button.max.x;
        let bottom = inner.max.y - button.max.y;
        assert!(
            right.abs() <= 1.0,
            "{w}x{h}: 按钮未贴右，右边距 {right:.1}（按钮 {button:?} / 内容区 {inner:?}）"
        );
        assert!(
            bottom.abs() <= 1.0,
            "{w}x{h}: 按钮未贴底，下边距 {bottom:.1}（按钮 {button:?} / 内容区 {inner:?}）"
        );
    }
}

/// 任意窗口尺寸下，右下边距必须相等（这正是用户看到的问题）。
#[test]
fn bottom_button_margins_match_each_other() {
    for (w, h) in [(340.0, 130.0), (360.0, 160.0), (760.0, 620.0)] {
        let (button, inner) = measure(egui::vec2(w, h), 12.0);
        let right = inner.max.x - button.max.x;
        let bottom = inner.max.y - button.max.y;
        assert!(
            (right - bottom).abs() <= 1.0,
            "{w}x{h}: 右边距 {right:.1} ≠ 下边距 {bottom:.1}"
        );
    }
}
