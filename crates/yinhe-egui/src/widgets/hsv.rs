use eframe::egui;

/// HSV 三滑块（色相 0-360° / 饱和度 / 明度）。
///
/// 基于 egui 自带 `ecolor::Hsva` 转换（无 alpha，不预乘）。
/// 任一滑块变化返回 `true`；调用方自行处理 undo 提交时机。
pub(crate) fn hsv_sliders(ui: &mut egui::Ui, color: &mut egui::Color32, slider_w: f32) -> bool {
    let mut hsv = egui::ecolor::Hsva::from(*color);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label("H");
        let mut h = hsv.h * 360.0;
        changed |= ui
            .add_sized(
                [slider_w, 20.0],
                egui::Slider::new(&mut h, 0.0..=360.0).show_value(false),
            )
            .changed();
        hsv.h = h / 360.0;
        ui.label("S");
        changed |= ui
            .add_sized(
                [slider_w, 20.0],
                egui::Slider::new(&mut hsv.s, 0.0..=1.0).show_value(false),
            )
            .changed();
        ui.label("V");
        changed |= ui
            .add_sized(
                [slider_w, 20.0],
                egui::Slider::new(&mut hsv.v, 0.0..=1.0).show_value(false),
            )
            .changed();
    });
    if changed {
        *color = hsv.into();
    }
    changed
}
