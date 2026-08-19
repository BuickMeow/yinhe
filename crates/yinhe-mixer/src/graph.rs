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
use crate::params::{MasterParams, StripParams};
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

    /// 回收时还原为具体类型（如 CLAP 处理器需要 deactivate 回实例）。
    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any>;
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
            meters: Vec::new(),
            meter_readings: Vec::new(),
            master_l: vec![0.0; frames],
            master_r: vec![0.0; frames],
            master_gain: 1.0,
            master_prev_gain: 1.0,
            master_inserts: Vec::new(),
            master_meter,
            master_reading,
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
        self.meters.truncate(n_old);
        self.meter_readings.truncate(n_old);
        for i in n_old..channel_count {
            buffers.push(ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            });
            self.strips.push(StripState::new(strips.get(i).copied().unwrap_or_default()));
            self.inserts.push(Vec::new());
            let (tap, reading) = MeterTap::new();
            self.meters.push(tap);
            self.meter_readings.push(reading);
        }
        self.buffers = buffers;
    }

    /// 通道数。
    pub fn channel_count(&self) -> usize {
        self.buffers.len()
    }

    /// 当前块长（帧）。
    pub fn frames(&self) -> usize {
        self.frames
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
    /// 旧链（CLAP 处理器等）需要在上层线程回收（deactivate），不能直接 drop
    /// 在渲染线程——调用方负责把返回值送回去。
    pub fn set_inserts(
        &mut self,
        channel: usize,
        inserts: Vec<Box<dyn InsertProcessor>>,
    ) -> Vec<Box<dyn InsertProcessor>> {
        if let Some(slot) = self.inserts.get_mut(channel) {
            std::mem::replace(slot, inserts)
        } else {
            inserts
        }
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
        }
    }

    /// 移除并返回槽位 `slot` 的处理器（上层回收 deactivate）。
    pub fn remove_insert(&mut self, channel: usize, slot: usize) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.inserts.get_mut(channel)?;
        (slot < chain.len()).then(|| chain.remove(slot))
    }

    /// 替换槽位 `slot` 的处理器，返回旧的（插件请求 restart 时用）。
    pub fn replace_insert(
        &mut self,
        channel: usize,
        slot: usize,
        p: Box<dyn InsertProcessor>,
    ) -> Option<Box<dyn InsertProcessor>> {
        let chain = self.inserts.get_mut(channel)?;
        chain.get_mut(slot).map(|old| std::mem::replace(old, p))
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
        out.append(&mut self.master_inserts);
        out
    }

    /// 通道电平表 tap（用于 UI 端读取）。
    pub fn channel_meter(&self, channel: usize) -> Option<MeterTap> {
        self.meters.get(channel).cloned()
    }

    pub fn master_meter(&self) -> MeterTap {
        self.master_meter.clone()
    }

    /// seek 后清空所有 insert 的处理状态（delay 尾音/envelope 等）。
    pub fn reset_inserts(&mut self) {
        for chain in &mut self.inserts {
            for insert in chain {
                insert.reset();
            }
        }
        for insert in &mut self.master_inserts {
            insert.reset();
        }
    }

    /// 处理一块：返回主输出 (left, right)。
    ///
    /// solo 语义：任一通道 solo 时，只有 solo 通道发声；
    /// mute 与 solo 独立判定（mute 优先于 solo，与主流 DAW 一致）。
    pub fn process(&mut self) -> (&[f32], &[f32]) {
        let frames = self.frames;
        self.master_l.iter_mut().for_each(|v| *v = 0.0);
        self.master_r.iter_mut().for_each(|v| *v = 0.0);

        let any_solo = self.strips.iter().any(|s| s.params.solo);
        let master_l = &mut self.master_l;
        let master_r = &mut self.master_r;

        for i in 0..self.buffers.len() {
            let buffers = &mut self.buffers[i];
            for insert in &mut self.inserts[i] {
                insert.process(&mut buffers.left, &mut buffers.right);
            }

            let strip = &mut self.strips[i];
            let p = strip.params;
            let audible = !p.mute && (!any_solo || p.solo);
            strip.accumulate(
                &buffers.left,
                &buffers.right,
                master_l,
                master_r,
                audible,
            );
            // 电平表取 post-insert、pre-fader；静音/被独奏排除的通道读数为 0，
            // 符合「这路现在出没出声」的直觉。
            if audible {
                self.meters[i].publish(&buffers.left[..frames], &buffers.right[..frames]);
            } else {
                self.meters[i].publish(&[0.0; 0], &[0.0; 0]);
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
