//! 通道属性面板的「默认乐器 / 鼓通道」区块。
//!
//! 默认乐器是通道级配置：同一 MIDI 通道的所有音轨共享，MIDI PC 事件优先。
//! 三个分量对应 Bank Select + Program Change：`Bank MSB`(CC0) /
//! `Bank LSB`(CC32) / `Program`。「鼓通道」是独立通道模式开关。

use std::sync::Arc;

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;

/// Program 的 GM 名字（鼓通道走鼓组表）。越界返回 None。
fn program_name(is_drum: bool, program: u8) -> Option<&'static str> {
    if is_drum {
        yinhe_types::gm::drum_kit_name(program)
    } else {
        yinhe_types::gm::melodic_name(program)
    }
}

/// 显示默认乐器（Bank MSB / Bank LSB / Program）+ 鼓通道开关。
/// 返回是否有改动（调用方据此通知音频引擎）。
pub(super) fn show(ui: &mut egui::Ui, doc: &mut Document, channel: u8) -> bool {
    let Some(current) = doc
        .data
        .model
        .channel_instruments
        .get(channel as usize)
        .copied()
        .flatten()
    else {
        // 跟随 MIDI 也允许关闭：用默认值初始化，再通过开关写入。
        return show_fields(ui, doc, channel, None);
    };
    show_fields(ui, doc, channel, Some(current))
}

fn show_fields(
    ui: &mut egui::Ui,
    doc: &mut Document,
    channel: u8,
    current: Option<yinhe_core::ChannelInstrument>,
) -> bool {
    let mut follow_midi = current.is_none();
    let mut next = current.unwrap_or_default();
    let mut changed = false;

    crate::widgets::rows::panel_row(ui, t!("track.default_instrument").as_ref(), |ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("track.instrument_follow_midi").as_ref())
                .color(crate::theme::text_label()),
        );
        if crate::widgets::switch::switch_compact(ui, &mut follow_midi).changed() {
            changed = true;
        }
    });

    ui.add_enabled_ui(!follow_midi, |ui| {
        for (label, field) in [
            ("track.bank_msb", &mut next.bank_msb),
            ("track.bank_lsb", &mut next.bank_lsb),
        ] {
            crate::widgets::rows::panel_row(ui, t!(label).as_ref(), |ui| {
                if ui
                    .add(
                        crate::widgets::numeric_input::decimal_drag_value(field)
                            .range(0..=127)
                            .speed(0.2),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
        }

        crate::widgets::rows::panel_row(ui, t!("track.program_label").as_ref(), |ui| {
            if ui
                .add(
                    crate::widgets::numeric_input::decimal_drag_value(&mut next.program)
                        .range(0..=127)
                        .speed(0.2),
                )
                .changed()
            {
                changed = true;
            }
            if let Some(name) = program_name(next.is_drum, next.program) {
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(name)
                        .color(crate::theme::text_label())
                        .small(),
                );
            }
        });
    });

    let mut drum_state = next.is_drum;
    crate::widgets::rows::panel_row(ui, t!("track.drum_channel").as_ref(), |ui| {
        if ui
            .push_id(("channel_drum_switch", channel), |ui| {
                crate::widgets::switch::switch_compact(ui, &mut drum_state)
            })
            .inner
            .changed()
        {
            changed = true;
        }
    });
    next.is_drum = drum_state;

    if !changed {
        return false;
    }
    let after = if follow_midi { None } else { Some(next) };
    if after == current {
        return false;
    }
    if let Some(slot) = Arc::make_mut(&mut doc.data.model)
        .channel_instruments
        .get_mut(channel as usize)
    {
        *slot = after;
        doc.data.bump_revision();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_name_uses_kind_specific_tables() {
        assert_eq!(program_name(false, 0), Some("Acoustic Grand Piano"));
        assert!(program_name(true, 0).is_some());
        assert_eq!(program_name(false, 200), None);
        assert_eq!(program_name(true, 200), None);
    }
}
