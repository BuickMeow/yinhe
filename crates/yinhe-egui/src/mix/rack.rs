//! 混音台插件机架：CLAP insert 实例的生命周期管理（UI/管理线程持有）。
//!
//! 与 `MixerParams` 的关系：`MixerParams.channel_inserts[ch]` / `master_inserts`
//! 是持久化的槽位列表（顺序即链顺序），本机架持有对应的运行时实例。
//! 不变量：机架链与 params 引用链**顺序一致**（移除槽位时 params 先删、机架
//! 等处理器退回后才删，过渡期机架用 `pending_remove` 跳过对齐）。
//!
//! 处理器流转：
//! - 激活：instance.activate() → ClapInsert（带旁通原子 + owner id）→ InsertAdd 命令；
//! - 移除/引擎拆除：渲染线程经 return 通道退回 → [`on_returns`](Self::on_returns)
//!   按 owner 匹配实例 deactivate；
//! - 插件请求 restart：两阶段（先 InsertRemove 收回旧处理器，退回后 deactivate →
//!   重新 activate → InsertAdd 回原槽位，由 [`ensure_all_sent`](Self::ensure_all_sent)
//!   统一补发）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use yinhe_audio::{AudioCommand, AudioHandle, ClapInsert, InsertTarget};
use yinhe_clap::{ClapProcessor, HostInfo};
use yinhe_mixer::{InsertProcessor, MixerParams, PluginFormat};
use yinhe_vst3::Vst3Insert;

use super::plugin_instance::{PluginEntry, PluginInstance};

/// 引擎实时渲染块长（yinhe-audio ENGINE_BLOCK_FRAMES），activate 的 max_frames。
/// 必须 ≥ 引擎实际块长，否则 ClapProcessor::process_effect 会截断尾部。
pub(crate) const ACTIVATE_MAX_FRAMES: u32 = 512;

/// 单个 insert 槽位的运行时状态。
pub(crate) struct SlotRuntime {
    /// 机架内全局唯一 id（处理器回收匹配用）。
    pub owner: u64,
    /// None = 加载失败占位：槽位保留（与 InsertRef 链顺序对齐），不参与处理。
    pub instance: Option<PluginInstance>,
    /// 与渲染线程处理器共享的旁通标志。
    pub bypass: Arc<AtomicBool>,
    /// 处理器当前在渲染线程（已 InsertAdd 且未退回）。
    pub sent: bool,
    /// 激活失败过：不再每帧重试（用户移除槽位后重加才会再试）。
    pub activate_failed: bool,
    /// 插件原生 GUI 窗口当前打开中（host 自建窗口嵌入插件 view）。
    pub gui_open: bool,
    /// 宿主侧 GUI 窗口（macOS NSWindow）。必须在 `instance` 之后声明：
    /// 字段按声明顺序 drop，instance 的 Drop 先执行 close_gui（插件 view
    /// 从父 view 移除），之后窗口对象才能释放。
    #[cfg(target_os = "macos")]
    pub gui_window: Option<super::gui_window::PluginGuiWindow>,
    /// 已发 InsertRemove，等待处理器退回后 drop 实例。
    pub pending_remove: bool,
}

/// 一个文档的混音台插件机架。
pub(crate) struct MixerRack {
    /// 源通道 → 槽位实例（稀疏：只有挂插件的通道有条目）。
    pub channels: HashMap<u8, Vec<SlotRuntime>>,
    /// 总线（bus/return）→ 槽位实例（稀疏）。
    pub buses: HashMap<u8, Vec<SlotRuntime>>,
    /// 乐器通道 insert 链（键 = 乐器通道号）。
    pub instruments: HashMap<u16, Vec<SlotRuntime>>,
    /// 音频通道 insert 链（键 = 音频通道号）。
    pub audios: HashMap<u16, Vec<SlotRuntime>>,
    pub master: Vec<SlotRuntime>,
    /// 总线删除时的孤儿槽位：Wait 已发 InsertRemove 的处理器退回后 deactivate。
    orphan: Vec<SlotRuntime>,
    next_owner: u64,
    /// 最近一次加载/激活失败信息（MIX 界面状态行展示）。
    pub last_error: Option<String>,
}

impl Default for MixerRack {
    fn default() -> Self {
        Self {
            channels: HashMap::new(),
            buses: HashMap::new(),
            instruments: HashMap::new(),
            audios: HashMap::new(),
            master: Vec::new(),
            orphan: Vec::new(),
            next_owner: 1,
            last_error: None,
        }
    }
}

pub(crate) fn host_info() -> HostInfo {
    // HostInfo::new 只在校验失败（非法字符等）时报错；这些字符串是编译期常量，
    // fallback 也是常量，理论不可达——但生产路径不留 unwrap（AGENTS 17）。
    HostInfo::new("yinhe", "yinhe", "", env!("CARGO_PKG_VERSION"))
        .unwrap_or_else(|_| HostInfo::new("yinhe", "", "", "0").expect("fallback host info"))
}

/// 插件加载/激活失败（MIX 界面状态行展示）。
pub(crate) struct PluginLoadError(pub String);

impl MixerRack {
    fn chain_mut(&mut self, target: impl Into<InsertTarget>) -> &mut Vec<SlotRuntime> {
        match target.into() {
            InsertTarget::Channel(ch) => self.channels.entry(ch).or_default(),
            InsertTarget::Instrument(ch) => self.instruments.entry(ch).or_default(),
            InsertTarget::Audio(ch) => self.audios.entry(ch).or_default(),
            InsertTarget::Bus(bus) => self.buses.entry(bus).or_default(),
            InsertTarget::Master => &mut self.master,
        }
    }

    pub(crate) fn chain(&self, target: impl Into<InsertTarget>) -> &[SlotRuntime] {
        match target.into() {
            InsertTarget::Channel(ch) => self.channels.get(&ch).map(Vec::as_slice).unwrap_or(&[]),
            InsertTarget::Instrument(ch) => {
                self.instruments.get(&ch).map(Vec::as_slice).unwrap_or(&[])
            }
            InsertTarget::Audio(ch) => self.audios.get(&ch).map(Vec::as_slice).unwrap_or(&[]),
            InsertTarget::Bus(bus) => self.buses.get(&bus).map(Vec::as_slice).unwrap_or(&[]),
            InsertTarget::Master => &self.master,
        }
    }

    /// 按槽位取插件实例（参数面板用）。槽位不存在/无实例返回 None。
    pub(crate) fn instance_mut(
        &mut self,
        target: impl Into<InsertTarget>,
        slot: usize,
    ) -> Option<&mut PluginInstance> {
        self.chain_mut(target).get_mut(slot)?.instance.as_mut()
    }

    /// 在链尾加载插件实例（不激活、不发送——发送走 [`ensure_all_sent`]）。
    ///
    /// `state`/`bypassed` 来自工程加载时的 InsertRef；手动添加传 None/false。
    /// 加载/状态恢复失败时槽位以无实例占位保留（与 InsertRef 链顺序对齐，
    /// 保存不丢引用），错误信息写入 `last_error` 并返回 Err。
    #[allow(clippy::too_many_arguments)] // 上下文透传参数，见 AGENTS 约定
    pub fn load_plugin(
        &mut self,
        target: impl Into<InsertTarget>,
        format: PluginFormat,
        plugin_path: &Path,
        plugin_id: &str,
        name: &str,
        state: Option<&[u8]>,
        bypassed: bool,
    ) -> Result<(), PluginLoadError> {
        let entry = PluginEntry {
            format,
            path: plugin_path.to_path_buf(),
            id: plugin_id.to_string(),
            name: name.to_string(),
            vendor: String::new(),
            is_instrument: false,
            is_effect: false,
            error: None,
        };
        let owner = self.next_owner;
        self.next_owner += 1;
        let bypass = Arc::new(AtomicBool::new(bypassed));
        let result = (|| {
            let mut instance = PluginInstance::load(&entry).map_err(PluginLoadError)?;
            if let Some(bytes) = state {
                instance
                    .load_state(bytes)
                    .map_err(|e| PluginLoadError(format!("恢复插件状态失败: {e}")))?;
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
        let target = target.into();
        self.chain_mut(target).push(SlotRuntime {
            owner,
            instance,
            bypass,
            sent: false,
            activate_failed: false,
            gui_open: false,
            #[cfg(target_os = "macos")]
            gui_window: None,
            pending_remove: false,
        });
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// 激活槽位并发送 InsertAdd。
    fn activate_slot(
        &mut self,
        target: impl Into<InsertTarget>,
        slot: usize,
        handle: &AudioHandle,
        sample_rate: u32,
    ) -> Result<(), PluginLoadError> {
        let target = target.into();
        let rt = &mut self.chain_mut(target)[slot];
        let Some(instance) = rt.instance.as_mut() else {
            return Ok(()); // 加载失败占位槽位：跳过激活
        };
        let processor: Box<dyn InsertProcessor> = match instance {
            PluginInstance::Clap(inst) => {
                let p = inst
                    .activate(sample_rate as f64, ACTIVATE_MAX_FRAMES)
                    .map_err(|e| PluginLoadError(format!("激活插件失败: {e}")))?;
                Box::new(ClapInsert::new(p, Arc::clone(&rt.bypass), rt.owner))
            }
            PluginInstance::Vst3 { instance, .. } => {
                let p = instance
                    .activate_audio(sample_rate as f64, ACTIVATE_MAX_FRAMES)
                    .map_err(|e| PluginLoadError(format!("激活 VST3 插件失败: {e}")))?;
                Box::new(Vst3Insert::new(p))
            }
        };
        handle.send(AudioCommand::InsertAdd {
            target,
            slot,
            processor,
        });
        rt.sent = true;
        Ok(())
    }

    /// 引擎（重）spawn / restart 退回后：补发所有「有实例但未在渲染线程」的槽位。
    pub fn ensure_all_sent(&mut self, handle: &AudioHandle, sample_rate: u32) {
        let mut targets: Vec<(InsertTarget, usize)> = Vec::new();
        for (ch, chain) in &self.channels {
            for (slot, rt) in chain.iter().enumerate() {
                if !rt.sent && !rt.pending_remove && !rt.activate_failed {
                    targets.push((InsertTarget::Channel(*ch), slot));
                }
            }
        }
        for (bus, chain) in &self.buses {
            for (slot, rt) in chain.iter().enumerate() {
                if !rt.sent && !rt.pending_remove && !rt.activate_failed {
                    targets.push((InsertTarget::Bus(*bus), slot));
                }
            }
        }
        for (ch, chain) in &self.instruments {
            for (slot, rt) in chain.iter().enumerate() {
                if !rt.sent && !rt.pending_remove && !rt.activate_failed {
                    targets.push((InsertTarget::Instrument(*ch), slot));
                }
            }
        }
        for (ch, chain) in &self.audios {
            for (slot, rt) in chain.iter().enumerate() {
                if !rt.sent && !rt.pending_remove && !rt.activate_failed {
                    targets.push((InsertTarget::Audio(*ch), slot));
                }
            }
        }
        for (slot, rt) in self.master.iter().enumerate() {
            if !rt.sent && !rt.pending_remove && !rt.activate_failed {
                targets.push((InsertTarget::Master, slot));
            }
        }
        for (target, slot) in targets {
            if let Err(e) = self.activate_slot(target, slot, handle, sample_rate) {
                self.last_error = Some(e.0);
                self.chain_mut(target)[slot].activate_failed = true;
            }
        }
    }

    /// 插件 GUI 状态轮询：插件主动关窗 / 用户关宿主窗口 / 尺寸请求。
    #[cfg(target_os = "macos")]
    fn poll_gui(rt: &mut SlotRuntime) {
        if !rt.gui_open {
            return;
        }
        // 用户点了宿主窗口的关闭按钮：关闭插件 GUI。
        if let Some(win) = rt.gui_window.as_ref()
            && !win.is_visible()
        {
            close_plugin_view(rt);
            rt.gui_window = None;
            rt.gui_open = false;
            return;
        }
        match rt.instance.as_mut() {
            Some(PluginInstance::Clap(instance)) => {
                // 插件侧主动断开（closed 回调）：host destroy 确认 + 释放窗口。
                if instance.take_gui_closed() {
                    instance.on_gui_closed();
                    rt.gui_window = None;
                    rt.gui_open = false;
                    return;
                }
                // 插件请求调整尺寸（如编辑器内部布局变化）。
                if let Some((w, h)) = instance.take_gui_resize()
                    && let Some(win) = rt.gui_window.as_ref()
                {
                    win.set_content_size(w, h);
                }
            }
            Some(PluginInstance::Vst3 { instance, .. }) => {
                // 插件请求调整尺寸：调窗口 + 回调 onSize（VST3 规范）。
                if let Some((w, h)) = instance.take_view_resize() {
                    if let Some(win) = rt.gui_window.as_ref() {
                        win.set_content_size(w, h);
                    }
                    instance.notify_view_resize(w, h);
                }
            }
            None => {}
        }
    }

    /// 非 macOS：GUI 未实现，无轮询。
    #[cfg(not(target_os = "macos"))]
    fn poll_gui(_rt: &mut SlotRuntime) {}

    /// 移除槽位（MIX 界面 ✕）：发 InsertRemove 并把槽位标记为待回收
    /// （处理器退回后 drop 实例）；处理器从未进引擎时直接 drop。
    pub fn remove_slot(
        &mut self,
        target: impl Into<InsertTarget>,
        slot: usize,
        handle: Option<&AudioHandle>,
    ) {
        let target = target.into();
        if slot >= self.chain(target).len() {
            return;
        }
        if self.chain(target)[slot].sent {
            if let Some(h) = handle {
                h.send(AudioCommand::InsertRemove { target, slot });
            }
            self.chain_mut(target)[slot].pending_remove = true;
        } else {
            self.chain_mut(target).remove(slot);
        }
    }

    /// 切换旁通：只动共享原子（渲染线程下一块生效）；持久化层由调用方写。
    pub fn set_bypass(&mut self, target: impl Into<InsertTarget>, slot: usize, bypassed: bool) {
        if let Some(rt) = self.chain_mut(target).get_mut(slot) {
            rt.bypass.store(bypassed, Ordering::Relaxed);
        }
    }

    /// 打开/关闭插件原生界面（host 自建窗口 + 插件 view 嵌入）。
    /// CLAP / VST3 共用宿主 NSWindow。
    #[cfg(target_os = "macos")]
    pub fn toggle_gui(
        &mut self,
        target: impl Into<InsertTarget>,
        slot: usize,
    ) -> Result<bool, PluginLoadError> {
        let Some(rt) = self.chain_mut(target).get_mut(slot) else {
            return Ok(false);
        };
        if rt.instance.is_none() {
            tracing::warn!("打开插件界面失败: 槽位无实例（插件未加载成功）");
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
        let (name, size_result): (String, Result<(u32, u32), String>) = match rt.instance.as_mut() {
            Some(PluginInstance::Clap(inst)) => (
                inst.info().name.clone(),
                inst.create_gui().map_err(|e| format!("{e}")),
            ),
            Some(PluginInstance::Vst3 { instance, name, .. }) => (
                name.clone(),
                instance.create_view().map_err(|e| format!("{e}")),
            ),
            None => return Err(PluginLoadError("插件未加载成功，无法打开界面".into())),
        };
        let (w, h) = size_result.map_err(|e| {
            tracing::warn!("插件界面创建失败 ({name}): {e}");
            PluginLoadError(e)
        })?;
        tracing::info!("插件界面创建中: {name} {w}x{h}");
        let Some(win) = super::gui_window::PluginGuiWindow::new(&name, w, h) else {
            close_plugin_view(rt);
            tracing::warn!("插件窗口创建失败: {name}");
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
            None => Err("实例丢失".into()),
        };
        if let Err(e) = attach_result {
            close_plugin_view(rt);
            tracing::warn!("插件界面嵌入失败 ({name}): {e}");
            return Err(PluginLoadError(e));
        }
        win.show();
        tracing::info!("插件界面已显示: {name}");
        rt.gui_window = Some(win);
        rt.gui_open = true;
        Ok(true)
    }

    /// 非 macOS：原生 GUI 尚未实现。
    #[cfg(not(target_os = "macos"))]
    pub fn toggle_gui(
        &mut self,
        _target: impl Into<InsertTarget>,
        _slot: usize,
    ) -> Result<bool, PluginLoadError> {
        Err(PluginLoadError("当前平台暂不支持插件界面".into()))
    }

    /// 删除总线的机架侧处理：发 InsertRemove 回收处理器、更高 bus 键前移一位
    /// （与 `MixerParams::remove_bus` 的路由语义保持一致）。
    ///
    /// 已发送的槽位移入 orphan，等处理器退回后 deactivate（bus 号已失效，
    /// 按 owner 匹配）。
    pub fn remove_bus_chain(&mut self, bus: u8, handle: Option<&AudioHandle>) {
        if let Some(chain) = self.buses.remove(&bus) {
            for (slot, rt) in chain.into_iter().enumerate() {
                if rt.sent {
                    if let Some(h) = handle {
                        h.send(AudioCommand::InsertRemove {
                            target: InsertTarget::Bus(bus),
                            slot,
                        });
                    }
                    self.orphan.push(rt);
                }
                // 未发送的槽位直接 drop（插件从未进入引擎）。
            }
        }
        let mut moved: Vec<(u8, Vec<SlotRuntime>)> = Vec::new();
        let keys: Vec<u8> = self.buses.keys().copied().filter(|b| *b > bus).collect();
        for b in keys {
            if let Some(chain) = self.buses.remove(&b) {
                moved.push((b - 1, chain));
            }
        }
        for (b, chain) in moved {
            self.buses.insert(b, chain);
        }
    }

    /// 处理渲染线程退回的处理器：downcast → 按 owner 匹配 → deactivate。
    /// 引擎 teardown 时会把全部 insert 退回，这里统一回收；回收后
    /// `sent = false`，由 `ensure_all_sent` 在新引擎上补发。
    pub fn on_returns(&mut self, returned: Vec<Box<dyn InsertProcessor>>) {
        for boxed in returned {
            let any = boxed.into_any();
            match any.downcast::<ClapInsert>() {
                Ok(insert) => {
                    let (processor, _bypass, owner) = insert.into_parts();
                    self.return_processor(owner, processor);
                }
                Err(any) => match any.downcast::<Vst3Insert>() {
                    Ok(insert) => {
                        // VST3：处理器独占，stop()（关激活）后直接释放。
                        let processor = insert.into_processor();
                        processor.stop();
                    }
                    Err(_) => tracing::warn!("退回的 insert 处理器类型未知，丢弃"),
                },
            }
        }
    }

    fn return_processor(&mut self, owner: u64, processor: ClapProcessor) {
        // 孤儿槽位（总线删除后等退回）优先匹配。
        if let Some(idx) = self.orphan.iter().position(|rt| rt.owner == owner) {
            let mut rt = self.orphan.remove(idx);
            if let Some(PluginInstance::Clap(instance)) = rt.instance.as_mut() {
                instance.deactivate(processor);
            }
            return;
        }
        // 找槽位（channels + buses + master 线性扫，槽位数很小）。
        let mut found: Option<(InsertTarget, usize)> = None;
        for (ch, chain) in &self.channels {
            if let Some(slot) = chain.iter().position(|rt| rt.owner == owner) {
                found = Some((InsertTarget::Channel(*ch), slot));
                break;
            }
        }
        if found.is_none() {
            for (bus, chain) in &self.buses {
                if let Some(slot) = chain.iter().position(|rt| rt.owner == owner) {
                    found = Some((InsertTarget::Bus(*bus), slot));
                    break;
                }
            }
        }
        if found.is_none()
            && let Some(slot) = self.master.iter().position(|rt| rt.owner == owner)
        {
            found = Some((InsertTarget::Master, slot));
        }
        let Some((target, slot)) = found else {
            tracing::warn!("退回的处理器 owner={owner} 找不到槽位，丢弃");
            return;
        };
        {
            let rt = &mut self.chain_mut(target)[slot];
            if let Some(PluginInstance::Clap(instance)) = rt.instance.as_mut() {
                instance.deactivate(processor);
            } else {
                tracing::warn!("owner={owner} 的槽位无 CLAP 实例，处理器无法 deactivate，丢弃");
            }
            rt.sent = false;
        }
        if self.chain(target)[slot].pending_remove {
            self.chain_mut(target).remove(slot);
        }
    }

    /// 每帧轮询插件反向请求（CLAP/VST3 统一）：
    /// - restart / I/O 变化 → 走两阶段回收（先 InsertRemove，退回后由
    ///   ensure_all_sent 重新激活补发）；
    /// - 参数重扫 → 实例内暂存，参数面板刷新时消费；
    /// - 延迟变化 → CLAP 在此重查共享值，然后通知引擎重算 PDC。
    ///
    /// 轮询不看 `sent`（未发送实例的 rescan/latency 也要消费，否则标志永久
    /// 挂起）；restart 仅在 `sent` 时走回收（未发送的由 ensure_all_sent 激活）。
    pub fn poll_requests(&mut self, handle: Option<&AudioHandle>) {
        let mut restarts: Vec<(InsertTarget, usize)> = Vec::new();
        let mut latency_changed = false;
        for (ch, chain) in self.channels.iter_mut() {
            for (slot, rt) in chain.iter_mut().enumerate() {
                if rt.pending_remove {
                    continue;
                }
                Self::poll_gui(rt);
                let Some(instance) = rt.instance.as_mut() else {
                    continue;
                };
                let requests = instance.poll_requests();
                if requests.latency_changed {
                    latency_changed = true;
                }
                if requests.restart && rt.sent {
                    restarts.push((InsertTarget::Channel(*ch), slot));
                }
            }
        }
        for (bus, chain) in self.buses.iter_mut() {
            for (slot, rt) in chain.iter_mut().enumerate() {
                if rt.pending_remove {
                    continue;
                }
                Self::poll_gui(rt);
                let Some(instance) = rt.instance.as_mut() else {
                    continue;
                };
                let requests = instance.poll_requests();
                if requests.latency_changed {
                    latency_changed = true;
                }
                if requests.restart && rt.sent {
                    restarts.push((InsertTarget::Bus(*bus), slot));
                }
            }
        }
        for (slot, rt) in self.master.iter_mut().enumerate() {
            if rt.pending_remove {
                continue;
            }
            Self::poll_gui(rt);
            let Some(instance) = rt.instance.as_mut() else {
                continue;
            };
            let requests = instance.poll_requests();
            if requests.latency_changed {
                latency_changed = true;
            }
            if requests.restart && rt.sent {
                restarts.push((InsertTarget::Master, slot));
            }
        }
        if let Some(h) = handle {
            for (target, slot) in restarts {
                h.send(AudioCommand::InsertRemove { target, slot });
            }
            if latency_changed {
                h.send(AudioCommand::RefreshLatency);
            }
        }
    }

    /// 保存前把实例状态写回持久化层（InsertRef.state / bypassed）。
    /// pending_remove 槽位跳过（其 InsertRef 已在移除时先删，保持两边顺序对齐）。
    pub fn sync_states_to(&mut self, mixer: &mut MixerParams) {
        for (ch, chain) in self.channels.iter_mut() {
            let Some(refs) = mixer.channel_inserts.get_mut(*ch as usize) else {
                continue;
            };
            sync_chain(chain.iter_mut(), refs.iter_mut());
        }
        for (bus, chain) in self.buses.iter_mut() {
            let Some(refs) = mixer.bus_inserts.get_mut(*bus as usize) else {
                continue;
            };
            sync_chain(chain.iter_mut(), refs.iter_mut());
        }
        sync_chain(self.master.iter_mut(), mixer.master_inserts.iter_mut());
    }
}

/// 关闭槽位当前插件的原生 view（CLAP/VST3 分派）。
#[cfg(target_os = "macos")]
fn close_plugin_view(rt: &mut SlotRuntime) {
    match rt.instance.as_mut() {
        Some(PluginInstance::Clap(inst)) => inst.close_gui(),
        Some(PluginInstance::Vst3 { instance, .. }) => instance.close_view(),
        None => {}
    }
}

fn sync_chain<'a>(
    chain: impl Iterator<Item = &'a mut SlotRuntime>,
    refs: impl Iterator<Item = &'a mut yinhe_mixer::InsertRef>,
) {
    let mut slots = chain.filter(|rt| !rt.pending_remove);
    for insert_ref in refs {
        let Some(rt) = slots.next() else { break };
        insert_ref.bypassed = rt.bypass.load(Ordering::Relaxed);
        // 加载失败的占位槽位无实例：保留工程里的旧 state。
        let Some(instance) = rt.instance.as_mut() else {
            continue;
        };
        // 无状态扩展/失败时返回 None，保留旧 state。
        if let Some(bytes) = instance.save_state() {
            insert_ref.state = Some(bytes);
        }
    }
}
