//! 听感开关：FL Studio 式小圆点。
//!
//! 亮 = 可听，灭 = 静音，金色 = 独奏。只有一个圆点，不显示文字；
//! 命中区固定 14px，窄轨道面板也放得下。

use eframe::egui;
use yinhe_types::Audibility;

/// 圆点半径（px）。
const DOT_R: f32 = 4.0;

/// 命中区边长（px）。比圆点大，方便点击。
pub(crate) const HIT: f32 = 14.0;

/// 在 `rect` 中心画听感圆点：可听 / 静音 / 独奏三态。
///
/// 只负责绘制与点击命中，切换听感由调用方写数据。
pub(crate) fn dot(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    audibility: Audibility,
    id: egui::Id,
) -> egui::Response {
    let resp = ui.interact(rect, id, egui::Sense::click());
    let color = match audibility {
        Audibility::Normal => crate::theme::accent_active(),
        Audibility::Soloed => crate::theme::warning_gold(),
        Audibility::Muted if resp.hovered() => crate::theme::text_secondary(),
        Audibility::Muted => crate::theme::text_disabled(),
    };
    ui.painter().circle_filled(rect.center(), DOT_R, color);
    if resp.has_focus() {
        super::a11y::paint_focus_ring(ui, rect, crate::scaling::scaled_font(ui.ctx(), 4.0));
    }
    resp
}

#[cfg(test)]
mod tests {
    use eframe::egui::{self, pos2, vec2};

    use super::*;

    fn click() -> bool {
        let ctx = egui::Context::default();
        let mut clicked = false;
        let mut run = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(200.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let rect = egui::Rect::from_center_size(pos2(50.0, 50.0), vec2(HIT, HIT));
                let resp = dot(
                    ui,
                    rect,
                    Audibility::Normal,
                    egui::Id::unique("audibility_dot_test"),
                );
                clicked |= resp.clicked();
            });
            output.textures_delta.clear();
        };
        let p = pos2(50.0, 50.0);
        let modifiers = egui::Modifiers::default();
        run(vec![]);
        run(vec![egui::Event::PointerMoved(p)]);
        run(vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers,
        }]);
        run(vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers,
        }]);
        clicked
    }

    /// 圆点虽小，命中区必须能接到点击。
    #[test]
    fn dot_click_registers() {
        assert!(click());
    }

    #[test]
    fn dot_secondary_click_registers() {
        let ctx = egui::Context::default();
        let mut secondary = false;
        let mut run = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(200.0, 200.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let rect = egui::Rect::from_center_size(pos2(50.0, 50.0), vec2(HIT, HIT));
                let resp = dot(
                    ui,
                    rect,
                    Audibility::Normal,
                    egui::Id::unique("audibility_dot_secondary"),
                );
                secondary |= resp.secondary_clicked();
            });
            output.textures_delta.clear();
        };
        let p = pos2(50.0, 50.0);
        let modifiers = egui::Modifiers::default();
        run(vec![]);
        run(vec![egui::Event::PointerMoved(p)]);
        run(vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers,
        }]);
        run(vec![egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers,
        }]);
        assert!(secondary, "右键应能被听感圆点接到");
    }
}
