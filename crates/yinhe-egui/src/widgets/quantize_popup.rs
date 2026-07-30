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
    ui.set_min_width(120.0);
    for preset in QuantizePreset::ALL {
        let is_sel = *preset == current;
        let btn = egui::Button::selectable(is_sel, preset.display_item(ppq)).frame(false);
        let resp = ui.add(btn);
        if resp.hovered() && !is_sel {
            ui.painter().rect_filled(resp.rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }
        if resp.clicked() {
            *pending = Some(*preset);
            ui.close();
        }
    }
    ui.separator();

    // ── 自定义时值 ──
    let is_frac = matches!(current, QuantizePreset::Fraction(_, _));
    let frac_btn = egui::Button::selectable(is_frac, t!("quantize.custom_fraction").as_ref()).frame(false);
    let frac_resp = ui.add(frac_btn);
    if frac_resp.hovered() && !is_frac {
        ui.painter().rect_filled(frac_resp.rect, 2.0, crate::theme::ROW_SELECTED_BG);
    }
    if frac_resp.clicked()
    {
        *pending = Some(QuantizePreset::Fraction(1, 1));
    }
    if let QuantizePreset::Fraction(num, den) = current {
        ui.horizontal(|ui| {
            ui.label(t!("quantize.numerator").as_ref());
            let mut n = num;
            if ui
                .add(crate::widgets::numeric_input::decimal_drag_value(&mut n).range(1..=9999).speed(0.5))
                .changed()
            {
                *pending = Some(QuantizePreset::Fraction(n, den));
            }
            ui.label(t!("quantize.denominator").as_ref());
            let mut d = den;
            if ui
                .add(crate::widgets::numeric_input::decimal_drag_value(&mut d).range(1..=9999).speed(0.5))
                .changed()
            {
                *pending = Some(QuantizePreset::Fraction(num, d.max(1)));
            }
        });
    }

    ui.separator();

    // ── 自定义Tick ──
    let is_abs = matches!(current, QuantizePreset::Absolute(_));
    let abs_btn = egui::Button::selectable(is_abs, t!("quantize.custom_tick").as_ref()).frame(false);
    let abs_resp = ui.add(abs_btn);
    if abs_resp.hovered() && !is_abs {
        ui.painter().rect_filled(abs_resp.rect, 2.0, crate::theme::ROW_SELECTED_BG);
    }
    if abs_resp.clicked()
    {
        *pending = Some(QuantizePreset::Absolute(1));
    }
    if let QuantizePreset::Absolute(n) = current {
        let mut val = n;
        if ui
            .add(crate::widgets::numeric_input::decimal_drag_value(&mut val).range(1..=99999).speed(0.5))
            .changed()
        {
            *pending = Some(QuantizePreset::Absolute(val));
        }
    }
}
