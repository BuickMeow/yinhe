//! 通道属性面板（MIX 右侧栏）。
//!
//! 与「音轨属性」的分工：
//! - 音轨属性：名称 / 端口 / 轨道→通道路由 / 颜色；
//! - 通道属性：通道级乐器、音量/声像、M/S、主输出、发送、插入。
//!
//! 通道由 `InfoContent::Channel` 显式指定；未指定时从选中轨推导（多选跨
//! 多个通道则不显示，避免随机取一条轨道造成误导）。

use eframe::egui;
use rust_i18n::t;

use yinhe_editor_core::document::Document;
use yinhe_mixer::StripParams;
use yinhe_types::MixerChannelId;

use crate::widgets::rows::{panel_row, section_header, value_row};

mod instrument;

/// 通道面板本帧改动。
#[derive(Default, Clone, Copy)]
pub(crate) struct ChannelInfoOut {
    /// 通道默认乐器等模型级改动（需重建音频模型）。
    pub model_changed: bool,
    /// 混音参数 / 主输出改动（需推送引擎）。
    pub mixer_changed: bool,
}

/// 从 `info_content` 或选中轨解析当前通道。
pub(crate) fn resolve_channel(
    doc: &Document,
    info_content: &Option<crate::right_panel::InfoContent>,
) -> Option<MixerChannelId> {
    if let Some(crate::right_panel::InfoContent::Channel(ch)) = info_content {
        return Some(*ch);
    }
    let mut channels = doc.edit.track_selected.iter().filter_map(|&t| {
        doc.data
            .model
            .tracks
            .get(t as usize)
            .and_then(|tr| tr.mixer_channel())
    });
    let first = channels.next()?;
    channels.all(|c| c == first).then_some(first)
}

/// 「通道属性」选项卡。
pub(crate) fn show_channel_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    info_content: &mut Option<crate::right_panel::InfoContent>,
) -> ChannelInfoOut {
    crate::right_panel::region_hint(ui, t!("hint.panel.info_channel"));
    let Some(ch) = resolve_channel(doc, info_content) else {
        crate::widgets::hint::empty_hint(ui, t!("panel.select_track_hint").as_ref());
        return ChannelInfoOut::default();
    };
    // 显式固定下来：后续拖动推子时 `track_selected` 变化不会让面板跳通道。
    *info_content = Some(crate::right_panel::InfoContent::Channel(ch));

    match ch {
        MixerChannelId::Midi(ch) => show_midi(ui, doc, ch),
        MixerChannelId::Audio(ch) => show_audio(ui, doc, ch),
        MixerChannelId::Bus(bus) => show_bus(ui, doc, bus),
        MixerChannelId::Master => show_master(ui, doc),
    }
}

fn show_midi(ui: &mut egui::Ui, doc: &mut Document, ch: u8) -> ChannelInfoOut {
    let mut out = ChannelInfoOut::default();
    let tracks = tracks_on_channel(doc, MixerChannelId::Midi(ch));
    header(ui, crate::mix::channel_label(ch), &tracks);

    let mut strip = doc.mixer.strip(ch);
    if strip_controls(ui, &mut strip)
        && let Some(slot) = doc.mixer_mut().channels.get_mut(ch as usize)
    {
        *slot = strip;
        out.mixer_changed = true;
    }

    section_header(ui, t!("track.output").as_ref());
    if output_selector(ui, doc, MixerChannelId::Midi(ch)) {
        out.mixer_changed = true;
    }

    // 默认乐器：数据模型目前只有 Port A 的 16 个槽位；其它端口不显示，
    // 避免把 B01 的设置误写到 A01。
    if ch < 16 {
        section_header(ui, t!("track.default_instrument").as_ref());
        out.model_changed |= instrument::show(ui, doc, ch);
    }

    section_header(ui, t!("mix.instrument").as_ref());
    show_instrument_summary(ui, doc, ch);

    section_header(ui, t!("mix.sends").as_ref());
    show_sends(ui, doc.mixer.sends.get(ch as usize).map(Vec::as_slice));

    section_header(ui, t!("channel.inserts").as_ref());
    show_inserts(
        ui,
        doc.mixer
            .channel_inserts
            .get(ch as usize)
            .map(Vec::as_slice),
    );
    out
}

fn show_audio(ui: &mut egui::Ui, doc: &mut Document, ch: u16) -> ChannelInfoOut {
    let mut out = ChannelInfoOut::default();
    let tracks = tracks_on_channel(doc, MixerChannelId::Audio(ch));
    header(ui, crate::mix::audio_label(ch), &tracks);

    let mut strip = doc.mixer.audio_strip(ch);
    if strip_controls(ui, &mut strip) {
        let mixer = doc.mixer_mut();
        if mixer.audio_channels.len() <= ch as usize {
            mixer
                .audio_channels
                .resize(ch as usize + 1, StripParams::default());
        }
        mixer.audio_channels[ch as usize] = strip;
        out.mixer_changed = true;
    }

    section_header(ui, t!("track.output").as_ref());
    if output_selector(ui, doc, MixerChannelId::Audio(ch)) {
        out.mixer_changed = true;
    }

    section_header(ui, t!("mix.sends").as_ref());
    show_sends(
        ui,
        doc.mixer.audio_sends.get(ch as usize).map(Vec::as_slice),
    );

    section_header(ui, t!("channel.inserts").as_ref());
    show_inserts(
        ui,
        doc.mixer.audio_inserts.get(ch as usize).map(Vec::as_slice),
    );
    out
}

fn show_bus(ui: &mut egui::Ui, doc: &mut Document, bus: yinhe_types::BusId) -> ChannelInfoOut {
    let mut out = ChannelInfoOut::default();
    header(ui, format!("BUS {}", bus.index() + 1), &[]);

    let Some(index) = doc.mixer.bus_index(bus) else {
        return out;
    };
    let mut strip = doc.mixer.buses[index];
    if strip_controls(ui, &mut strip)
        && let Some(slot) = doc.mixer_mut().buses.get_mut(index)
    {
        *slot = strip;
        out.mixer_changed = true;
    }

    let sends = doc.mixer.bus_sends.get(index).cloned().unwrap_or_default();
    let inserts = doc
        .mixer
        .bus_inserts
        .get(index)
        .cloned()
        .unwrap_or_default();
    section_header(ui, t!("mix.sends").as_ref());
    show_sends(ui, Some(&sends));

    section_header(ui, t!("channel.inserts").as_ref());
    show_inserts(ui, Some(&inserts));
    out
}

fn show_master(ui: &mut egui::Ui, doc: &mut Document) -> ChannelInfoOut {
    let mut out = ChannelInfoOut::default();
    header(ui, t!("mix.master").to_string(), &[]);

    let mut gain_db = crate::widgets::meter::gain_to_db(doc.mixer.master.gain);
    panel_row(ui, t!("track.gain").as_ref(), |ui| {
        ui.add(
            crate::widgets::numeric_input::decimal_drag_value(&mut gain_db)
                .range(crate::widgets::meter::DB_MIN..=crate::widgets::meter::DB_MAX)
                .speed(0.1)
                .suffix(" dB"),
        );
    });
    let gain = crate::widgets::meter::db_to_gain(gain_db);
    if (gain - doc.mixer.master.gain).abs() > f32::EPSILON {
        doc.mixer_mut().master.gain = gain;
        out.mixer_changed = true;
    }

    section_header(ui, t!("channel.inserts").as_ref());
    show_inserts(ui, Some(doc.mixer.master_inserts.as_slice()));
    out
}

fn header(ui: &mut egui::Ui, title: String, tracks: &[u16]) {
    section_header(ui, &title);
    if !tracks.is_empty() {
        value_row(
            ui,
            String::new(),
            t!("ca.tracks", n = tracks.len()).to_string(),
        );
    }
}

/// 通道条参数行：音量 / 声像 / M / S。返回是否有改动。
fn strip_controls(ui: &mut egui::Ui, strip: &mut StripParams) -> bool {
    let before = *strip;
    let mut gain_db = crate::widgets::meter::gain_to_db(strip.gain);
    panel_row(ui, t!("track.gain").as_ref(), |ui| {
        ui.add(
            crate::widgets::numeric_input::decimal_drag_value(&mut gain_db)
                .range(crate::widgets::meter::DB_MIN..=crate::widgets::meter::DB_MAX)
                .speed(0.1)
                .suffix(" dB"),
        );
    });
    strip.gain = crate::widgets::meter::db_to_gain(gain_db);

    panel_row(ui, t!("channel.pan").as_ref(), |ui| {
        ui.add(
            crate::widgets::numeric_input::decimal_drag_value(&mut strip.pan)
                .range(-1.0..=1.0)
                .speed(0.01),
        );
    });
    panel_row(ui, t!("mix.mute").as_ref(), |ui| {
        crate::widgets::switch::switch_compact(ui, &mut strip.mute);
    });
    panel_row(ui, t!("mix.solo").as_ref(), |ui| {
        crate::widgets::switch::switch_compact(ui, &mut strip.solo);
    });
    *strip != before
}

fn tracks_on_channel(doc: &Document, ch: MixerChannelId) -> Vec<u16> {
    doc.data
        .model
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, tr)| tr.mixer_channel() == Some(ch))
        .map(|(i, _)| i as u16)
        .collect()
}

/// 通道主输出（Master / BUS n）。返回是否改动。
fn output_selector(ui: &mut egui::Ui, doc: &mut Document, ch: MixerChannelId) -> bool {
    let bus_ids = doc.mixer.bus_ids.clone();
    let bus_options: Vec<(usize, String)> = std::iter::once((0, "Master".to_string()))
        .chain(
            bus_ids
                .iter()
                .enumerate()
                .map(|(i, id)| (i + 1, format!("BUS {}", id.index() + 1))),
        )
        .collect();
    let (outputs, index) = match ch {
        MixerChannelId::Midi(ch) => (&mut doc.mixer.midi_outputs, ch as usize),
        MixerChannelId::Audio(ch) => (&mut doc.mixer.audio_outputs, ch as usize),
        _ => return false,
    };
    if outputs.len() <= index {
        outputs.resize(index + 1, None);
    }
    let mut sel = outputs[index]
        .and_then(|id| bus_ids.iter().position(|&x| x == id))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut changed = false;
    panel_row(ui, t!("track.output").as_ref(), |ui| {
        if crate::widgets::combo::combo_select(
            ui,
            "channel_main_output",
            &mut sel,
            ui.available_width().max(1.0),
            &bus_options,
        ) {
            changed = true;
        }
    });
    if changed {
        outputs[index] = if sel == 0 {
            None
        } else {
            bus_ids.get(sel - 1).copied()
        };
        doc.mark_mixer_dirty();
    }
    changed
}

fn show_instrument_summary(ui: &mut egui::Ui, doc: &Document, ch: u8) {
    let chain = doc
        .mixer
        .instruments
        .get(ch as usize)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if chain.is_empty() {
        return;
    }
    for ins in chain {
        value_row(ui, ins.name.clone(), "");
    }
}

fn show_sends(ui: &mut egui::Ui, sends: Option<&[yinhe_mixer::SendParams]>) {
    let sends = sends.unwrap_or(&[]);
    if sends.is_empty() {
        value_row(ui, String::new(), t!("mix.no_sends").to_string());
        return;
    }
    for send in sends {
        let mode = if send.pre_fader {
            t!("mix.pre_fader")
        } else {
            t!("mix.post_fader")
        };
        value_row(
            ui,
            format!("BUS {}", send.bus.index() + 1),
            format!("{:.0}% · {}", send.amount * 100.0, mode),
        );
    }
}

fn show_inserts(ui: &mut egui::Ui, inserts: Option<&[yinhe_mixer::InsertRef]>) {
    let inserts = inserts.unwrap_or(&[]);
    if inserts.is_empty() {
        return;
    }
    for ins in inserts {
        value_row(ui, ins.name.clone(), "");
    }
}
