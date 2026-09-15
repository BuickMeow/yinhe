//! 混音处理图（渲染线程持有，处理期间零分配、零锁）。
//!
//! 信号流（每块）：
//!   每通道：上层把音源渲染进通道缓冲 → insert 链 → 增益/声像斜坡 → 累加进主输出
//!   主输出：master insert 链 → master 增益 → 电平表
//!
//! 所有缓冲在 [`MixerGraph::resize`] 时一次性分配，之后处理不再分配。
//!
//! 内部按「平行数组」组织（buffers/strips/inserts/meters 四个等长 Vec），
//! 以便渲染线程把 buffers 整体借出做跨通道并行渲染（rayon）。

use crate::meter::{MeterReading, MeterTap};
use crate::params::{MasterParams, SendParams, StripParams};
use crate::strip::StripState;

/// insert 效果器抽象。由 yinhe-audio 把 CLAP（未来 VST3）处理器适配进来。
///
/// 实现者要求：
/// - `process` 内不得分配内存、不得阻塞（渲染线程实时约束）；
/// - 原地处理 `left`/`right`（长度相等，等于块长）。
pub trait InsertProcessor: Send {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);

    /// 清空内部处理状态（envelope、delay 尾音等）。seek 后调用。
    fn reset(&mut self) {}

    /// 暂停/停止时把待发参数送达插件（输出丢弃）。播放时由 `process` 携带，
    /// 无需调用。默认无操作。
    fn flush_pending_params(&mut self, _position_samples: u64) {}

    /// 插件报告的延迟（采样数），供延迟补偿（PDC）用。
    fn latency_samples(&self) -> u32 {
        0
    }

    /// 本处理器接管的 MIDI CC 号（默认空）。挂在通道 insert 链上时生效：
    /// 宿主把被接管的 CC 分流到 [`InsertProcessor::apply_cc`]，
    /// 不再下发给合成器/乐器插件（见 `docs/spec-yinhe-dsp.md` §5.3）。
    fn handled_ccs(&self) -> &'static [u8] {
        &[]
    }

    /// 接收被接管的 CC 值（0..127）。仅对 `handled_ccs` 中的 CC 调用。
    fn apply_cc(&mut self, _cc: u8, _value: u8) {}

    /// 回收时还原为具体类型（如插件处理器需要 deactivate 回实例）。
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
}

/// 单通道延迟线（PDC 对齐）：先读后写，`delay` 个样本后输出输入，
/// 起始输出为静音。`delay == 0` 时直通（不分配缓冲）。
struct DelayLine {
    left: Vec<f32>,
    right: Vec<f32>,
    write: usize,
}

impl DelayLine {
    fn new() -> Self {
        Self {
            left: Vec::new(),
            right: Vec::new(),
            write: 0,
        }
    }

    #[cfg(test)]
    fn delay(&self) -> usize {
        self.left.len()
    }

    /// 设定延迟样本数（变化时重建缓冲；仅命令处理阶段调用，允许分配）。
    fn set_delay(&mut self, delay: usize) {
        if self.left.len() != delay {
            self.left = vec![0.0; delay];
            self.right = vec![0.0; delay];
            self.write = 0;
        }
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.left.is_empty() {
            return;
        }
        let len = self.left.len();
        for i in 0..left.len() {
            let out_l = self.left[self.write];
            let out_r = self.right[self.write];
            self.left[self.write] = left[i];
            self.right[self.write] = right[i];
            left[i] = out_l;
            right[i] = out_r;
            self.write += 1;
            if self.write == len {
                self.write = 0;
            }
        }
    }

    fn clear(&mut self) {
        self.left.fill(0.0);
        self.right.fill(0.0);
        self.write = 0;
    }
}

/// 一条通道的立体声缓冲（planar），供上层音源渲染写入。
pub struct ChannelBuffers {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

/// 混音处理图。只在渲染线程使用，不实现 Clone。
pub struct MixerGraph {
    buffers: Vec<ChannelBuffers>,
    strips: Vec<StripState>,
    inserts: Vec<Vec<Box<dyn InsertProcessor>>>,
    /// 每通道的 PDC 对齐延迟线（补偿到最长路径；长度 0 = 直通）。
    delays: Vec<DelayLine>,
    /// 每通道的基础延迟（乐器插件上报的采样数，由引擎设置）。
    base_latency: Vec<u32>,
    meters: Vec<MeterTap>,
    /// 与 meters 一一对应的 UI 侧读数端（引擎创建时被上层取走克隆）。
    meter_readings: Vec<MeterReading>,
    master_l: Vec<f32>,
    master_r: Vec<f32>,
    master_gain: f32,
    master_prev_gain: f32,
    master_inserts: Vec<Box<dyn InsertProcessor>>,
    master_meter: MeterTap,
    master_reading: MeterReading,
    /// 总线（bus / return）缓冲：每 bus 一对立体声（求和点在 master 之前）。
    bus_buffers: Vec<ChannelBuffers>,
    bus_strips: Vec<StripState>,
    bus_inserts: Vec<Vec<Box<dyn InsertProcessor>>>,
    bus_meters: Vec<MeterTap>,
    bus_readings: Vec<MeterReading>,
    /// 每通道（dense）的发送列表（源通道 → 总线）。
    sends: Vec<Vec<SendParams>>,
    frames: usize,
}

impl MixerGraph {
    /// 创建空图（0 通道）。通道数/块长变化走 [`resize`](Self::resize)。
    pub fn new(frames: usize) -> Self {
        let (master_meter, master_reading) = MeterTap::new();
        Self {
            buffers: Vec::new(),
            strips: Vec::new(),
            inserts: Vec::new(),
            delays: Vec::new(),
            base_latency: Vec::new(),
            meters: Vec::new(),
            meter_readings: Vec::new(),
            master_l: vec![0.0; frames],
            master_r: vec![0.0; frames],
            master_gain: 1.0,
            master_prev_gain: 1.0,
            master_inserts: Vec::new(),
            master_meter,
            master_reading,
            bus_buffers: Vec::new(),
            bus_strips: Vec::new(),
            bus_inserts: Vec::new(),
            bus_meters: Vec::new(),
            bus_readings: Vec::new(),
            sends: Vec::new(),
            frames,
        }
    }

    /// 重建通道缓冲。仅在引擎创建/块长变化时调用（会分配内存）。
    ///
    /// 已有通道的 strip/insert/meter 状态按索引保留，新增通道用
    /// `strips`（不足补默认值）。master 参数用 [`set_master`](Self::set_master) 单独推。
    pub fn resize(&mut self, channel_count: usize, frames: usize, strips: &[StripParams]) {
        self.frames = frames;
        self.master_l = vec![0.0; frames];
        self.master_r = vec![0.0; frames];

        let n_old = self.buffers.len().min(channel_count);
        let mut buffers = Vec::with_capacity(channel_count);
        buffers.append(&mut self.buffers);
        buffers.truncate(n_old);
        for b in buffers.iter_mut() {
            b.left = vec![0.0; frames];
            b.right = vec![0.0; frames];
        }
        self.strips.truncate(n_old);
        self.inserts.truncate(n_old);
        self.delays.truncate(n_old);
        self.base_latency.truncate(n_old);
        self.meters.truncate(n_old);
        self.meter_readings.truncate(n_old);
        self.sends.truncate(n_old);
        for i in n_old..channel_count {
            buffers.push(ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            });
            self.strips
                .push(StripState::new(strips.get(i).copied().unwrap_or_default()));
            self.inserts.push(Vec::new());
            self.delays.push(DelayLine::new());
            self.base_latency.push(0);
            self.sends.push(Vec::new());
            let (tap, reading) = MeterTap::new();
            self.meters.push(tap);
            self.meter_readings.push(reading);
        }
        self.buffers = buffers;
        self.refresh_pdc();
    }

    /// 通道数。
    pub fn channel_count(&self) -> usize {
        self.buffers.len()
    }

    /// 当前块长（帧）。
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// 是否有任何 insert 处理器（含 master；导出的尾音判断用）。
    pub fn has_inserts(&self) -> bool {
        !self.master_inserts.is_empty()
            || self.inserts.iter().any(|c| !c.is_empty())
            || self.bus_inserts.iter().any(|c| !c.is_empty())
    }

    /// 整体借出通道缓冲：渲染线程跨通道并行写入音源（每通道一个 rayon 任务）。
    /// 每块开始前上层应自行清零或完全覆盖。
    pub fn buffers_mut(&mut self) -> &mut [ChannelBuffers] {
        &mut self.buffers
    }

    /// 取单条通道缓冲供音源写入（非并行路径用）。
    pub fn channel_buffers_mut(&mut self, channel: usize) -> Option<&mut ChannelBuffers> {
        self.buffers.get_mut(channel)
    }

    /// 清空全部通道缓冲。空闲渲染（停止状态驱动乐器插件）用：
    /// 该路径没有 xsynth/音频轨覆盖写，只有插件写自己的通道缓冲。
    pub fn clear_channel_buffers(&mut self) {
        for cb in &mut self.buffers {
            cb.left.fill(0.0);
            cb.right.fill(0.0);
        }
    }

    /// 更新某通道的 strip 目标参数（推子拖动等高频操作直接调这个，幂等）。
    pub fn set_strip(&mut self, channel: usize, params: StripParams) {
        if let Some(s) = self.strips.get_mut(channel) {
            s.set_params(params);
        }
    }

    pub fn set_master(&mut self, params: MasterParams) {
        self.master_gain = params.gain;
    }

    /// 替换某通道 insert 链（新链在上层构建好后整体换入），返回旧链。
    /// 旧链（插件处理器等）需要在上层线程回收（deactivate），不能直接 drop
    /// 在渲染线程——调用方负责把返回值送回去。
    pub fn set_inserts(
        &mut self,
        channel: usize,
        inserts: Vec<Box<dyn InsertProcessor>>,
    ) -> Vec<Box<dyn InsertProcessor>> {
        let old = if let Some(slot) = self.inserts.get_mut(channel) {
            std::mem::replace(slot, inserts)
        } else {
            inserts
        };
        self.refresh_pdc();
        old
    }

    pub fn set_master_inserts(
        &mut self,
        inserts: Vec<Box<dyn InsertProcessor>>,
    ) -> Vec<Box<dyn InsertProcessor>> {
        std::mem::replace(&mut self.master_inserts, inserts)
    }

    /// 在槽位 `slot` 处插入一个处理器（链尾之后则追加）。
    pub fn insert_insert(&mut self, channel: usize, slot: usize, p: Box<dyn InsertProcessor>) {
        if let Some(chain) = self.inserts.get_mut(channel) {
            chain.insert(slot.min(chain.len()), p);
            self.refresh_pdc();
        }
    }

    /// 移除并返回槽位 `slot` 的处理器（上层回收 deactivate）。
    pub fn remove_insert(
        &mut self,
        channel: usize,
        slot: usize,
    ) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.inserts.get_mut(channel)?;
        let removed = (slot < chain.len()).then(|| chain.remove(slot));
        if removed.is_some() {
            self.refresh_pdc();
        }
        removed
    }

    /// 替换槽位 `slot` 的处理器，返回旧的（插件请求 restart 时用）。
    pub fn replace_insert(
        &mut self,
        channel: usize,
        slot: usize,
        p: Box<dyn InsertProcessor>,
    ) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.inserts.get_mut(channel)?;
        let old = chain.get_mut(slot).map(|old| std::mem::replace(old, p));
        if old.is_some() {
            self.refresh_pdc();
        }
        old
    }

    /// 把一段 CC 直接广播给该通道 insert 链上处理它的模块（yinhe-dsp）。
    ///
    /// 对应"CC 效果直接进 DSP 链"：dispatch 对通道级 CC 调用本方法，
    /// 不再下发合成器；模块按自己的 `handled_ccs` 过滤，无关 CC 被忽略。
    pub fn broadcast_channel_cc(&mut self, channel: usize, cc: u8, value: u8) {
        if let Some(chain) = self.inserts.get_mut(channel) {
            for p in chain.iter_mut() {
                if p.handled_ccs().contains(&cc) {
                    p.apply_cc(cc, value);
                }
            }
        }
    }

    /// 在 master 链槽位 `slot` 处插入处理器（越界则追加）。
    pub fn insert_master_insert(&mut self, slot: usize, p: Box<dyn InsertProcessor>) {
        self.master_inserts
            .insert(slot.min(self.master_inserts.len()), p);
    }

    /// 移除并返回 master 链槽位 `slot` 的处理器。
    pub fn remove_master_insert(&mut self, slot: usize) -> Option<Box<dyn InsertProcessor>> {
        (slot < self.master_inserts.len()).then(|| self.master_inserts.remove(slot))
    }

    /// 替换 master 链槽位 `slot` 的处理器，返回旧的。
    pub fn replace_master_insert(
        &mut self,
        slot: usize,
        p: Box<dyn InsertProcessor>,
    ) -> Option<Box<dyn InsertProcessor>> {
        self.master_inserts
            .get_mut(slot)
            .map(|old| std::mem::replace(old, p))
    }

    /// 重建总线数量/块长（增删总线或块长变化时调用；会分配内存）。
    /// 已有总线的 strip 状态按索引保留，新增总线用 `params`（不足补默认）。
    pub fn resize_buses(&mut self, count: usize, frames: usize, params: &[StripParams]) {
        while self.bus_buffers.len() > count {
            self.bus_buffers.pop();
            self.bus_strips.pop();
            self.bus_inserts.pop();
            self.bus_meters.pop();
            self.bus_readings.pop();
        }
        for b in &mut self.bus_buffers {
            b.left.resize(frames, 0.0);
            b.right.resize(frames, 0.0);
        }
        while self.bus_buffers.len() < count {
            self.bus_buffers.push(ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            });
            let i = self.bus_strips.len();
            self.bus_strips
                .push(StripState::new(params.get(i).copied().unwrap_or_default()));
            self.bus_inserts.push(Vec::new());
            let (tap, reading) = MeterTap::new();
            self.bus_meters.push(tap);
            self.bus_readings.push(reading);
        }
    }

    /// 总线数量。
    pub fn bus_count(&self) -> usize {
        self.bus_buffers.len()
    }

    /// 更新某总线的 strip 参数（推子拖动高频路径，幂等）。
    pub fn set_bus_strip(&mut self, bus: usize, params: StripParams) {
        if let Some(s) = self.bus_strips.get_mut(bus) {
            s.set_params(params);
        }
    }

    /// 总线电平表读数端（UI 线程持有克隆）。
    pub fn bus_meter_reading(&self, bus: usize) -> Option<MeterReading> {
        self.bus_readings.get(bus).cloned()
    }

    /// 设置某通道（dense）的发送列表（结构性变化时全量推）。
    pub fn set_sends(&mut self, channel: usize, sends: Vec<SendParams>) {
        if let Some(slot) = self.sends.get_mut(channel) {
            *slot = sends;
        }
    }

    /// 在总线链槽位插入处理器（越界则追加）。
    pub fn insert_bus_insert(&mut self, bus: usize, slot: usize, p: Box<dyn InsertProcessor>) {
        if let Some(chain) = self.bus_inserts.get_mut(bus) {
            chain.insert(slot.min(chain.len()), p);
        }
    }

    /// 移除并返回总线链槽位的处理器。
    pub fn remove_bus_insert(
        &mut self,
        bus: usize,
        slot: usize,
    ) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.bus_inserts.get_mut(bus)?;
        (slot < chain.len()).then(|| chain.remove(slot))
    }

    /// 替换总线链槽位的处理器，返回旧的。
    pub fn replace_bus_insert(
        &mut self,
        bus: usize,
        slot: usize,
        p: Box<dyn InsertProcessor>,
    ) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.bus_inserts.get_mut(bus)?;
        chain.get_mut(slot).map(|old| std::mem::replace(old, p))
    }

    /// 设置某通道的基础延迟（乐器插件上报的采样数；0 = 无延迟）。
    /// 变化时重算 PDC 对齐（重建延迟线缓冲）。
    pub fn set_channel_latency(&mut self, channel: usize, samples: u32) {
        if let Some(slot) = self.base_latency.get_mut(channel)
            && *slot != samples
        {
            *slot = samples;
            self.refresh_pdc();
        }
    }

    /// 重算 PDC 对齐：每通道补 `最长路径 - 本通道路径` 的延迟，
    /// 使所有通道在求和点时间对齐。insert 链/乐器延迟变化后调用。
    ///
    /// 路径延迟 = 乐器延迟（base_latency）+ insert 链延迟之和。
    /// master 链的延迟对所有通道相同，不影响通道间对齐，不参与补偿。
    /// 仅命令处理阶段调用（延迟线缓冲重建会分配内存）。
    pub fn refresh_pdc(&mut self) {
        let mut total: Vec<u32> = self.base_latency.clone();
        for (i, chain) in self.inserts.iter().enumerate() {
            total[i] =
                total[i].saturating_add(chain.iter().map(|p| p.latency_samples()).sum::<u32>());
        }
        let align = total.iter().copied().max().unwrap_or(0);
        for (i, delay) in self.delays.iter_mut().enumerate() {
            delay.set_delay(align.saturating_sub(total[i]) as usize);
        }
    }

    /// 通道电平表读数端（UI 线程持有克隆，Arc 共享）。
    pub fn channel_meter_reading(&self, channel: usize) -> Option<MeterReading> {
        self.meter_readings.get(channel).cloned()
    }

    pub fn master_meter_reading(&self) -> MeterReading {
        self.master_reading.clone()
    }

    /// 取所有 insert（引擎拆除时整体回收，所有权交还上层）。
    pub fn take_all_inserts(&mut self) -> Vec<Box<dyn InsertProcessor>> {
        let mut out = Vec::new();
        for slot in &mut self.inserts {
            out.append(slot);
        }
        for slot in &mut self.bus_inserts {
            out.append(slot);
        }
        out.append(&mut self.master_inserts);
        out
    }

    /// 暂停/停止时把待发参数经各 insert 送达插件（输出丢弃；播放时无需调用）。
    pub fn flush_pending_insert_params(&mut self, position_samples: u64) {
        for chain in &mut self.inserts {
            for insert in chain {
                insert.flush_pending_params(position_samples);
            }
        }
        for chain in &mut self.bus_inserts {
            for insert in chain {
                insert.flush_pending_params(position_samples);
            }
        }
        for insert in &mut self.master_inserts {
            insert.flush_pending_params(position_samples);
        }
    }

    /// 通道电平表 tap（用于 UI 端读取）。
    pub fn channel_meter(&self, channel: usize) -> Option<MeterTap> {
        self.meters.get(channel).cloned()
    }

    pub fn master_meter(&self) -> MeterTap {
        self.master_meter.clone()
    }

    /// seek 后清空所有 insert 的处理状态（delay 尾音/envelope 等）
    /// 与 PDC 延迟线内容。
    pub fn reset_inserts(&mut self) {
        for chain in &mut self.inserts {
            for insert in chain {
                insert.reset();
            }
        }
        for chain in &mut self.bus_inserts {
            for insert in chain {
                insert.reset();
            }
        }
        for insert in &mut self.master_inserts {
            insert.reset();
        }
        for delay in &mut self.delays {
            delay.clear();
        }
        for b in &mut self.bus_buffers {
            b.left.fill(0.0);
            b.right.fill(0.0);
        }
    }

    /// 处理一块：返回主输出 (left, right)。
    ///
    /// 信号流：每通道 insert 链 → PDC 对齐 → 推子前 send → 推子（增益/声像
    /// 原地）→ 推子后 send → master；随后每总线 insert 链 → 总线推子 → master；
    /// 最后 master insert 链 → master 增益。
    ///
    /// solo 语义：任一对象（通道或总线）solo 时，只有 solo 对象发声——
    /// 通道 solo 静音其他通道；总线 solo 静音其它总线与通道直达 master 的路径
    /// （只保留送往该总线的部分）。mute 优先于 solo。
    ///
    /// PDC：按通道最长路径对齐（乐器/insert 延迟）。总线 insert 的延迟暂不参与
    /// 对齐（总线通常挂不要求严格相位对齐的效果，如混响）。
    pub fn process(&mut self) -> (&[f32], &[f32]) {
        let frames = self.frames;
        for bus in &mut self.bus_buffers {
            bus.left.fill(0.0);
            bus.right.fill(0.0);
        }
        self.master_l.fill(0.0);
        self.master_r.fill(0.0);

        let any_channel_solo = self.strips.iter().any(|s| s.params.solo);
        let any_bus_solo = self.bus_strips.iter().any(|s| s.params.solo);
        let any_solo = any_channel_solo || any_bus_solo;
        let any_solo_flag = any_solo;

        for i in 0..self.buffers.len() {
            // insert 链 → PDC 延迟线。
            {
                let (buffers, inserts) = (&mut self.buffers[i], &mut self.inserts[i]);
                for insert in inserts {
                    insert.process(&mut buffers.left, &mut buffers.right);
                }
            }
            {
                let (buffers, delay) = (&mut self.buffers[i], &mut self.delays[i]);
                delay.process(&mut buffers.left, &mut buffers.right);
            }

            let p = self.strips[i].params;
            let to_master = !p.mute && (!any_solo_flag || p.solo);

            // 电平表取 post-insert、pre-fader（推子会原地改缓冲，先发布）。
            if to_master {
                let (buffers, meter) = (&self.buffers[i], &mut self.meters[i]);
                meter.publish(&buffers.left[..frames], &buffers.right[..frames]);
            } else {
                self.meters[i].publish(&[0.0; 0], &[0.0; 0]);
            }

            // 推子前 send（insert 后、fader 前的信号）。
            for send in &self.sends[i] {
                if !send.pre_fader || send.amount == 0.0 {
                    continue;
                }
                let bus_solo = self
                    .bus_strips
                    .get(send.bus as usize)
                    .is_some_and(|s| s.params.solo);
                if p.mute || (any_solo_flag && !p.solo && !bus_solo) {
                    continue;
                }
                let Some(bus) = self.bus_buffers.get_mut(send.bus as usize) else {
                    continue;
                };
                let src = &self.buffers[i];
                for f in 0..frames {
                    bus.left[f] += src.left[f] * send.amount;
                    bus.right[f] += src.right[f] * send.amount;
                }
            }

            // 推子（原地应用；静音/未 solo 时也照常推进斜坡）。
            {
                let (buffers, strip) = (&mut self.buffers[i], &mut self.strips[i]);
                strip.apply_fader(&mut buffers.left, &mut buffers.right);
            }

            // 推子后 send 与 master 累加。
            for send in &self.sends[i] {
                if send.pre_fader || send.amount == 0.0 {
                    continue;
                }
                let bus_solo = self
                    .bus_strips
                    .get(send.bus as usize)
                    .is_some_and(|s| s.params.solo);
                if p.mute || (any_solo_flag && !p.solo && !bus_solo) {
                    continue;
                }
                let Some(bus) = self.bus_buffers.get_mut(send.bus as usize) else {
                    continue;
                };
                let src = &self.buffers[i];
                for f in 0..frames {
                    bus.left[f] += src.left[f] * send.amount;
                    bus.right[f] += src.right[f] * send.amount;
                }
            }
            if to_master {
                let src = &self.buffers[i];
                for f in 0..frames {
                    self.master_l[f] += src.left[f];
                    self.master_r[f] += src.right[f];
                }
            }
        }

        // 总线：insert 链 → 电平表（pre-fader）→ 推子 → master。
        for b in 0..self.bus_buffers.len() {
            {
                let (bus, inserts) = (&mut self.bus_buffers[b], &mut self.bus_inserts[b]);
                for insert in inserts {
                    insert.process(&mut bus.left, &mut bus.right);
                }
            }
            let p = self.bus_strips[b].params;
            let audible = !p.mute && (!any_solo_flag || p.solo);
            if audible {
                let bus = &self.bus_buffers[b];
                self.bus_meters[b].publish(&bus.left[..frames], &bus.right[..frames]);
            } else {
                self.bus_meters[b].publish(&[0.0; 0], &[0.0; 0]);
            }
            {
                let (bus, strip) = (&mut self.bus_buffers[b], &mut self.bus_strips[b]);
                strip.apply_fader(&mut bus.left, &mut bus.right);
            }
            if audible {
                let src = &self.bus_buffers[b];
                for f in 0..frames {
                    self.master_l[f] += src.left[f];
                    self.master_r[f] += src.right[f];
                }
            }
        }

        for insert in &mut self.master_inserts {
            insert.process(&mut self.master_l, &mut self.master_r);
        }

        // master 增益斜坡（复用通道同款逐样本线性插值）。
        let gain_start = self.master_prev_gain;
        let gain_step = (self.master_gain - gain_start) / frames as f32;
        for i in 0..frames {
            let g = gain_start + gain_step * (i + 1) as f32;
            self.master_l[i] *= g;
            self.master_r[i] *= g;
        }
        self.master_prev_gain = self.master_gain;

        self.master_meter.publish(&self.master_l, &self.master_r);
        (&self.master_l, &self.master_r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Doubler;
    impl InsertProcessor for Doubler {
        fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
            left.iter_mut().for_each(|v| *v *= 2.0);
            right.iter_mut().for_each(|v| *v *= 2.0);
        }

        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    /// 记录 flush 调用的 insert（验证暂停参数 flush 覆盖所有链）。
    struct FlushCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl InsertProcessor for FlushCounter {
        fn process(&mut self, _left: &mut [f32], _right: &mut [f32]) {}

        fn flush_pending_params(&mut self, _position_samples: u64) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    #[test]
    fn flush_pending_insert_params_reaches_all_chains() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let count = std::sync::Arc::new(AtomicUsize::new(0));
        let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
        g.insert_insert(0, 0, Box::new(FlushCounter(count.clone())));
        g.insert_insert(1, 0, Box::new(FlushCounter(count.clone())));
        g.insert_master_insert(0, Box::new(FlushCounter(count.clone())));
        g.flush_pending_insert_params(123);
        assert_eq!(count.load(Ordering::Relaxed), 3);
    }

    fn graph_with(channels: &[StripParams], frames: usize) -> MixerGraph {
        let mut g = MixerGraph::new(frames);
        g.resize(channels.len(), frames, channels);
        g
    }

    fn fill(g: &mut MixerGraph, channel: usize, value: f32) {
        let b = g.channel_buffers_mut(channel).unwrap();
        b.left.iter_mut().for_each(|v| *v = value);
        b.right.iter_mut().for_each(|v| *v = value);
    }

    #[test]
    fn single_channel_unity_gain_passthrough() {
        let mut g = graph_with(&[StripParams::default()], 4);
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        // 居中声像等功率：0.5 * 1.0 * √0.5，左右相同。
        assert!((l[3] - 0.5 * core::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }

    #[test]
    fn mute_silences_channel() {
        let mut g = graph_with(
            &[
                StripParams {
                    mute: true,
                    ..StripParams::default()
                },
                StripParams::default(),
            ],
            4,
        );
        fill(&mut g, 0, 1.0);
        fill(&mut g, 1, 0.5);
        let (l, _r) = g.process();
        let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn solo_excludes_other_channels() {
        let mut g = graph_with(
            &[
                StripParams {
                    solo: true,
                    ..StripParams::default()
                },
                StripParams::default(),
            ],
            4,
        );
        fill(&mut g, 0, 0.25);
        fill(&mut g, 1, 1.0);
        let (l, _r) = g.process();
        let expect = 0.25 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn mute_wins_over_solo() {
        let mut g = graph_with(
            &[StripParams {
                solo: true,
                mute: true,
                ..StripParams::default()
            }],
            4,
        );
        fill(&mut g, 0, 1.0);
        let (l, _r) = g.process();
        assert!(l.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn insert_runs_before_fader() {
        let mut g = graph_with(&[StripParams::default()], 4);
        fill(&mut g, 0, 0.25);
        g.set_inserts(0, vec![Box::new(Doubler)]);
        let (l, _r) = g.process();
        let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn master_gain_applies() {
        let mut g = graph_with(&[StripParams::default()], 4);
        fill(&mut g, 0, 1.0);
        g.set_master(MasterParams { gain: 0.5 });
        let (l, _r) = g.process();
        let expect = core::f32::consts::FRAC_1_SQRT_2 * 0.5;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn resize_keeps_existing_strip_state() {
        let mut g = graph_with(&[StripParams::default()], 4);
        g.set_strip(
            0,
            StripParams {
                gain: 0.3,
                ..StripParams::default()
            },
        );
        g.resize(2, 4, &[]);
        assert_eq!(g.channel_count(), 2);
        // 0 号通道增益状态保留。
        assert_eq!(g.strips[0].params.gain, 0.3);
    }

    #[test]
    fn resize_reallocates_buffers_on_frame_change() {
        let mut g = graph_with(&[StripParams::default()], 4);
        g.resize(1, 8, &[]);
        assert_eq!(g.frames(), 8);
        assert_eq!(g.buffers[0].left.len(), 8);
    }

    /// 报告延迟 N 且实际延迟 N 个样本的 insert（PDC 测试用）。
    struct DelayInsert {
        line: DelayLine,
        latency: u32,
    }

    impl DelayInsert {
        fn new(latency: u32) -> Self {
            let mut line = DelayLine::new();
            line.set_delay(latency as usize);
            Self { line, latency }
        }
    }

    impl InsertProcessor for DelayInsert {
        fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
            self.line.process(left, right);
        }

        fn latency_samples(&self) -> u32 {
            self.latency
        }

        fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
            self
        }
    }

    #[test]
    fn delay_line_delays_by_n_samples() {
        let mut d = DelayLine::new();
        d.set_delay(2);
        assert_eq!(d.delay(), 2);
        let mut l = [1.0f32, 2.0, 3.0, 4.0];
        let mut r = l;
        d.process(&mut l, &mut r);
        // 前两块是延迟线初始静音，随后是延迟 2 的输入。
        assert_eq!(l, [0.0, 0.0, 1.0, 2.0]);
        assert_eq!(r, [0.0, 0.0, 1.0, 2.0]);
        // 下一块继续吐出剩余的 3、4。
        let mut l2 = [5.0f32, 6.0];
        let mut r2 = l2;
        d.process(&mut l2, &mut r2);
        assert_eq!(l2, [3.0, 4.0]);
        // clear 后重新从静音开始。
        d.clear();
        let mut l3 = [7.0f32, 8.0];
        d.process(&mut l3, &mut [0.0, 0.0]);
        assert!(l3.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn delay_line_zero_delay_is_passthrough() {
        let mut d = DelayLine::new();
        let mut l = [1.0f32, 2.0];
        d.process(&mut l, &mut [0.0, 0.0]);
        assert_eq!(l, [1.0, 2.0]);
    }

    #[test]
    fn pdc_aligns_channels_with_latency_insert() {
        // 通道 1 有 2 样本延迟的 insert；通道 0 无。
        // 两通道同时输入脉冲：PDC 应给通道 0 补 2 样本，使求和点对齐。
        let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
        g.set_inserts(1, vec![Box::new(DelayInsert::new(2))]);
        fill(&mut g, 0, 0.0);
        fill(&mut g, 1, 0.0);
        {
            let b = g.channel_buffers_mut(0).unwrap();
            b.left[0] = 1.0;
            b.right[0] = 1.0;
        }
        {
            let b = g.channel_buffers_mut(1).unwrap();
            b.left[0] = 1.0;
            b.right[0] = 1.0;
        }
        // 第一块：两个脉冲都应出现在 sample 2（通道 1 由 insert 延迟，
        // 通道 0 由 PDC 延迟线延迟）。
        let (l, _r) = g.process();
        let single = core::f32::consts::FRAC_1_SQRT_2;
        assert!(l[0].abs() < 1e-6, "sample 0 应无输出（对齐后）");
        assert!(l[1].abs() < 1e-6, "sample 1 应无输出（对齐后）");
        assert!(
            (l[2] - 2.0 * single).abs() < 1e-6,
            "sample 2 应为两个通道脉冲之和: {}",
            l[2]
        );
        assert!(l[3].abs() < 1e-6);
    }

    #[test]
    fn pdc_keeps_single_channel_alignment_when_no_latency() {
        // 没有延迟插入时延迟线全为 0（直通），输出与无 PDC 时一致。
        let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
        assert!(g.delays.iter().all(|d| d.delay() == 0));
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        let expect = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[0] - expect).abs() < 1e-6);
    }

    #[test]
    fn pdc_recomputes_on_insert_removal() {
        // 移除延迟 insert 后对齐回退为 0（延迟线长度归零）。
        let mut g = graph_with(&[StripParams::default(), StripParams::default()], 4);
        g.set_inserts(1, vec![Box::new(DelayInsert::new(3))]);
        assert_eq!(g.delays[0].delay(), 3);
        g.set_inserts(1, Vec::new());
        assert!(g.delays.iter().all(|d| d.delay() == 0));
    }

    #[test]
    fn reset_clears_pdc_delay_lines() {
        let mut g = graph_with(&[StripParams::default(), StripParams::default()], 2);
        g.set_inserts(1, vec![Box::new(DelayInsert::new(2))]);
        {
            let b = g.channel_buffers_mut(0).unwrap();
            b.left[0] = 1.0;
        }
        let _ = g.process();
        g.reset_inserts();
        fill(&mut g, 0, 0.0);
        let (l, _r) = g.process();
        assert!(l.iter().all(|&v| v == 0.0), "reset 后延迟线应清空");
    }

    fn bus_graph(strip: StripParams, send: SendParams) -> MixerGraph {
        let mut g = graph_with(&[StripParams::default()], 4);
        g.resize_buses(1, 4, &[strip]);
        g.set_sends(0, vec![send]);
        g
    }

    #[test]
    fn post_fader_send_routes_to_bus() {
        // 通道直达 master + 推子后送 bus（bus 再进 master）= 2 份。
        let mut g = bus_graph(
            StripParams::default(),
            SendParams {
                bus: 0,
                amount: 1.0,
                pre_fader: false,
            },
        );
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        // 直达一份；bus 路径再去一次 bus 声像（居中等功率 = √0.5，
        // 与通道 strip 同语义）。
        let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        let expect = single + single * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn pre_fader_send_ignores_channel_gain() {
        // 通道 gain=0：直达静音，推子前 send 不受影响（只有 bus 路径）。
        let mut g = bus_graph(
            StripParams::default(),
            SendParams {
                bus: 0,
                amount: 1.0,
                pre_fader: true,
            },
        );
        g.set_strip(
            0,
            StripParams {
                gain: 0.0,
                ..StripParams::default()
            },
        );
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - single).abs() < 1e-6);
    }

    #[test]
    fn bus_mute_silences_bus_path_only() {
        let mut g = bus_graph(
            StripParams {
                mute: true,
                ..StripParams::default()
            },
            SendParams {
                bus: 0,
                amount: 1.0,
                pre_fader: false,
            },
        );
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - single).abs() < 1e-6);
    }

    #[test]
    fn bus_solo_mutes_direct_path() {
        // bus solo：只听 bus（通道直达 master 静音）。
        let mut g = bus_graph(
            StripParams {
                solo: true,
                ..StripParams::default()
            },
            SendParams {
                bus: 0,
                amount: 1.0,
                pre_fader: false,
            },
        );
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        // 只听 bus：bus 声像（居中等功率）。
        let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        let expect = single * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn bus_insert_processes_bus_signal() {
        // bus 上挂倍增 insert：直达 1 份 + bus 2 份 = 3 份。
        let mut g = bus_graph(
            StripParams::default(),
            SendParams {
                bus: 0,
                amount: 1.0,
                pre_fader: false,
            },
        );
        g.insert_bus_insert(0, 0, Box::new(Doubler));
        fill(&mut g, 0, 0.5);
        let (l, _r) = g.process();
        // 直达 1 份 + bus 2 份 × bus 声像。
        let single = 0.5 * core::f32::consts::FRAC_1_SQRT_2;
        let expect = single + 2.0 * single * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }

    #[test]
    fn resize_buses_keeps_existing_state() {
        let mut g = graph_with(&[StripParams::default()], 4);
        g.resize_buses(2, 4, &[StripParams::default(), StripParams::default()]);
        g.set_bus_strip(
            1,
            StripParams {
                gain: 0.25,
                ..StripParams::default()
            },
        );
        g.resize_buses(3, 4, &[]);
        assert_eq!(g.bus_count(), 3);
        assert_eq!(g.bus_strips[1].params.gain, 0.25);
    }

    #[test]
    fn set_inserts_returns_old_chain() {
        let mut g = graph_with(&[StripParams::default()], 4);
        g.set_inserts(0, vec![Box::new(Doubler)]);
        let old = g.set_inserts(0, Vec::new());
        assert_eq!(old.len(), 1);
        fill(&mut g, 0, 0.25);
        let (l, _r) = g.process();
        // 新链为空：无倍增。
        let expect = 0.25 * core::f32::consts::FRAC_1_SQRT_2;
        assert!((l[3] - expect).abs() < 1e-6);
    }
}
