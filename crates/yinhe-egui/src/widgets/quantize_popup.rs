use eframe::egui;

use yinhe_editor_core::quantize::QuantizePreset;

/// Quantization popup menu: common presets + custom fraction + custom tick.
pub fn show(
    ui: &mut egui::Ui,
    ppq: u32,
    current: QuantizePreset,
    pending: &mut Option<QuantizePreset>,
) {
    ui.set_min_width(120.0);
    for preset in QuantizePreset::ALL {
        if ui
            .add(egui::Button::selectable(*preset == current, preset.display_item(ppq)))
            .clicked()
        {
            *pending = Some(*preset);
            ui.close();
        }
    }
    ui.separator();

    // ── 自定义时值 ──
    let is_frac = matches!(current, QuantizePreset::Fraction(_, _));
    if ui
        .add(egui::Button::selectable(is_frac, "自定义时值"))
        .clicked()
    {
        *pending = Some(QuantizePreset::Fraction(1, 1));
    }
    if let QuantizePreset::Fraction(num, den) = current {
        ui.horizontal(|ui| {
            ui.label("n:");
            let mut n = num;
            if ui
                .add(egui::DragValue::new(&mut n).range(1..=9999).speed(0.5))
                .changed()
            {
                *pending = Some(QuantizePreset::Fraction(n, den));
            }
            ui.label("d:");
            let mut d = den;
            if ui
                .add(egui::DragValue::new(&mut d).range(1..=9999).speed(0.5))
                .changed()
            {
                *pending = Some(QuantizePreset::Fraction(num, d.max(1)));
            }
        });
    }

    ui.separator();

    // ── 自定义Tick ──
    let is_abs = matches!(current, QuantizePreset::Absolute(_));
    if ui
        .add(egui::Button::selectable(is_abs, "自定义Tick"))
        .clicked()
    {
        *pending = Some(QuantizePreset::Absolute(1));
    }
    if let QuantizePreset::Absolute(n) = current {
        let mut val = n;
        if ui
            .add(egui::DragValue::new(&mut val).range(1..=99999).speed(0.5))
            .changed()
        {
            *pending = Some(QuantizePreset::Absolute(val));
        }
    }
}
