//! 引擎的混音台接线：MixerParams（源通道索引）↔ MixerGraph（dense 索引）映射，
//! insert 命令处理与处理器回收。

use yinhe_mixer::{
    InsertProcessor, InstrumentProcessor, MasterParams, MixerParams, SendParams, StripParams,
};

use crate::channel_layout::ChannelNamespace;
use crate::engine::AudioEngine;
use crate::spawn::InsertTarget;

impl AudioEngine {
    /// 全量同步混音台参数（引擎 spawn/工程加载后由 UI 推一次）。
    /// 只推 strip/master 参数；insert 处理器走 Insert* 命令单独进。
    pub(crate) fn set_mixer_params(&mut self, mut params: MixerParams) {
        // 防御：通道表补齐到当前布局需要的长度（只增不减，保留下线通道设置）。
        params.ensure_channel_tables(self.channel_layout.audio_channels().len());
        params.ensure_len();
        self.mixer_params = params;
        let strips = self.dense_strip_params();
        for (dense, p) in strips.into_iter().enumerate() {
            self.mixer.set_strip(dense, p);
        }
        self.mixer.set_master(self.mixer_params.master);
        // 总线与发送：全量同步（数量变化走 resize_buses）。
        let buses = self.mixer_params.buses.clone();
        self.mixer
            .resize_buses(buses.len(), self.mixer.frames(), &buses);
        for (i, p) in buses.into_iter().enumerate() {
            self.mixer.set_bus_strip(i, p);
        }
        self.sync_sends_to_graph();
    }

    /// 把按通道命名空间索引的发送列表映射到 dense 通道并推给混音图。
    /// 未激活通道的 send 暂存于持久化层，引擎重建/激活后由全量同步补上。
    fn sync_sends_to_graph(&mut self) {
        let sends = self.mixer_params.sends.clone();
        let audio_sends = self.mixer_params.audio_sends.clone();
        let count = self.channel_set.channel_count();
        for dense in 0..count {
            let list = self
                .dense_to_namespace(dense)
                .and_then(|ns| match ns {
                    ChannelNamespace::Midi(src) => sends.get(src),
                    ChannelNamespace::Audio(ach) => audio_sends.get(ach),
                })
                .cloned()
                .unwrap_or_default();
            self.mixer.set_sends(dense, list);
        }
    }

    /// dense 索引 → 所属通道命名空间（仅结构性操作时调用，O(1)）。
    fn dense_to_namespace(&self, dense: usize) -> Option<ChannelNamespace> {
        let d = dense as u32;
        if d >= self.channel_layout.midi_compacted() {
            let idx = (d - self.channel_layout.midi_compacted()) as usize;
            self.channel_layout
                .audio_channels()
                .get(idx)
                .map(|&ach| ChannelNamespace::Audio(ach as usize))
        } else {
            self.channel_layout
                .channel_map()
                .iter()
                .position(|&x| x as usize == dense)
                .map(ChannelNamespace::Midi)
        }
    }

    /// 更新某总线的 strip 参数（高频路径）。
    pub(crate) fn set_bus_strip(&mut self, bus: u8, params: StripParams) {
        let idx = bus as usize;
        if let Some(slot) = self.mixer_params.buses.get_mut(idx) {
            *slot = params;
        }
        self.mixer.set_bus_strip(idx, params);
    }

    /// 全量同步总线参数与发送（增删总线 / 改 send 后推一次）。
    pub(crate) fn sync_bus_config(&mut self, buses: Vec<StripParams>, sends: Vec<Vec<SendParams>>) {
        self.mixer_params.buses = buses;
        self.mixer_params.sends = sends;
        let buses = self.mixer_params.buses.clone();
        self.mixer
            .resize_buses(buses.len(), self.mixer.frames(), &buses);
        for (i, p) in buses.into_iter().enumerate() {
            self.mixer.set_bus_strip(i, p);
        }
        self.sync_sends_to_graph();
    }

    /// 更新某源通道的 strip（推子/声像/M/S 拖动的高频路径，幂等）。
    pub(crate) fn set_channel_strip(&mut self, channel: u8, params: StripParams) {
        if let Some(slot) = self.mixer_params.channels.get_mut(channel as usize) {
            *slot = params;
        }
        let dense = self.channel_layout.dense_for(channel as usize);
        if dense != u32::MAX {
            self.mixer.set_strip(dense as usize, params);
        }
    }

    pub(crate) fn set_master_params(&mut self, params: MasterParams) {
        self.mixer_params.master = params;
        self.mixer.set_master(params);
    }

    /// 更新某音频通道的 strip（推子/声像/M/S 拖动的高频路径，幂等）。
    pub(crate) fn set_audio_strip(&mut self, channel: u16, params: StripParams) {
        if let Some(slot) = self.mixer_params.audio_channels.get_mut(channel as usize) {
            *slot = params;
        }
        let dense = self.channel_layout.audio_dense_for(channel);
        if dense != u32::MAX {
            self.mixer.set_strip(dense as usize, params);
        }
    }

    /// 各 dense 通道当前的 strip 参数（resize 重建 strip 状态用）。
    /// MIDI 源通道按 channel_map 反查，音频通道按 dense 段位置索引。
    pub(crate) fn dense_strip_params(&self) -> Vec<StripParams> {
        (0..self.channel_set.channel_count())
            .map(|dense| match self.dense_to_namespace(dense) {
                Some(ChannelNamespace::Midi(src)) => self.mixer_params.strip(src as u8),
                Some(ChannelNamespace::Audio(ach)) => self.mixer_params.audio_strip(ach as u16),
                None => StripParams::default(),
            })
            .collect()
    }

    /// 源通道 → dense 索引（未激活返回 None）。
    fn dense_of(&self, channel: u8) -> Option<usize> {
        let dense = self.channel_layout.dense_for(channel as usize);
        (dense != u32::MAX).then_some(dense as usize)
    }

    pub(crate) fn insert_add(
        &mut self,
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        match target {
            InsertTarget::Channel(ch) => match self.dense_of(ch) {
                Some(dense) => self.mixer.insert_insert(dense, slot, processor),
                // 通道未激活（模型无音轨用此通道）：处理器无处安放，直接退回。
                None => self.insert_returns.push(processor),
            },
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                if dense != u32::MAX {
                    self.mixer.insert_insert(dense as usize, slot, processor);
                } else {
                    self.insert_returns.push(processor);
                }
            }
            InsertTarget::Bus(bus) => self.mixer.insert_bus_insert(bus as usize, slot, processor),
            InsertTarget::Master => self.mixer.insert_master_insert(slot, processor),
        }
    }

    pub(crate) fn insert_remove(&mut self, target: InsertTarget, slot: usize) {
        let removed = match target {
            InsertTarget::Channel(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.remove_insert(dense, slot)),
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                (dense != u32::MAX)
                    .then(|| self.mixer.remove_insert(dense as usize, slot))
                    .flatten()
            }
            InsertTarget::Bus(bus) => self.mixer.remove_bus_insert(bus as usize, slot),
            InsertTarget::Master => self.mixer.remove_master_insert(slot),
        };
        if let Some(p) = removed {
            self.insert_returns.push(p);
        }
    }

    pub(crate) fn insert_replace(
        &mut self,
        target: InsertTarget,
        slot: usize,
        processor: Box<dyn InsertProcessor>,
    ) {
        let old = match target {
            InsertTarget::Channel(ch) => self
                .dense_of(ch)
                .and_then(|dense| self.mixer.replace_insert(dense, slot, processor)),
            InsertTarget::Audio(ch) => {
                let dense = self.channel_layout.audio_dense_for(ch);
                (dense != u32::MAX)
                    .then(|| self.mixer.replace_insert(dense as usize, slot, processor))
                    .flatten()
            }
            InsertTarget::Bus(bus) => self.mixer.replace_bus_insert(bus as usize, slot, processor),
            InsertTarget::Master => self.mixer.replace_master_insert(slot, processor),
        };
        if let Some(p) = old {
            self.insert_returns.push(p);
        }
    }

    /// 取出待回收的 insert 处理器（renderer 每轮命令处理后调用，送回 UI 线程）。
    pub(crate) fn drain_insert_returns(&mut self) -> Vec<Box<dyn InsertProcessor>> {
        std::mem::take(&mut self.insert_returns)
    }

    /// 安装/替换/移除某 MIDI 通道上的乐器插件实例（CLAP/VST3 等，抽象为 trait）。
    /// 移除（`processor = None`）后该通道回到默认 XSynth。
    ///
    /// 由 `AudioCommand::SetInstrument` 触发，渲染线程调用。被替换/移除的旧
    /// 处理器（以及通道未激活却收到安装命令的多余处理器）攒进 `instrument_returns`
    /// 送回 UI 线程 deactivate——渲染线程不能 deactivate 插件。
    pub(crate) fn set_instrument(
        &mut self,
        channel: u8,
        processor: Option<Box<dyn InstrumentProcessor>>,
    ) {
        let dense = self.channel_layout.dense_for(channel as usize);
        let Some(dense) = (dense != u32::MAX).then_some(dense as usize) else {
            if let Some(p) = processor {
                self.instrument_returns.push((channel, p));
            }
            return;
        };
        if dense >= self.instruments.len() {
            // 命令与模型不同步（dense 越界）：直接退回，不越界写。
            if let Some(p) = processor {
                self.instrument_returns.push((channel, p));
            }
            return;
        }
        // 乐器延迟（PDC）：安装时查询一次，用于该通道延迟补偿。
        let latency = processor.as_ref().map(|p| p.latency_samples()).unwrap_or(0);
        let old = std::mem::replace(
            &mut self.instruments[dense],
            processor.map(|p| crate::instrument::InstrumentSource::new(channel, p)),
        );
        self.mixer.set_channel_latency(dense, latency);
        if let Some(old) = old {
            self.instrument_returns.push((old.channel, old.processor));
        }
    }

    /// 取出待回收的乐器处理器（renderer 每轮命令处理后调用，送回 UI 线程）。
    pub(crate) fn drain_instrument_returns(&mut self) -> Vec<(u8, Box<dyn InstrumentProcessor>)> {
        std::mem::take(&mut self.instrument_returns)
    }

    /// 插件延迟变化：重新查询乐器延迟并重算 PDC 对齐（insert 链延迟由
    /// `refresh_pdc` 内部向各处理器查询最新值）。
    pub(crate) fn refresh_latency(&mut self) {
        for dense in 0..self.instruments.len() {
            let latency = self.instruments[dense]
                .as_ref()
                .map(|src| src.processor.latency_samples());
            if let Some(latency) = latency {
                self.mixer.set_channel_latency(dense, latency);
            }
        }
        self.mixer.refresh_pdc();
    }
}
