//! 乐器机架：每个乐器通道（TrackData.instrument_channel）对应一个 CLAP 乐器实例，
//! UI/管理线程持有其生命周期。比效果器机架（rack.rs）简单：一个通道只有一个
//! 乐器插件（无链、无旁通、原生 GUI 暂不支持），输出直接混进该乐器 dense 通道。
//!
//! 数据流：
//! - 选择插件：InsertRef 写入 doc.mixer.instruments[channel]（持久化）→ rack.load
//!   加载实例（不激活、不发送）→ ensure_all_sent 激活并发 SetInstrument 安装；
//! - 移除/替换：rack.unload / load 替换 —— 已安装的旧实例移入 pending_return，
//!   送 SetInstrument(None)/替换命令，旧处理器退回后 deactivate 旧实例；
//! - 回收：渲染线程经乐器 return 通道退回（含通道号）→ on_returns 先匹配
//!   pending_return（旧实例）再匹配当前槽位（teardown 回收），deactivate；
//! - 引擎重建：teardown 把全部乐器处理器退回 → on_returns 置 sent=false，
//!   新引擎 ensure_all_sent 重新安装（与效果器机架同一模式）。

use std::path::Path;

use yinhe_audio::{AudioCommand, AudioHandle};
use yinhe_clap::{ClapPluginInstance, ClapProcessor, PluginInfo};
use yinhe_mixer::MixerParams;

use super::rack::{ACTIVATE_MAX_FRAMES, PluginLoadError, host_info};

/// 单个乐器通道的运行时槽位。
pub(crate) struct InstrumentSlot {
    /// 乐器通道号（0 起）。
    pub channel: u16,
    /// None = 加载失败占位（持久化层仍保留 InsertRef，保存不丢引用）。
    pub instance: Option<ClapPluginInstance>,
    /// 处理器当前在渲染线程（已 SetInstrument 且未退回）。
    pub sent: bool,
    /// 激活失败过：不再每帧重试（重新选择插件才会再试）。
    pub activate_failed: bool,
}

/// 一个文档的乐器机架（与 documents 平行，索引 = 文档 idx）。
#[derive(Default)]
pub(crate) struct InstrumentRack {
    /// 当前已分配乐器的通道槽位，按 channel 升序。
    pub slots: Vec<InstrumentSlot>,
    /// 已移除/被替换但仍占着渲染线程的旧实例：其旧处理器退回后 deactivate。
    /// 每通道至多一条（再次替换会直接覆盖丢弃更旧的——其处理器在引擎侧已丢失）。
    pending_return: Vec<(u16, ClapPluginInstance)>,
    /// 最近一次加载/激活失败信息（MIX 界面状态行展示）。
    pub last_error: Option<String>,
}

impl InstrumentRack {
    fn slot_mut(&mut self, channel: u16) -> Option<&mut InstrumentSlot> {
        self.slots.iter_mut().find(|s| s.channel == channel)
    }

    /// 加载某乐器通道的插件实例（不激活、不发送——发送走 ensure_all_sent）。
    /// 替换该通道已有槽位：已安装的旧实例移入 pending_return，等旧处理器退回 deactivate。
    /// 持久化层 InsertRef 由调用方先行写入。
    pub fn load(
        &mut self,
        channel: u16,
        plugin_path: &Path,
        plugin_id: &str,
        name: &str,
        state: Option<&[u8]>,
    ) -> Result<(), PluginLoadError> {
        let info = PluginInfo {
            path: plugin_path.to_path_buf(),
            id: plugin_id.to_string(),
            name: name.to_string(),
            vendor: None,
            version: None,
            features: Vec::new(),
        };
        let result = (|| {
            let mut instance = ClapPluginInstance::load(&info, &host_info())
                .map_err(|e| PluginLoadError(format!("{e}")))?;
            if let Some(bytes) = state {
                instance
                    .load_state(bytes)
                    .map_err(|e| PluginLoadError(format!("恢复乐器插件状态失败: {e}")))?;
            }
            Ok::<_, PluginLoadError>(instance)
        })();
        let (instance, error) = match result {
            Ok(inst) => (Some(inst), None),
            Err(e) => (None, Some(e)),
        };
        if let Some(e) = &error {
            self.last_error = Some(e.0.clone());
        }
        // 替换已有槽位：旧实例已安装则移入 pending_return，等旧处理器退回。
        if let Some(old_idx) = self.slots.iter().position(|s| s.channel == channel) {
            let old = self.slots.remove(old_idx);
            if old.sent
                && let Some(inst) = old.instance
            {
                self.pending_return.push((channel, inst));
            }
        }
        self.slots.push(InstrumentSlot {
            channel,
            instance,
            sent: false,
            activate_failed: false,
        });
        self.slots.sort_by_key(|s| s.channel);
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 激活槽位并发送 SetInstrument 安装。
    fn activate_slot(
        &mut self,
        channel: u16,
        handle: &AudioHandle,
        sample_rate: u32,
    ) -> Result<(), PluginLoadError> {
        let Some(rt) = self.slot_mut(channel) else {
            return Ok(());
        };
        let Some(instance) = rt.instance.as_mut() else {
            return Ok(()); // 加载失败占位：跳过激活
        };
        let processor = instance
            .activate(sample_rate as f64, ACTIVATE_MAX_FRAMES)
            .map_err(|e| PluginLoadError(format!("激活乐器插件失败: {e}")))?;
        handle.send(AudioCommand::SetInstrument {
            channel,
            processor: Some(Box::new(processor)),
        });
        rt.sent = true;
        Ok(())
    }

    /// 引擎（重）spawn 后：补发所有「有实例但未在渲染线程」的乐器槽位。
    pub fn ensure_all_sent(&mut self, handle: &AudioHandle, sample_rate: u32) {
        let targets: Vec<u16> = self
            .slots
            .iter()
            .filter(|rt| !rt.sent && !rt.activate_failed)
            .map(|rt| rt.channel)
            .collect();
        for channel in targets {
            if let Err(e) = self.activate_slot(channel, handle, sample_rate) {
                self.last_error = Some(e.0);
                if let Some(rt) = self.slot_mut(channel) {
                    rt.activate_failed = true;
                }
            }
        }
    }

    /// 移除某乐器通道（MIX 界面 ✕）：已安装的送 SetInstrument(None)，旧实例移入
    /// pending_return 等旧处理器退回 deactivate；从未进引擎时直接 drop。
    pub fn unload(&mut self, channel: u16, handle: Option<&AudioHandle>) {
        let Some(idx) = self.slots.iter().position(|s| s.channel == channel) else {
            return;
        };
        let slot = self.slots.remove(idx);
        if slot.sent {
            if let Some(h) = handle {
                h.send(AudioCommand::SetInstrument {
                    channel,
                    processor: None,
                });
            }
            if let Some(inst) = slot.instance {
                self.pending_return.push((channel, inst));
            }
        }
    }

    /// 处理渲染线程退回的乐器处理器：先匹配 pending_return（移除/替换的旧实例），
    /// 再匹配当前槽位（引擎 teardown 回收），deactivate 并置 sent=false。
    pub fn on_returns(&mut self, returned: Vec<(u16, ClapProcessor)>) {
        for (channel, processor) in returned {
            if let Some(idx) = self.pending_return.iter().position(|(c, _)| *c == channel) {
                let (_, mut inst) = self.pending_return.remove(idx);
                inst.deactivate(processor);
                continue;
            }
            let Some(rt) = self.slot_mut(channel) else {
                tracing::warn!("退回的乐器处理器 channel={channel} 找不到槽位，丢弃");
                continue;
            };
            if let Some(instance) = rt.instance.as_mut() {
                instance.deactivate(processor);
            } else {
                tracing::warn!("channel={channel} 的槽位无实例，处理器无法 deactivate，丢弃");
            }
            rt.sent = false;
        }
    }

    /// 保存前把实例状态写回持久化层（mixer.instruments[channel].state）。
    /// 已移除通道的 InsertRef 由移除动作置 None，这里跳过。
    pub fn sync_states_to(&mut self, mixer: &mut MixerParams) {
        for rt in self.slots.iter_mut() {
            let c = rt.channel as usize;
            if mixer.instruments.len() <= c {
                mixer.instruments.resize(c + 1, None);
            }
            let Some(r) = mixer.instruments[c].as_mut() else {
                continue;
            };
            // 加载失败占位无实例：保留工程里的旧 state。
            let Some(instance) = rt.instance.as_mut() else {
                continue;
            };
            match instance.save_state() {
                Ok(Some(bytes)) => r.state = Some(bytes),
                Ok(None) => {}
                Err(e) => tracing::warn!("保存乐器插件状态失败: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_states_preserves_state_for_placeholder_slot() {
        // 无实例占位槽位（channel 3）：sync 不应 panic 且不改旧 state。
        let mut rack = InstrumentRack::default();
        rack.slots.push(InstrumentSlot {
            channel: 3,
            instance: None,
            sent: false,
            activate_failed: false,
        });
        let mut mixer = MixerParams::default();
        mixer.instruments.resize(4, None);
        mixer.instruments[3] = Some(yinhe_mixer::InsertRef {
            plugin_path: std::path::PathBuf::from("/tmp/x.clap"),
            plugin_id: "test".into(),
            name: "Test".into(),
            bypassed: false,
            state: Some(vec![1, 2, 3]),
        });
        rack.sync_states_to(&mut mixer);
        assert_eq!(
            mixer.instruments[3].as_ref().unwrap().state,
            Some(vec![1, 2, 3])
        );
    }

    #[test]
    fn unload_unsent_slot_drops_it() {
        // 手工塞一个未安装槽位，unload 直接 drop（无引擎命令）。
        let mut rack = InstrumentRack::default();
        rack.slots.push(InstrumentSlot {
            channel: 2,
            instance: None,
            sent: false,
            activate_failed: false,
        });
        rack.unload(2, None);
        assert!(!rack.slots.iter().any(|s| s.channel == 2));
        assert!(rack.pending_return.is_empty());
    }
}
