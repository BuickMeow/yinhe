//! 混音台交互动作：一帧内 strip 收集，渲染后统一应用。

use eframe::egui;
use rust_i18n::t;
use yinhe_audio::InsertTarget;
use yinhe_mixer::{MasterParams, StripParams};
use yinhe_types::{ChannelId, MixerChannelId};

use super::{App, PluginEntry, assign_instrument_plugin, insert_refs, start_plugin_scan};

/// 一帧内 strip 交互产出的动作（渲染完统一应用，避开借用冲突）。
pub(crate) enum MixAction {
    /// 更新任意混音节点（MIDI / 音频 / Bus）的 strip 参数。
    SetNodeStrip {
        node: MixerChannelId,
        params: StripParams,
    },
    SetMaster {
        params: MasterParams,
    },
    AddInsert {
        target: InsertTarget,
        plugin: PluginEntry,
    },
    /// C 的任意通道发送（dB 旋钮）。
    SetChannelSend {
        source: ChannelId,
        target: ChannelId,
        amount: f32,
        pre_fader: bool,
    },
    /// 给任意混音节点添加 MixerGain 自动化 lane。
    AddMixerGainAutomation {
        channel: MixerChannelId,
    },
    /// 给任意混音节点添加 MixerPan 自动化 lane。
    AddMixerPanAutomation {
        channel: MixerChannelId,
    },
    /// 冻结一条 LC（离线渲染整条时间轴）。
    FreezeChannel {
        channel: MixerChannelId,
    },
    /// 解除某节点的冻结。
    UnfreezeChannel {
        node: yinhe_types::MixerNodeId,
    },
    /// 为 MIDI 通道**追加**一个插件乐器到链尾（InsertRef 入持久化层 +
    /// 机架加载 + 安装引擎）。
    AssignInstrument {
        channel: u8,
        plugin: PluginEntry,
    },
    /// 清空某 MIDI 通道的全部插件乐器（回到内置 XSynth）。
    ClearInstruments {
        channel: u8,
    },
    RescanPlugins,
}

pub(super) fn apply_action(app: &mut App, _ctx: &egui::Context, idx: usize, action: MixAction) {
    match action {
        MixAction::SetNodeStrip { node, params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_node_strip(idx, node, params);
        }
        MixAction::SetMaster { params } => {
            app.note_mixer_edit(t!("undo.mix_param").as_ref());
            app.apply_master(idx, params);
        }
        MixAction::AddInsert { target, plugin } => {
            if let Some(refs) = insert_refs(app.workspace.documents[idx].mixer_mut(), target) {
                refs.push(yinhe_mixer::InsertRef {
                    uid: 0,
                    plugin_path: plugin.path.clone(),
                    plugin_id: plugin.id.clone(),
                    name: plugin.name.clone(),
                    format: plugin.format,
                    bypassed: false,
                    state: None,
                });
            }
            app.workspace.documents[idx].data.bump_revision();
            let rack = app.mixer_rack_mut(idx);
            if let Err(e) = rack.load_plugin(
                target,
                plugin.format,
                &plugin.path,
                &plugin.id,
                &plugin.name,
                None,
                false,
            ) {
                // 加载失败：引用已入持久化层（保存不丢），但机架无实例；
                // 状态行提示用户。
                rack.last_error = Some(e.0);
            }
            app.push_mixer_state_to_engine(idx);
            app.mix.picker_for = None;
        }
        MixAction::SetChannelSend {
            source,
            target,
            amount,
            pre_fader,
        } => {
            app.note_mixer_edit(t!("undo.mix_send").as_ref());
            if app.workspace.documents[idx]
                .mixer_mut()
                .set_channel_send(source, target, amount, pre_fader)
            {
                app.push_mixer_state_to_engine(idx);
            }
        }
        MixAction::FreezeChannel { channel } => {
            app.start_lc_freeze(idx, channel);
        }
        MixAction::UnfreezeChannel { node } => {
            let doc = &mut app.workspace.documents[idx];
            let before = doc.capture_snapshot();
            if let Some(action) = doc.set_node_freeze(node, None) {
                doc.push_undo(action, t!("undo.freeze_lc").as_ref(), before);
                app.push_mixer_state_to_engine(idx);
            }
        }
        MixAction::AddMixerGainAutomation { channel } => {
            let value = app.workspace.documents[idx]
                .mixer
                .node_strip(channel)
                .gain
                .clamp(0.0, 1.0);
            let node_id = app.workspace.documents[idx].mixer.node_id_for(channel);
            let doc = &mut app.workspace.documents[idx];
            let before = doc.capture_snapshot();
            if let Some(action) = doc.add_mixer_automation_lane(
                node_id,
                yinhe_types::AutomationTarget::MixerGain { channel },
                value,
            ) {
                doc.push_undo(action, t!("undo.create_automation").as_ref(), before);
                app.workspace.documents[idx].mark_mixer_dirty();
                app.push_mixer_state_to_engine(idx);
            }
        }
        MixAction::AddMixerPanAutomation { channel } => {
            let pan = app.workspace.documents[idx].mixer.node_strip(channel).pan;
            let value = (pan.clamp(-1.0, 1.0) + 1.0) * 0.5;
            let node_id = app.workspace.documents[idx].mixer.node_id_for(channel);
            let doc = &mut app.workspace.documents[idx];
            let before = doc.capture_snapshot();
            if let Some(action) = doc.add_mixer_automation_lane(
                node_id,
                yinhe_types::AutomationTarget::MixerPan { channel },
                value,
            ) {
                doc.push_undo(action, t!("undo.create_automation").as_ref(), before);
                app.workspace.documents[idx].mark_mixer_dirty();
                app.push_mixer_state_to_engine(idx);
            }
        }
        MixAction::AssignInstrument { channel, plugin } => {
            assign_instrument_plugin(app, idx, channel, &plugin);
            app.mix.instrument_picker_for = None;
        }
        MixAction::ClearInstruments { channel } => {
            app.workspace.documents[idx].mixer_mut().racks[channel as usize]
                .instruments
                .clear();
            app.workspace.documents[idx].data.bump_revision();
            if idx < app.instrument_racks.len() {
                let handle = app.audio_state.handle.as_ref().map(|a| &a.handle);
                let rack = &mut app.instrument_racks[idx];
                while rack.slot_count(channel) > 0 {
                    rack.remove(channel, 0, handle);
                }
            }
            app.notify_audio_model_changed();
            app.push_mixer_state_to_engine(idx);
        }
        MixAction::RescanPlugins => start_plugin_scan(app),
    }
}
