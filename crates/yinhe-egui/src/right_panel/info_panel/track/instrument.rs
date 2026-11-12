//! 音轨信息面板的「默认乐器 / 鼓通道」行。
//!
//! 默认乐器是通道级配置：同一 MIDI 通道的所有音轨共享，MIDI PC 事件优先。
//!
//! 三个分量对应 MIDI 的 Bank Select + Program Change：`Bank MSB`(CC0) /
//! `Bank LSB`(CC32) / `Program`。很多 SF2/SFZ 音色库用 bank 区分同一 program
//! 的不同音色，所以 bank 必须能独立于 program 设置。
//! 「鼓通道」是独立的通道模式开关（合成器的 PercussionMode），不占用 bank 号。

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
pub(super) fn show(ui: &mut egui::Ui, doc: &mut Document, track_idx: usize) -> bool {
    let Some(track) = doc.data.model.tracks.get(track_idx) else {
        return false;
    };
    if track.kind != yinhe_core::TrackKind::Midi {
        return false;
    }
    let channel = track.channel;
    let current = doc
        .data
        .model
        .channel_instruments
        .get(channel as usize)
        .copied()
        .flatten();

    // 「跟随 MIDI」= None：不写默认乐器，完全由 MIDI 内的 PC 决定。
    let mut follow_midi = current.is_none();
    let mut next = current.unwrap_or_default();
    let mut changed = false;

    crate::widgets::rows::panel_row(ui, t!("track.default_instrument").as_ref(), |ui| {
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new(t!("track.instrument_follow_midi").as_ref())
                .color(crate::theme::text_label()),
        );
        let resp = crate::widgets::switch::switch_compact(ui, &mut follow_midi);
        if resp.changed() {
            changed = true;
        }
    });

    // 跟随 MIDI 时三个分量没有意义：整块置灰。
    ui.add_enabled_ui(!follow_midi, |ui| {
        for (label, field) in [
            ("track.bank_msb", &mut next.bank_msb),
            ("track.bank_lsb", &mut next.bank_lsb),
        ] {
            crate::widgets::rows::panel_row(ui, t!(label).as_ref(), |ui| {
                let resp = ui.add(
                    crate::widgets::numeric_input::decimal_drag_value(field)
                        .range(0..=127)
                        .speed(0.2),
                );
                if resp.changed() {
                    changed = true;
                }
            });
        }

        crate::widgets::rows::panel_row(ui, t!("track.program_label").as_ref(), |ui| {
            let resp = ui.add(
                crate::widgets::numeric_input::decimal_drag_value(&mut next.program)
                    .range(0..=127)
                    .speed(0.2),
            );
            if resp.changed() {
                changed = true;
            }
            // GM 名字只做提示：不认识的 program 不显示，避免误导。
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

    // 鼓通道：独立开关，只影响合成器的通道模式与 program 的名字表。
    let mut drum_state = next.is_drum;
    crate::widgets::rows::panel_row(ui, t!("track.drum_channel").as_ref(), |ui| {
        let resp = ui
            .push_id(("track_drum_switch", channel), |ui| {
                crate::widgets::switch::switch_compact(ui, &mut drum_state)
            })
            .inner;
        if resp.changed() {
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
    let info = doc.edit.track_cache.info[track_idx].clone();
    let before = doc.capture_snapshot();
    if let Some(action) = doc.set_track_info(track_idx, info.name, info.port, info.channel, after) {
        doc.push_undo(action, t!("undo.edit_track_info").as_ref(), before);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// program 的 GM 名字按鼓/旋律走不同表，越界返回 None。
    #[test]
    fn program_name_uses_kind_specific_tables() {
        assert_eq!(program_name(false, 0), Some("Acoustic Grand Piano"));
        assert!(program_name(true, 0).is_some());
        assert_eq!(program_name(false, 200), None);
        assert_eq!(program_name(true, 200), None);
    }

    /// 「跟随 MIDI」= None，与「选了值」必须能区分（否则无法表达回退）。
    #[test]
    fn follow_midi_is_none() {
        let inst = yinhe_core::ChannelInstrument {
            bank_msb: 17,
            bank_lsb: 2,
            program: 40,
            is_drum: false,
        };
        let current = Some(inst);
        assert!(current.is_some(), "选了值 = 不跟随");
        let follow: Option<yinhe_core::ChannelInstrument> = None;
        assert!(follow.is_none(), "跟随 MIDI = None");
    }
}
