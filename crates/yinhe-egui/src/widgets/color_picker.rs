use eframe::egui;
use egui::ecolor::Hsva;

/// 自定义颜色编辑按钮：点击弹出调色板窗口。
///
/// 相比 egui 默认 `color_edit_button_srgba`，数值输入区支持
/// **RGBA / HSV 两种模式切换**（egui 默认只有 RGBA 数值输入，
/// HSV 只有图形面板）。HSV 图形面板复用 egui 公开 API。
///
/// 颜色视为不透明（alpha 恒 255，主题色/音轨色均如此），
/// 面板与数值区都不编辑 alpha。
pub(crate) fn color_edit_button(ui: &mut egui::Ui, color: &mut egui::Color32) -> egui::Response {
    let popup_id = ui.auto_id_with("color_picker_popup");

    let mut btn = ui.add(
        egui::Button::new("  ")
            .fill(*color)
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(100)))
            .min_size(egui::vec2(28.0, 20.0))
            .corner_radius(3.0),
    );

    egui::Popup::from_toggle_button_response(&btn)
        .id(popup_id)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().slider_width = 240.0;

            let mut hsva = Hsva::from(*color);
            let mut changed = false;

            // ── HSV 图形面板（色相条 + 饱和度/明度平面）──
            changed |= egui::widgets::color_picker::color_picker_hsva_2d(
                ui,
                &mut hsva,
                egui::widgets::color_picker::Alpha::Opaque,
            );

            ui.add_space(4.0);

            // ── 数值输入：RGBA / HSV 模式切换 ──
            let mode_id = popup_id.with("numeric_mode");
            let mut mode: u8 = ui.data_mut(|d| d.get_temp(mode_id)).unwrap_or(0);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut mode, 0, "RGBA");
                ui.selectable_value(&mut mode, 1, "HSV");
            });
            ui.data_mut(|d| d.insert_temp(mode_id, mode));

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                match mode {
                    0 => {
                        // RGBA：r/g/b 0-255（alpha 不透明，不编辑）
                        let mut rgba = hsva.to_srgba_unmultiplied();
                        for (i, name) in ["R", "G", "B"].iter().enumerate() {
                            ui.label(*name);
                            changed |= ui
                                .add(egui::DragValue::new(&mut rgba[i]).range(0..=255))
                                .changed();
                        }
                        if changed {
                            hsva = Hsva::from_srgba_unmultiplied(rgba);
                        }
                    }
                    _ => {
                        // HSV：H 0-360°、S/V 0-100%
                        ui.label("H");
                        let mut h = hsva.h * 360.0;
                        changed |= ui
                            .add(egui::DragValue::new(&mut h).range(0.0..=360.0).suffix("°"))
                            .changed();
                        hsva.h = h / 360.0;
                        ui.label("S");
                        let mut s = hsva.s * 100.0;
                        changed |= ui
                            .add(egui::DragValue::new(&mut s).range(0.0..=100.0).suffix("%"))
                            .changed();
                        hsva.s = s / 100.0;
                        ui.label("V");
                        let mut v = hsva.v * 100.0;
                        changed |= ui
                            .add(egui::DragValue::new(&mut v).range(0.0..=100.0).suffix("%"))
                            .changed();
                        hsva.v = v / 100.0;
                    }
                }
            });

            if changed {
                *color = egui::Color32::from(hsva);
                btn.mark_changed();
            }
        });

    btn
}
