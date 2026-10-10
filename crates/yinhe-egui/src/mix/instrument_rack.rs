//! 乐器机架：每个 MIDI 通道（TrackData.global_channel()）可挂**多个**乐器插件
//! 实例（叠加），UI/管理线程持有其生命周期。音符事件广播给链内每个乐器，输出
//! 相加。空链时该通道使用内置 XSynth。
//!
//! 数据流：
//! - 选择插件：InsertRef 追加到 doc.mixer.instruments[channel]（持久化）→ rack.add
//!   加载实例（不激活、不发送）→ ensure_all_sent 激活并发 SetInstrumentSlot 安装；
//! - 移除/替换：rack.remove → 已安装的送 SetInstrumentSlot{slot_id, None}，旧实例
//!   移入 pending_return，旧处理器退回后按 slot_id 匹配 deactivate；
//! - 回收：渲染线程经乐器 return 通道退回 (slot_id, processor) → on_returns；
//! - 引擎重建：teardown 把全部乐器处理器退回 → on_returns 置 sent=false，
//!   新引擎 ensure_all_sent 重新安装（与效果器机架同一模式）。

use std::path::Path;

use yinhe_audio::{AudioCommand, AudioHandle};
use yinhe_clap::ClapProcessor;
use yinhe_mixer::{InstrumentProcessor, MixerParams, PluginFormat};
use yinhe_vst3::Vst3Processor;

use super::plugin_instance::{PluginEntry, PluginInstance};
use super::rack::{ACTIVATE_MAX_FRAMES, PluginLoadError};

/// 单个乐器槽位的运行时状态。
pub(crate) struct InstrumentSlot {
    /// 乐器通道号（0 起）。
    pub channel: u8,
    /// 稳定的 UI 侧槽位 id（引擎回收退回时匹配用）。
    pub slot_id: u64,
    /// None = 加载失败占位（持久化层仍保留 InsertRef，保存不丢引用）。
    pub instance: Option<PluginInstance>,
    /// 处理器当前在渲染线程（已 SetInstrumentSlot 且未退回）。
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
    /// 全部乐器槽位，按通道分组、组内按挂载顺序排列（= 持久化链顺序）。
    slots: Vec<InstrumentSlot>,
    /// 下一个槽位 id（单调递增）。
    next_slot_id: u64,
    /// 已移除/被替换但仍占着渲染线程的旧实例：其旧处理器退回后按 slot_id deactivate。
    pending_return: Vec<(u64, PluginInstance)>,
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

    /// 某通道第 `index` 个槽位（按挂载顺序）。
    fn slot_mut(&mut self, channel: u8, index: usize) -> Option<&mut InstrumentSlot> {
        self.slots
            .iter_mut()
            .filter(|s| s.channel == channel)
            .nth(index)
    }

    /// 该通道的槽位数量（UI 列表长度 / 设备链乐器段长度）。
    pub(crate) fn slot_count(&self, channel: u8) -> usize {
        self.slots.iter().filter(|s| s.channel == channel).count()
    }

    /// 按槽位 id 取变体（引擎退回匹配用）。
    fn slot_by_id(&self, slot_id: u64) -> Option<usize> {
        self.slots.iter().position(|s| s.slot_id == slot_id)
    }

    /// 按乐器通道 + 第 index 个取插件实例（参数面板用）。
    pub(crate) fn instance_mut(
        &mut self,
        channel: u8,
        index: usize,
    ) -> Option<&mut PluginInstance> {
        self.slot_mut(channel, index)?.instance.as_mut()
    }

    /// 该乐器通道是否持有可用插件实例（音符预览路由用：有实例走插件试听）。
    pub(crate) fn has_instance(&self, channel: u8) -> bool {
        self.slots
            .iter()
            .any(|s| s.channel == channel && s.instance.is_some())
    }

    /// 打开/关闭乐器插件原生界面（host 自建窗口 + 插件 view 嵌入）。
    /// CLAP / VST3 共用宿主 NSWindow（与效果器机架同一实现）。
    #[cfg(target_os = "macos")]
    pub fn toggle_gui(&mut self, channel: u8, index: usize) -> Result<bool, PluginLoadError> {
        let Some(rt) = self.slot_mut(channel, index) else {
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
    pub fn toggle_gui(&mut self, _channel: u8, _index: usize) -> Result<bool, PluginLoadError> {
        Err(PluginLoadError("当前平台暂不支持插件界面".into()))
    }

    /// 追加一个乐器槽位到该通道链尾（不激活、不发送——发送走 ensure_all_sent）。
    /// 持久化层 InsertRef 由调用方先行 append。
    pub fn add(
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
        let slot_id = self.next_slot_id;
        self.next_slot_id += 1;
        self.slots.push(InstrumentSlot {
            channel,
            slot_id,
            instance,
            sent: false,
            activate_failed: false,
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
        });
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 激活槽位并发送 SetInstrumentSlot 安装。
    fn activate_slot(
        &mut self,
        pos: usize,
        handle: &AudioHandle,
        sample_rate: u32,
    ) -> Result<(), PluginLoadError> {
        let (channel, slot_id) = (self.slots[pos].channel, self.slots[pos].slot_id);
        let Some(instance) = self.slots[pos].instance.as_mut() else {
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
        handle.send(AudioCommand::SetInstrumentSlot {
            channel,
            slot_id,
            processor: Some(processor),
        });
        self.slots[pos].sent = true;
        Ok(())
    }

    /// 引擎（重）spawn 后：补发所有「有实例但未在渲染线程」的乐器槽位。
    pub fn ensure_all_sent(&mut self, handle: &AudioHandle, sample_rate: u32) {
        let targets: Vec<usize> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, rt)| !rt.sent && !rt.activate_failed)
            .map(|(i, _)| i)
            .collect();
        for pos in targets {
            if let Err(e) = self.activate_slot(pos, handle, sample_rate) {
                self.last_error = Some(e.0);
                self.slots[pos].activate_failed = true;
            }
        }
    }

    /// 移除该通道第 `index` 个乐器槽位：已安装的送 SetInstrumentSlot{None}，旧实例
    /// 移入 pending_return 等旧处理器退回 deactivate；从未进引擎时直接 drop。
    pub fn remove(&mut self, channel: u8, index: usize, handle: Option<&AudioHandle>) {
        let pos = match self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.channel == channel)
            .nth(index)
            .map(|(i, _)| i)
        {
            Some(pos) => pos,
            None => return,
        };
        let slot = self.slots.remove(pos);
        if slot.sent {
            if let Some(h) = handle {
                h.send(AudioCommand::SetInstrumentSlot {
                    channel,
                    slot_id: slot.slot_id,
                    processor: None,
                });
            }
            if let Some(inst) = slot.instance {
                self.pending_return.push((slot.slot_id, inst));
            }
        }
    }

    /// 每帧轮询乐器插件的反向请求（CLAP/VST3 统一）：
    /// - restart / I/O 变化 → 送 `SetInstrumentSlot{None}` 收回处理器，退回后
    ///   `on_returns` 置 `sent=false`，下一帧 `ensure_all_sent` 重新激活补发；
    /// - 参数重扫 → 实例内暂存，参数面板刷新时消费；
    /// - 延迟变化 → CLAP 在此重查共享值，然后通知引擎重算 PDC。
    pub fn poll_requests(&mut self, handle: &AudioHandle) {
        let mut restarts: Vec<(u8, u64)> = Vec::new();
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
                restarts.push((rt.channel, rt.slot_id));
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
        for (channel, slot_id) in restarts {
            handle.send(AudioCommand::SetInstrumentSlot {
                channel,
                slot_id,
                processor: None,
            });
        }
        if latency_changed {
            handle.send(AudioCommand::RefreshLatency);
        }
    }

    /// 处理渲染线程退回的乐器处理器：先按 slot_id 匹配 pending_return（移除/替换的
    /// 旧实例），再匹配当前槽位（引擎 teardown 回收），deactivate 并置 sent=false。
    pub fn on_returns(&mut self, returned: Vec<(u64, Box<dyn InstrumentProcessor>)>) {
        for (slot_id, processor) in returned {
            let any = processor.into_any();
            match any.downcast::<ClapProcessor>() {
                Ok(clap) => {
                    if let Some(idx) = self
                        .pending_return
                        .iter()
                        .position(|(id, _)| *id == slot_id)
                    {
                        let (_, mut inst) = self.pending_return.remove(idx);
                        if let PluginInstance::Clap(instance) = &mut inst {
                            instance.deactivate(*clap);
                        }
                        continue;
                    }
                    let Some(pos) = self.slot_by_id(slot_id) else {
                        tracing::warn!("退回的乐器处理器 slot_id={slot_id} 找不到槽位，丢弃");
                        continue;
                    };
                    if let Some(PluginInstance::Clap(instance)) = self.slots[pos].instance.as_mut()
                    {
                        instance.deactivate(*clap);
                    } else {
                        tracing::warn!(
                            "slot_id={slot_id} 的槽位无 CLAP 实例，处理器无法 deactivate，丢弃"
                        );
                    }
                    self.slots[pos].sent = false;
                }
                Err(any) => match any.downcast::<Vst3Processor>() {
                    Ok(vst3) => {
                        // VST3：stop（关激活）后直接释放；被替换的旧实例无需匹配。
                        vst3.stop();
                        if let Some(idx) = self
                            .pending_return
                            .iter()
                            .position(|(id, _)| *id == slot_id)
                        {
                            self.pending_return.remove(idx);
                            continue;
                        }
                        if let Some(pos) = self.slot_by_id(slot_id) {
                            self.slots[pos].sent = false;
                        }
                    }
                    Err(_) => tracing::warn!("退回的乐器处理器类型未知，丢弃"),
                },
            }
        }
    }

    /// 保存前把实例状态写回持久化层（mixer.instruments[channel][index].state）。
    /// 槽位顺序与持久化链顺序一致（都是挂载顺序）。
    pub fn sync_states_to(&mut self, mixer: &mut MixerParams) {
        let mut counter: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
        for slot in self.slots.iter_mut() {
            let index = *counter.entry(slot.channel).or_insert(0);
            *counter.get_mut(&slot.channel).unwrap() += 1;
            let Some(r) = mixer
                .instruments
                .get_mut(slot.channel as usize)
                .and_then(|refs| refs.get_mut(index))
            else {
                continue;
            };
            // 加载失败占位无实例：保留工程里的旧 state。
            let Some(instance) = slot.instance.as_mut() else {
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
            slot_id: 0,
            instance: None,
            sent: false,
            activate_failed: false,
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
        });
        let mut mixer = MixerParams::default();
        mixer.instruments[3] = vec![yinhe_mixer::InsertRef {
            format: yinhe_mixer::PluginFormat::Clap,
            plugin_path: std::path::PathBuf::from("/tmp/x.clap"),
            plugin_id: "test".into(),
            name: "Test".into(),
            bypassed: false,
            state: Some(vec![1, 2, 3]),
        }];
        rack.sync_states_to(&mut mixer);
        assert_eq!(mixer.instruments[3][0].state, Some(vec![1, 2, 3]));
    }

    #[test]
    fn remove_unsent_slot_drops_it() {
        // 手工塞一个未安装槽位，remove 直接 drop（无引擎命令）。
        let mut rack = InstrumentRack::default();
        rack.slots.push(InstrumentSlot {
            channel: 2,
            slot_id: 0,
            instance: None,
            sent: false,
            activate_failed: false,
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
        });
        rack.remove(2, 0, None);
        assert!(rack.slots.is_empty());
        assert!(rack.pending_return.is_empty());
    }
}
