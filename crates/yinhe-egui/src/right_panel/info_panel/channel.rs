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
use yinhe_types::{ChannelId, ChannelSource, RackId, RackOutputMode};

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
) -> Option<ChannelId> {
    if let Some(crate::right_panel::InfoContent::Channel(ch)) = info_content {
        return Some(*ch);
    }
    let mut channels = doc
        .edit
        .track_selected
        .iter()
        .filter_map(|&t| doc.track_channel_id(t as usize));
    let first = channels.next()?;
    channels.all(|c| c == first).then_some(first)
}

/// 「通道属性」选项卡。
pub(crate) fn show_channel_info(
    ui: &mut egui::Ui,
    doc: &mut Document,
    info_content: &mut Option<crate::right_panel::InfoContent>,
    per_track_supported: bool,
) -> ChannelInfoOut {
    crate::right_panel::region_hint(ui, t!("hint.panel.info_channel"));
    let Some(ch) = resolve_channel(doc, info_content) else {
        crate::widgets::hint::empty_hint(ui, t!("panel.select_track_hint").as_ref());
        return ChannelInfoOut::default();
    };
    // 显式固定下来：后续拖动推子时 `track_selected` 变化不会让面板跳通道。
    *info_content = Some(crate::right_panel::InfoContent::Channel(ch));

    if ch == ChannelId::Master {
        show_master(ui, doc)
    } else {
        show_channel(ui, doc, ch, per_track_supported)
    }
}

/// 通道标题：AP = A01；音频 LC = Audio-XX；每轨 LC = 轨道号 + 名称。
fn channel_title(doc: &Document, ch: ChannelId) -> String {
    match ch {
        ChannelId::Ap(ap) => crate::mix::channel_label(ap.group.0 as u8),
        ChannelId::Master => t!("mix.master").to_string(),
        ChannelId::Lc(lc) => match doc.mixer.source_of(ch) {
            Some(ChannelSource::Audio(ach)) => crate::mix::audio_label(ach),
            Some(ChannelSource::Track(t)) => doc
                .edit
                .track_cache
                .info
                .get(t as usize)
                .map(|ti| format!("{:03} {}", ti.index, ti.name))
                .unwrap_or_else(|| format!("{:03}", t)),
            _ => crate::mix::lc_label(doc.mixer.lc_display_map().get(&lc).copied().unwrap_or(0)),
        },
    }
}

/// 该通道绑定的 MIDI 源通道（AP / OwnLc 轨有；音频轨没有）。
fn midi_channel_of(doc: &Document, ch: ChannelId) -> Option<u8> {
    match ch {
        ChannelId::Ap(ap) => Some(ap.group.0 as u8),
        ChannelId::Lc(_) => match doc.mixer.source_of(ch) {
            Some(ChannelSource::Track(t)) => doc
                .data
                .model
                .tracks
                .get(t as usize)
                .map(|t| t.global_channel()),
            _ => None,
        },
        ChannelId::Master => None,
    }
}

fn show_channel(
    ui: &mut egui::Ui,
    doc: &mut Document,
    ch: ChannelId,
    per_track_supported: bool,
) -> ChannelInfoOut {
    let mut out = ChannelInfoOut::default();
    let tracks = tracks_on_channel(doc, ch);
    header(ui, channel_title(doc, ch), &tracks);

    let mut strip = doc.mixer.node_strip(ch);
    if strip_controls(ui, &mut strip) {
        doc.mixer_mut().set_node_strip(ch, strip);
        out.mixer_changed = true;
    }

    section_header(ui, t!("track.output").as_ref());
    if output_selector(ui, doc, ch) {
        out.mixer_changed = true;
    }

    if let Some(global) = midi_channel_of(doc, ch) {
        section_header(ui, t!("rack.output").as_ref());
        let rack = RackId::from_midi_channel(global);
        let plugin = !doc.mixer.rack_instruments(rack).is_empty();
        if plugin {
            value_row(
                ui,
                t!("rack.output.single").to_string(),
                "Main · 2ch".to_string(),
            );
        } else if !per_track_supported {
            value_row(
                ui,
                t!("rack.output.shared").to_string(),
                t!("rack.output.requires_yinhe_cpu").to_string(),
            );
        } else {
            let current = doc.rack_output_mode(global);
            let mut sel = usize::from(current == RackOutputMode::PerTrack);
            let options = [
                (0usize, t!("rack.output.shared").to_string()),
                (1usize, t!("rack.output.per_track").to_string()),
            ];
            let mut changed = false;
            panel_row(ui, "", |ui| {
                changed = crate::widgets::combo::combo_select(
                    ui,
                    "rack_output_mode",
                    &mut sel,
                    ui.available_width().max(1.0),
                    &options,
                );
            });
            if changed {
                let mode = if sel == 0 {
                    RackOutputMode::Shared
                } else {
                    RackOutputMode::PerTrack
                };
                if doc.set_rack_output_mode(global, mode) {
                    out.model_changed = true;
                    out.mixer_changed = true;
                }
            }
        }

        if global < 16 {
            section_header(ui, t!("track.default_instrument").as_ref());
            out.model_changed |= instrument::show(ui, doc, global);
        }

        section_header(ui, t!("mix.instrument").as_ref());
        show_instrument_summary(ui, doc, global);
    }

    section_header(ui, t!("mix.sends").as_ref());
    show_channel_sends(ui, doc, ch);

    section_header(ui, t!("channel.inserts").as_ref());
    show_inserts(
        ui,
        doc.mixer
            .channel_node(ch)
            .map(|node| node.inserts.as_slice()),
    );
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
    show_inserts(ui, Some(doc.mixer.master.inserts.as_slice()));
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

fn tracks_on_channel(doc: &Document, ch: ChannelId) -> Vec<u16> {
    (0..doc.data.model.tracks.len())
        .filter(|&i| doc.track_channel_id(i) == Some(ch))
        .map(|i| i as u16)
        .collect()
}

/// 通道主输出（Master / 另一条 LC）。返回是否改动。
fn output_selector(ui: &mut egui::Ui, doc: &mut Document, ch: ChannelId) -> bool {
    let map = doc.mixer.channel_display_map();
    let source = ch;
    let mut targets: Vec<(yinhe_types::ChannelId, String)> = Vec::new();
    // 普通 LC（音频 / 每轨）+ AP 通道。
    for node in doc.mixer.nodes() {
        let id = node.channel_id();
        if id == source {
            continue;
        }
        if matches!(
            node.source,
            yinhe_types::ChannelSource::Audio(_) | yinhe_types::ChannelSource::Track(_)
        ) {
            targets.push((id, crate::mix::channel_id_label(id, &map)));
        }
    }
    for ap in &doc.mixer.aps {
        let id = yinhe_types::ChannelId::Ap(yinhe_types::ApChannelId::new(ap.id, 0));
        if id != source {
            targets.push((id, crate::mix::channel_id_label(id, &map)));
        }
    }
    let options: Vec<(usize, String)> = std::iter::once((0usize, "Master".to_string()))
        .chain(
            targets
                .iter()
                .enumerate()
                .map(|(i, (_, label))| (i + 1, label.clone())),
        )
        .collect();
    let current = doc.mixer.node_output(ch);
    let mut sel = current
        .and_then(|target| targets.iter().position(|(id, _)| *id == target))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut changed = false;
    panel_row(ui, t!("track.output").as_ref(), |ui| {
        if crate::widgets::combo::combo_select(
            ui,
            "channel_main_output",
            &mut sel,
            ui.available_width().max(1.0),
            &options,
        ) {
            changed = true;
        }
    });
    if changed {
        let target = if sel == 0 {
            None
        } else {
            targets.get(sel - 1).map(|(id, _)| *id)
        };
        let accepted = doc.mixer_mut().set_node_output(ch, target);
        if accepted {
            doc.mark_mixer_dirty();
        }
        return accepted;
    }
    false
}

fn show_instrument_summary(ui: &mut egui::Ui, doc: &Document, ch: u8) {
    let chain = doc
        .mixer
        .rack_instruments(yinhe_types::RackId::from_midi_channel(ch));
    if chain.is_empty() {
        return;
    }
    for ins in chain {
        value_row(ui, ins.name.clone(), "");
    }
}

fn show_channel_sends(ui: &mut egui::Ui, doc: &Document, ch: ChannelId) {
    let source = ch;
    let map = doc.mixer.channel_display_map();
    let sends: Vec<_> = doc.mixer.channel_sends_from(source).collect();
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
        let db = if send.amount <= 0.0 {
            "-inf".to_string()
        } else {
            format!("{:.1}", crate::widgets::meter::gain_to_db(send.amount))
        };
        value_row(
            ui,
            crate::mix::channel_id_label(send.target, &map),
            format!("{db} dB · {mode}"),
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
