//! 乐器机架：每个 MIDI 通道（TrackData.global_channel()）至多挂一个乐器插件实例，
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
use yinhe_clap::ClapProcessor;
use yinhe_mixer::{InstrumentProcessor, MixerParams, PluginFormat};
use yinhe_vst3::Vst3Processor;

use super::plugin_instance::{PluginEntry, PluginInstance};
use super::rack::{ACTIVATE_MAX_FRAMES, PluginLoadError};

/// 单个乐器通道的运行时槽位。
pub(crate) struct InstrumentSlot {
    /// 乐器通道号（0 起）。
    pub channel: u8,
    /// None = 加载失败占位（持久化层仍保留 InsertRef，保存不丢引用）。
    pub instance: Option<PluginInstance>,
    /// 处理器当前在渲染线程（已 SetInstrument 且未退回）。
    pub sent: bool,
    /// 激活失败过：不再每帧重试（重新选择插件才会再试）。
    pub activate_failed: bool,
    /// 插件原生 GUI 窗口当前打开中（host 自建窗口嵌入插件 view）。
    pub gui_open: bool,
    /// 宿主侧 GUI 窗口（macOS NSWindow）。必须在 `instance` 之后声明：
    /// 字段按声明顺序 drop，instance 的 Drop 先执行 close_gui（插件 view
    /// 从父 view 移除），之后窗口对象才能释放。
    #[cfg(target_os = "macos")]
    pub gui_window: Option<super::gui_window::PluginGuiWindow>,
}

/// 一个文档的乐器机架（与 documents 平行，索引 = 文档 idx）。
#[derive(Default)]
pub(crate) struct InstrumentRack {
    /// 当前已分配乐器的通道槽位，按 channel 升序。
    pub slots: Vec<InstrumentSlot>,
    /// 已移除/被替换但仍占着渲染线程的旧实例：其旧处理器退回后 deactivate。
    /// 每通道至多一条（再次替换会直接覆盖丢弃更旧的——其处理器在引擎侧已丢失）。
    pending_return: Vec<(u8, PluginInstance)>,
    /// 最近一次加载/激活失败信息（MIX 界面状态行展示）。
    pub last_error: Option<String>,
    /// 待处理的插件 GUI 改参（MIDI 通道, param_id, 归一化值）。
    /// 每帧由 UI 消费（写入插件参数 AM lane）；队列在 `poll_requests` 填充。
    pub gui_param_changes: Vec<(u8, u32, f64)>,
    /// 正在 GUI 编辑（beginEdit 后未 endEdit）的乐器通道集：
    /// 一次拖动合并为一条 undo 的分组依据。
    pub gui_param_editing: std::collections::HashSet<u8>,
}

impl InstrumentRack {
    /// 机架内是否没有任何插件乐器（引擎侧没有本机架装过的乐器）。
    /// 跨文档复用引擎的前提之一，语义同 `MixerRack::is_plugin_free`。
    pub(crate) fn is_plugin_free(&self) -> bool {
        self.slots.iter().all(|s| s.instance.is_none() && !s.sent) && self.pending_return.is_empty()
    }

    fn slot_mut(&mut self, channel: u8) -> Option<&mut InstrumentSlot> {
        self.slots.iter_mut().find(|s| s.channel == channel)
    }

    /// 按乐器通道取插件实例（参数面板用）。槽位不存在/无实例返回 None。
    pub(crate) fn instance_mut(&mut self, channel: u8) -> Option<&mut PluginInstance> {
        self.slot_mut(channel)?.instance.as_mut()
    }

    /// 该乐器通道是否持有可用插件实例（音符预览路由用：有实例走插件试听）。
    pub(crate) fn has_instance(&self, channel: u8) -> bool {
        self.slots
            .iter()
            .find(|s| s.channel == channel)
            .is_some_and(|s| s.instance.is_some())
    }

    /// 打开/关闭乐器插件原生界面（host 自建窗口 + 插件 view 嵌入）。
    /// CLAP / VST3 共用宿主 NSWindow（与效果器机架同一实现）。
    #[cfg(target_os = "macos")]
    pub fn toggle_gui(&mut self, channel: u8) -> Result<bool, PluginLoadError> {
        let Some(rt) = self.slot_mut(channel) else {
            return Ok(false);
        };
        if rt.instance.is_none() {
            tracing::warn!("打开乐器界面失败: 槽位无实例（插件未加载成功）");
            return Err(PluginLoadError("插件未加载成功，无法打开界面".into()));
        }
        // 关闭：先让插件 view 脱离父 view，再释放窗口对象。
        if rt.gui_open {
            close_plugin_view(rt);
            rt.gui_window = None;
            rt.gui_open = false;
            return Ok(false);
        }
        // 打开：创建插件 view + 宿主窗口 + 嵌入。
        // resizable = 插件能力（CLAP can_resize / VST3 canResize）：不支持时
        // 窗口去掉缩放样式，避免拖大后插件 GUI 不跟随、露出空白背景。
        let (name, size_result, resizable): (String, Result<(u32, u32), String>, bool) =
            match rt.instance.as_mut() {
                Some(PluginInstance::Clap(inst)) => {
                    let result = inst.create_gui().map_err(|e| format!("{e}"));
                    let can_resize = result.is_ok() && inst.gui_can_resize();
                    (inst.info().name.clone(), result, can_resize)
                }
                Some(PluginInstance::Vst3 { instance, name, .. }) => {
                    let result = instance.create_view().map_err(|e| format!("{e}"));
                    let can_resize = result.is_ok() && instance.view_can_resize();
                    (name.clone(), result, can_resize)
                }
                Some(PluginInstance::Builtin { .. }) => {
                    return Err(PluginLoadError(
                        "内置效果器不是乐器，无法打开乐器界面".into(),
                    ));
                }
                None => return Err(PluginLoadError("插件未加载成功，无法打开界面".into())),
            };
        let (w, h) = size_result.map_err(|e| {
            tracing::warn!("乐器界面创建失败 ({name}): {e}");
            PluginLoadError(e)
        })?;
        tracing::info!("乐器界面创建中: {name} {w}x{h} resizable={resizable}");
        let Some(win) = super::gui_window::PluginGuiWindow::new(&name, w, h, resizable) else {
            close_plugin_view(rt);
            tracing::warn!("乐器窗口创建失败: {name}");
            return Err(PluginLoadError("创建插件窗口失败".into()));
        };
        let attach_result: Result<(), String> = match rt.instance.as_mut() {
            Some(PluginInstance::Clap(inst)) => inst
                .attach_and_show_gui(win.view_ptr())
                .map_err(|e| format!("{e}")),
            Some(PluginInstance::Vst3 { instance, .. }) => unsafe {
                instance
                    .attach_view(win.view_ptr())
                    .map_err(|e| format!("{e}"))
            },
            Some(PluginInstance::Builtin { .. }) => Err("内置效果器不是乐器".into()),
            None => Err("实例丢失".into()),
        };
        if let Err(e) = attach_result {
            close_plugin_view(rt);
            tracing::warn!("乐器界面嵌入失败 ({name}): {e}");
            return Err(PluginLoadError(e));
        }
        win.show();
        tracing::info!("乐器界面已显示: {name}");
        rt.gui_window = Some(win);
        rt.gui_open = true;
        Ok(true)
    }

    /// 非 macOS：原生 GUI 尚未实现。
    #[cfg(not(target_os = "macos"))]
    pub fn toggle_gui(&mut self, _channel: u8) -> Result<bool, PluginLoadError> {
        Err(PluginLoadError("当前平台暂不支持插件界面".into()))
    }

    /// 加载某乐器通道的插件实例（不激活、不发送——发送走 ensure_all_sent）。
    /// 替换该通道已有槽位：已安装的旧实例移入 pending_return，等旧处理器退回 deactivate。
    /// 持久化层 InsertRef 由调用方先行写入。
    pub fn load(
        &mut self,
        channel: u8,
        format: PluginFormat,
        plugin_path: &Path,
        plugin_id: &str,
        name: &str,
        state: Option<&[u8]>,
    ) -> Result<(), PluginLoadError> {
        let entry = PluginEntry {
            format,
            path: plugin_path.to_path_buf(),
            id: plugin_id.to_string(),
            name: name.to_string(),
            vendor: String::new(),
            is_instrument: true,
            is_effect: false,
            error: None,
        };
        let result = (|| {
            let mut instance = PluginInstance::load(&entry).map_err(PluginLoadError)?;
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
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
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
        channel: u8,
        handle: &AudioHandle,
        sample_rate: u32,
    ) -> Result<(), PluginLoadError> {
        let Some(rt) = self.slot_mut(channel) else {
            return Ok(());
        };
        let Some(instance) = rt.instance.as_mut() else {
            return Ok(()); // 加载失败占位：跳过激活
        };
        let processor: Box<dyn InstrumentProcessor> = match instance {
            PluginInstance::Clap(inst) => {
                let p = inst
                    .activate(sample_rate as f64, ACTIVATE_MAX_FRAMES)
                    .map_err(|e| PluginLoadError(format!("激活乐器插件失败: {e}")))?;
                Box::new(p)
            }
            PluginInstance::Vst3 { instance, .. } => {
                let p = instance
                    .activate_audio(sample_rate as f64, ACTIVATE_MAX_FRAMES)
                    .map_err(|e| PluginLoadError(format!("激活 VST3 乐器失败: {e}")))?;
                Box::new(p)
            }
            PluginInstance::Builtin { .. } => {
                return Err(PluginLoadError("内置效果器不是乐器，无法激活".into()));
            }
        };
        handle.send(AudioCommand::SetInstrument {
            channel,
            processor: Some(processor),
        });
        rt.sent = true;
        Ok(())
    }

    /// 引擎（重）spawn 后：补发所有「有实例但未在渲染线程」的乐器槽位。
    pub fn ensure_all_sent(&mut self, handle: &AudioHandle, sample_rate: u32) {
        let targets: Vec<u8> = self
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
    pub fn unload(&mut self, channel: u8, handle: Option<&AudioHandle>) {
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

    /// 每帧轮询乐器插件的反向请求（CLAP/VST3 统一）：
    /// - restart / I/O 变化 → 送 `SetInstrument(None)` 收回处理器，退回后
    ///   `on_returns` 置 `sent=false`，下一帧 `ensure_all_sent` 重新激活补发；
    /// - 参数重扫 → 实例内暂存，参数面板刷新时消费；
    /// - 延迟变化 → CLAP 在此重查共享值，然后通知引擎重算 PDC。
    pub fn poll_requests(&mut self, handle: &AudioHandle) {
        let mut restarts: Vec<u8> = Vec::new();
        let mut latency_changed = false;
        // GUI 改参先收集，循环后再写自身字段（避免与 slots 的借用冲突）。
        let mut param_changes: Vec<(u8, u32, f64)> = Vec::new();
        let mut editing_now: Vec<(u8, bool)> = Vec::new();
        for rt in self.slots.iter_mut() {
            // 插件原生 GUI 轮询（主题同步 / 用户缩放 / 插件改尺寸 / 关窗）。
            #[cfg(target_os = "macos")]
            if rt.gui_open
                && !super::gui_window::poll_plugin_gui(rt.instance.as_mut(), &mut rt.gui_window)
            {
                close_plugin_view(rt);
                rt.gui_window = None;
                rt.gui_open = false;
            }
            let Some(instance) = rt.instance.as_mut() else {
                continue;
            };
            let (changes, editing) = instance.take_gui_param_changes();
            for (param_id, value) in changes {
                param_changes.push((rt.channel, param_id, value));
            }
            editing_now.push((rt.channel, editing));
            let requests = instance.poll_requests();
            if requests.latency_changed {
                latency_changed = true;
            }
            if requests.restart && rt.sent {
                restarts.push(rt.channel);
            }
        }
        for (channel, editing) in editing_now {
            if editing {
                self.gui_param_editing.insert(channel);
            } else {
                self.gui_param_editing.remove(&channel);
            }
        }
        self.gui_param_changes.extend(param_changes);
        for channel in restarts {
            handle.send(AudioCommand::SetInstrument {
                channel,
                processor: None,
            });
        }
        if latency_changed {
            handle.send(AudioCommand::RefreshLatency);
        }
    }

    /// 处理渲染线程退回的乐器处理器：先匹配 pending_return（移除/替换的旧实例），
    /// 再匹配当前槽位（引擎 teardown 回收），deactivate 并置 sent=false。
    ///
    /// 退回的处理器是格式无关 trait object：按具体格式 downcast 回 CLAP 处理器
    ///（VST3 接入后在此按槽位记录的格式分派）。
    pub fn on_returns(&mut self, returned: Vec<(u8, Box<dyn InstrumentProcessor>)>) {
        for (channel, processor) in returned {
            let any = processor.into_any();
            match any.downcast::<ClapProcessor>() {
                Ok(clap) => {
                    if let Some(idx) = self.pending_return.iter().position(|(c, _)| *c == channel) {
                        let (_, mut inst) = self.pending_return.remove(idx);
                        if let PluginInstance::Clap(instance) = &mut inst {
                            instance.deactivate(*clap);
                        }
                        continue;
                    }
                    let Some(rt) = self.slot_mut(channel) else {
                        tracing::warn!("退回的乐器处理器 channel={channel} 找不到槽位，丢弃");
                        continue;
                    };
                    if let Some(PluginInstance::Clap(instance)) = rt.instance.as_mut() {
                        instance.deactivate(*clap);
                    } else {
                        tracing::warn!(
                            "channel={channel} 的槽位无 CLAP 实例，处理器无法 deactivate，丢弃"
                        );
                    }
                    rt.sent = false;
                }
                Err(any) => match any.downcast::<Vst3Processor>() {
                    Ok(vst3) => {
                        // VST3：stop（关激活）后直接释放；被替换的旧实例无需匹配。
                        vst3.stop();
                        if let Some(idx) =
                            self.pending_return.iter().position(|(c, _)| *c == channel)
                        {
                            self.pending_return.remove(idx);
                            continue;
                        }
                        if let Some(rt) = self.slot_mut(channel) {
                            rt.sent = false;
                        }
                    }
                    Err(_) => tracing::warn!("退回的乐器处理器类型未知，丢弃"),
                },
            }
        }
    }

    /// 保存前把实例状态写回持久化层（mixer.instruments[channel].state）。
    /// 已移除通道的 InsertRef 由移除动作置 None，这里跳过。
    pub fn sync_states_to(&mut self, mixer: &mut MixerParams) {
        for rt in self.slots.iter_mut() {
            let Some(r) = mixer
                .instruments
                .get_mut(rt.channel as usize)
                .and_then(|slot| slot.as_mut())
            else {
                continue;
            };
            // 加载失败占位无实例：保留工程里的旧 state。
            let Some(instance) = rt.instance.as_mut() else {
                continue;
            };
            if let Some(bytes) = instance.save_state() {
                r.state = Some(bytes);
            }
        }
    }
}

/// 关闭槽位当前插件的原生 view（CLAP/VST3 分派）。
#[cfg(target_os = "macos")]
fn close_plugin_view(slot: &mut InstrumentSlot) {
    match slot.instance.as_mut() {
        Some(PluginInstance::Clap(inst)) => inst.close_gui(),
        Some(PluginInstance::Vst3 { instance, .. }) => instance.close_view(),
        Some(PluginInstance::Builtin { .. }) | None => {}
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
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
        });
        let mut mixer = MixerParams::default();
        mixer.instruments.resize(4, None);
        mixer.instruments[3] = Some(yinhe_mixer::InsertRef {
            format: yinhe_mixer::PluginFormat::Clap,
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
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
        });
        rack.unload(2, None);
        assert!(!rack.slots.iter().any(|s| s.channel == 2));
        assert!(rack.pending_return.is_empty());
    }
}
