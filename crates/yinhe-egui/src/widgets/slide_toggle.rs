//! 按住 M/S 后滑过多个轨道：所有经过的轨道统一设置为按下时的目标状态。
//!
//! 只处理指针按压/拖动，不改变按钮的视觉反馈；调用方仍用 `Response` 决定配色。

use eframe::egui;

/// `kind` 用于区分同一视图里的 M 与 S（调用方自定义，如 0 = M、1 = S）。
///
/// 返回 `Some(target)` 表示本帧应把该轨道开关设为 `target`：
/// - 按下瞬间：`target = !current`，同时记录本次滑动的目标状态；
/// - 按住滑过其他同 `kind` 按钮：返回同一个 `target`（幂等）。
pub(crate) fn slide_toggle(
    ui: &egui::Ui,
    id: egui::Id,
    kind: u8,
    rect: egui::Rect,
    current: bool,
) -> Option<bool> {
    if !ui.input(|i| i.pointer.primary_down()) {
        ui.data_mut(|d| d.remove::<(u8, bool)>(id));
        return None;
    }
    if ui.input(|i| i.pointer.primary_pressed())
        && ui.is_rect_visible(rect)
        && ui.rect_contains_pointer(rect)
    {
        let target = !current;
        ui.data_mut(|d| d.insert_temp(id, (kind, target)));
        return Some(target);
    }
    match ui.data(|d| d.get_temp::<(u8, bool)>(id)) {
        Some((k, target)) if k == kind && ui.rect_contains_pointer(rect) => Some(target),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut probe: impl FnMut(&mut egui::Ui, egui::Rect, egui::Rect) -> (Option<bool>, Option<bool>),
    ) -> (Option<bool>, Option<bool>) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(200.0, 120.0),
            )),
            ..Default::default()
        };
        input.events = events;
        let mut result = (None, None);
        let output = ctx.run_ui(input, |ui| {
            let a = egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(20.0, 20.0));
            let b = egui::Rect::from_min_size(egui::pos2(10.0, 60.0), egui::vec2(20.0, 20.0));
            result = probe(ui, a, b);
        });
        output.drop_without_applying_deltas();
        result
    }

    #[test]
    fn drag_sets_same_target_across_buttons() {
        let ctx = egui::Context::default();
        let a_center = egui::pos2(20.0, 20.0);
        let b_center = egui::pos2(20.0, 70.0);
        let id = egui::Id::unique("slide_toggle_test");

        let press = vec![egui::Event::PointerButton {
            pos: a_center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        }];
        let (a, _) = frame(&ctx, press, |ui, a, b| {
            let _ = ui.allocate_rect(a, egui::Sense::click());
            let _ = ui.allocate_rect(b, egui::Sense::click());
            (
                slide_toggle(ui, id, 0, a, false),
                slide_toggle(ui, id, 0, b, false),
            )
        });
        assert_eq!(a, Some(true), "按下 M 应设置为按下瞬间的目标状态");

        let (_, b) = frame(
            &ctx,
            vec![egui::Event::PointerMoved(b_center)],
            |ui, a, b| {
                let _ = ui.allocate_rect(a, egui::Sense::click());
                let _ = ui.allocate_rect(b, egui::Sense::click());
                (
                    slide_toggle(ui, id, 0, a, false),
                    slide_toggle(ui, id, 0, b, false),
                )
            },
        );
        assert_eq!(b, Some(true), "滑到另一轨应沿用同一目标状态");

        let (released, _) = frame(
            &ctx,
            vec![egui::Event::PointerButton {
                pos: b_center,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            |ui, a, b| {
                let _ = ui.allocate_rect(a, egui::Sense::click());
                let _ = ui.allocate_rect(b, egui::Sense::click());
                (
                    slide_toggle(ui, id, 0, a, true),
                    slide_toggle(ui, id, 0, b, true),
                )
            },
        );
        assert_eq!(released, None);
    }
}
