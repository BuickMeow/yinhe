//! 音轨信息面板的「通道默认乐器 / 鼓通道」行。
//!
//! 默认乐器是通道级配置：同一 MIDI 通道的所有音轨共享，MIDI PC 事件优先。
//! 鼓通道用 Bank MSB 128 表示（GM 约定），勾选后该通道无 PC 时使用鼓组。

use eframe::egui;
use rust_i18n::t;

use yinhe_core::ChannelInstrument;
use yinhe_editor_core::document::Document;

fn is_drum(inst: Option<ChannelInstrument>) -> bool {
    inst.is_some_and(|i| i.bank_msb >= 128)
}

fn drum_kit(program: u8) -> ChannelInstrument {
    ChannelInstrument {
        bank_msb: 128,
        program: program.min(8),
    }
}

/// 非鼓通道的选项：跟随 MIDI + 128 个 GM 旋律音色 + 当前未知 bank。
fn melodic_options(current: Option<ChannelInstrument>) -> Vec<(Option<ChannelInstrument>, String)> {
    let mut out = vec![(None, t!("track.instrument_follow_midi").to_string())];
    for p in 0..128u8 {
        let name = yinhe_types::gm::melodic_name(p).unwrap_or("");
        out.push((
            Some(ChannelInstrument {
                bank_msb: 0,
                program: p,
            }),
            format!("{:03} {}", p + 1, name),
        ));
    }
    if let Some(c) = current
        && c.bank_msb != 0
        && c.bank_msb < 128
    {
        out.push((Some(c), format!("Bank {} PC {}", c.bank_msb, c.program + 1)));
    }
    out
}

/// 鼓通道的选项：GM 鼓组 + 当前未知鼓 kit。
fn drum_options(current: Option<ChannelInstrument>) -> Vec<(Option<ChannelInstrument>, String)> {
    let mut out = Vec::with_capacity(10);
    for p in 0..9u8 {
        let name = yinhe_types::gm::drum_kit_name(p).unwrap_or("");
        out.push((Some(drum_kit(p)), format!("D{:02} {}", p + 1, name)));
    }
    if let Some(c) = current
        && c.bank_msb >= 128
        && c.program >= 9
    {
        out.push((Some(c), format!("Bank {} PC {}", c.bank_msb, c.program + 1)));
    }
    out
}

/// 显示默认乐器 + 鼓通道开关。返回是否有改动（调用方据此通知音频引擎）。
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

    let mut selected = current;
    let mut instrument_changed = false;
    let drum = is_drum(current);
    crate::widgets::rows::panel_row(ui, t!("track.default_instrument").as_ref(), |ui| {
        let options = if drum {
            drum_options(current)
        } else {
            melodic_options(current)
        };
        if crate::widgets::combo::combo_select(
            ui,
            ("track_channel_instrument", channel),
            &mut selected,
            220.0,
            &options,
        ) {
            instrument_changed = true;
        }
    });

    // 组合框选择鼓组/旋律音色时，鼓通道开关随之联动。
    let mut next = selected;
    let mut drum_state = is_drum(next);
    let mut drum_changed = false;
    crate::widgets::rows::panel_row(ui, t!("track.drum_channel").as_ref(), |ui| {
        if crate::widgets::switch::switch(ui, &mut drum_state).changed() {
            drum_changed = true;
        }
    });
    if drum_changed {
        next = if drum_state {
            let kit = current
                .filter(|i| i.bank_msb >= 128)
                .map_or(0, |i| i.program);
            Some(drum_kit(kit))
        } else {
            // 显式保存「非鼓」，避免 A10 的旧存档迁移再次自动启用鼓。
            Some(ChannelInstrument {
                bank_msb: 0,
                program: 0,
            })
        };
    }

    let changed = instrument_changed || drum_changed;
    if !changed || next == current {
        return false;
    }
    let info = doc.edit.track_cache.info[track_idx].clone();
    let before = doc.capture_snapshot();
    if let Some(action) = doc.set_track_info(track_idx, info.name, info.port, info.channel, next) {
        doc.push_undo(action, t!("undo.edit_track_info").as_ref(), before);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drum_detection_uses_bank_128() {
        assert!(is_drum(Some(ChannelInstrument {
            bank_msb: 128,
            program: 0
        })));
        assert!(!is_drum(Some(ChannelInstrument {
            bank_msb: 0,
            program: 0
        })));
        assert!(!is_drum(None));
    }

    #[test]
    fn drum_kit_program_is_clamped() {
        assert_eq!(drum_kit(0).program, 0);
        assert_eq!(drum_kit(8).program, 8);
        assert_eq!(drum_kit(200).program, 8);
        assert_eq!(drum_kit(3).bank_msb, 128);
    }
}
