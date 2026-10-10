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

    // ── 自定义时值（分子 / 分母，各一个步进器）──
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
        let mut n = num;
        labeled_stepper(ui, t!("quantize.numerator").as_ref(), |ui| {
            crate::widgets::stepper::stepper(&mut n)
                .range(1u32..=9999)
                .step(1.0)
                .decimals(0)
                .width(stepper_w(ui))
                .show(ui);
        });
        let mut d = den;
        labeled_stepper(ui, t!("quantize.denominator").as_ref(), |ui| {
            crate::widgets::stepper::stepper(&mut d)
                .range(1u32..=9999)
                .step(1.0)
                .decimals(0)
                .width(stepper_w(ui))
                .show(ui);
        });
        if n != num {
            *pending = Some(QuantizePreset::Fraction(n.max(1), den));
        }
        if d != den {
            *pending = Some(QuantizePreset::Fraction(num, d.max(1)));
        }
    }

    ui.separator();

    // ── 自定义 Tick ──
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
        labeled_stepper(ui, t!("quantize.custom_tick").as_ref(), |ui| {
            crate::widgets::stepper::stepper(&mut val)
                .range(1u32..=99999)
                .step(1.0)
                .decimals(0)
                .width(stepper_w(ui))
                .show(ui);
        });
        if val != n {
            *pending = Some(QuantizePreset::Absolute(val.max(1)));
        }
    }
}

fn stepper_w(ui: &egui::Ui) -> f32 {
    crate::scaling::scaled_font(ui.ctx(), 96.0)
}

/// 标签 + 控件行：先固定行高、标签列等宽再落控件，保证垂直居中对齐
/// （与 `rows::form_row` 同法，但标签用正常字号——自定义时值不写小字）。
fn labeled_stepper(ui: &mut egui::Ui, label: &str, add_control: impl FnOnce(&mut egui::Ui)) {
    let h = super::control::h(ui.ctx());
    let w = crate::scaling::scaled_font(ui.ctx(), 56.0);
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), h),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(w, h),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(label);
                },
            );
            add_control(ui);
        },
    );
}
