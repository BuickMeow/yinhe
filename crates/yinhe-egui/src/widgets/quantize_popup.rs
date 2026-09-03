use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::quantize::QuantizePreset;

/// Quantization popup menu: common presets + custom fraction + custom tick.
pub fn show(
    ui: &mut egui::Ui,
    ppq: u32,
    current: QuantizePreset,
    pending: &mut Option<QuantizePreset>,
) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.spacing_mut().item_spacing.x = 4.0;
    ui.set_min_width(200.0);
    ui.set_max_width(200.0);
    for preset in QuantizePreset::ALL {
        if ui
            .add(crate::widgets::menu::menu_item_button(
                ui,
                *preset == current,
                preset.display_item(ppq),
            ))
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
        .add(crate::widgets::menu::menu_item_button(
            ui,
            is_frac,
            t!("quantize.custom_fraction"),
        ))
        .clicked()
    {
        *pending = Some(QuantizePreset::Fraction(1, 1));
    }
    if let QuantizePreset::Fraction(num, den) = current {
        ui.horizontal(|ui| {
            ui.label(t!("quantize.numerator").as_ref());
            let mut n = num;
            if ui
                .add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut n)
                        .range(1..=9999)
                        .speed(0.5),
                )
                .changed()
            {
                *pending = Some(QuantizePreset::Fraction(n, den));
            }
            ui.label(t!("quantize.denominator").as_ref());
            let mut d = den;
            if ui
                .add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut d)
                        .range(1..=9999)
                        .speed(0.5),
                )
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
        .add(crate::widgets::menu::menu_item_button(
            ui,
            is_abs,
            t!("quantize.custom_tick"),
        ))
        .clicked()
    {
        *pending = Some(QuantizePreset::Absolute(1));
    }
    if let QuantizePreset::Absolute(n) = current {
        let mut val = n;
        if ui
            .add(
                crate::widgets::numeric_input::decimal_drag_value(&mut val)
                    .range(1..=99999)
                    .speed(0.5),
            )
            .changed()
        {
            *pending = Some(QuantizePreset::Absolute(val));
        }
    }
}
