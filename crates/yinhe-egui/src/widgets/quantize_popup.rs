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
        let text = preset.display_item(ppq);
        let text_color = if is_sel {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let font = egui::FontId::proportional(13.0);
        let galley = ui.painter().layout_no_wrap(text.clone(), font, text_color);
        let btn_height = 22.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), btn_height),
            egui::Sense::click(),
        );

        // hover 背景（在文字下方）
        if response.hovered() && !is_sel {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }
        // 选中背景
        if is_sel {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }

        // 文字
        ui.painter().galley(
            rect.left_center() + egui::vec2(8.0, 0.0),
            galley,
            egui::Color32::WHITE,
        );

        if response.clicked() {
            *pending = Some(*preset);
            ui.close();
        }
    }
    ui.separator();

    // ── 自定义时值 ──
    let is_frac = matches!(current, QuantizePreset::Fraction(_, _));
    {
        let text = t!("quantize.custom_fraction").to_string();
        let text_color = if is_frac {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let font = egui::FontId::proportional(13.0);
        let galley = ui.painter().layout_no_wrap(text, font, text_color);
        let btn_height = 22.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), btn_height),
            egui::Sense::click(),
        );

        if response.hovered() && !is_frac {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }
        if is_frac {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }

        ui.painter().galley(
            rect.left_center() + egui::vec2(8.0, 0.0),
            galley,
            egui::Color32::WHITE,
        );

        if response.clicked() {
            *pending = Some(QuantizePreset::Fraction(1, 1));
        }
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
    {
        let text = t!("quantize.custom_tick").to_string();
        let text_color = if is_abs {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        let font = egui::FontId::proportional(13.0);
        let galley = ui.painter().layout_no_wrap(text, font, text_color);
        let btn_height = 22.0;
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), btn_height),
            egui::Sense::click(),
        );

        if response.hovered() && !is_abs {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }
        if is_abs {
            ui.painter().rect_filled(rect, 2.0, crate::theme::ROW_SELECTED_BG);
        }

        ui.painter().galley(
            rect.left_center() + egui::vec2(8.0, 0.0),
            galley,
            egui::Color32::WHITE,
        );

        if response.clicked() {
            *pending = Some(QuantizePreset::Absolute(1));
        }
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
