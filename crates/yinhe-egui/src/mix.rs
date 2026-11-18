//! MIX 模式：混音台界面。
//!
//! 通道条按**源 MIDI 通道**（A01..P16）组织：每条 strip 对应一个被工程音轨
//! 使用的通道，音频域的增益/声像/静音/独奏/insert 全部生效（多条轨道共享
//! 同一通道时共享一条 strip，UI 上标注使用该通道的轨道名）。
//!
//! 数据流：
//! - 参数读写：`doc.mixer`（持久化，mixer_mut 标脏）→ `AudioCommand::Set*` 推引擎；
//! - 电平表：渲染线程 → `AudioHandle` 的 MeterReading → 这里做 UI 侧峰值衰减；
//! - insert 生命周期：`MixerRack`（见 rack.rs）。

mod actions;
#[cfg(target_os = "macos")]
pub(crate) mod gui_window;
pub(crate) mod instrument_rack;
pub(crate) mod labels;
mod meter;
pub(crate) mod param_panel;
pub(crate) mod plugin_instance;
mod plugins;
pub(crate) mod rack;
mod session;
mod state;
mod strip;
mod tool_bar;
mod view;

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_audio::channel_layout::ChannelLayout;
use yinhe_mixer::{MasterParams, StripParams};
use yinhe_types::ChannelId;

use self::plugin_instance::PluginEntry;
use crate::plugin_scan::ScanProgress;

use crate::app::App;
use crate::widgets::meter::{db_to_gain, gain_to_db};

/// 源通道总数（16 port × 16 通道）。
pub(crate) use actions::MixAction;
pub(crate) use instrument_rack::InstrumentRack;
pub(crate) use labels::{
    audio_label, channel_id_label, channel_label, channel_title, lc_label, midi_label,
    track_target_label,
};
pub(crate) use meter::smoothed_peak;
pub(crate) use meter::{auto_panel_height, update_master_meter, update_track_meters};
pub(crate) use param_panel::ParamPanel;
pub(crate) use plugins::strip_builtin_inserts;
pub(crate) use plugins::{
    assign_instrument_plugin, insert_refs, poll_plugin_scan, show_global_overlays,
    start_plugin_scan,
};
pub(crate) use rack::MixerRack;
pub(crate) use state::{MeterState, MixUiState, SOURCE_CHANNELS};
pub(crate) use strip::picker_viewport_id;
pub(crate) use view::show;

#[cfg(test)]
mod tests {
    use super::*;
    use yinhe_mixer::{InsertRef, MixerParams, PluginFormat};
    use yinhe_types::ApChannelId;
    use yinhe_types::ApGroupId;

    fn insert_ref(format: PluginFormat, id: &str) -> InsertRef {
        InsertRef {
            plugin_id: id.to_string(),
            format,
            ..Default::default()
        }
    }

    #[test]
    fn ap_and_audio_channels_map_to_channel_ids() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        assert!(mixer.ensure_ap(9));
        assert_eq!(
            mixer.ap_channel_id(9),
            Some(ChannelId::Ap(ApChannelId::new(ApGroupId::new(9), 0)))
        );
        let audio_lc = mixer.ensure_lc_node(yinhe_types::ChannelSource::Audio(1));
        assert_eq!(
            mixer.audio_node(1).expect("audio node").id,
            ChannelId::Lc(audio_lc)
        );
        assert_eq!(mixer.master.id, ChannelId::Master);
    }

    #[test]
    fn ap_channels_show_midi_port_and_channel() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        assert_eq!(channel_title(&mixer, 0), "A01");
        assert!(mixer.ensure_ap(9));
        // AP 也按端口 + 通道显示，不再用 APxx 序号。
        assert_eq!(channel_title(&mixer, 9), "A10");
        assert_eq!(crate::mix::labels::ap_label(0), "A01");
        assert_eq!(crate::mix::labels::ap_label(9), "A10");
        assert_eq!(crate::mix::labels::ap_label(16), "B01");
    }

    /// 迁移只移除内置效果器 insert（channel/master），插件保留；
    /// 无内置时不报告移除（调用方由此避免把刚打开的工程标脏）。
    #[test]
    fn strip_builtin_inserts_removes_builtin_only() {
        let mut mixer = MixerParams::default();
        mixer.ensure_len();
        mixer.ensure_ap(3);
        let ap = mixer.ap_channel_id(3).expect("ap 3");
        mixer.node_mut(ap).expect("ap node").inserts = vec![
            insert_ref(PluginFormat::Builtin, "channel_gain"),
            insert_ref(PluginFormat::Clap, "clap-fx"),
        ];
        mixer.master.inserts = vec![
            insert_ref(PluginFormat::Vst3, "vst-master"),
            insert_ref(PluginFormat::Builtin, "channel_filter"),
        ];

        assert!(strip_builtin_inserts(&mut mixer));
        assert_eq!(mixer.node(ap).expect("ap node").inserts.len(), 1);
        assert_eq!(
            mixer.node(ap).expect("ap node").inserts[0].plugin_id,
            "clap-fx"
        );
        assert_eq!(mixer.master.inserts.len(), 1);
        assert_eq!(mixer.master.inserts[0].plugin_id, "vst-master");

        assert!(!strip_builtin_inserts(&mut mixer), "无内置 insert 不算迁移");
    }

    /// 回归：主输出贴面板右缘（无右边距），与通道区**无间隙**；
    /// 常规宽度下主输出条宽度 = 通道条宽度（不溢出、不居中留缝）。
    #[test]
    fn master_rect_is_flush_with_panel_right() {
        let ctx = egui::Context::default();
        let body = egui::Rect::from_min_max(egui::pos2(44.0, 100.0), egui::pos2(1440.0, 700.0));
        let mut channels = egui::Rect::NOTHING;
        let mut master = egui::Rect::NOTHING;
        ctx.run_ui(Default::default(), |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                crate::widgets::grid::two_columns_right(ui, 0.0, strip::STRIP_WIDTH, |cells| {
                    cells.cell(|ui| channels = ui.max_rect());
                    cells.cell(|ui| master = ui.max_rect());
                });
            });
        })
        .drop_without_applying_deltas();
        assert_eq!(master.max.x, body.max.x, "主输出应贴右缘");
        assert_eq!(master.width(), strip::STRIP_WIDTH, "条宽 = 通道条宽");
        assert_eq!(master.min.x, channels.max.x, "与通道区无间隙");
        assert_eq!(channels.min.x, body.min.x, "通道区从主体左缘开始");
    }
}
