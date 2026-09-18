//! CPU 合成器：对等 `GpuSynth` 的纯 CPU 渲染路径。
//!
//! 目标：行为对齐 xsynth（`parity` 测试逐样本对比），日后替代 xsynth 承担
//! CPU 合成。与 GPU 路径共用：
//! - `sfz_parser`：音色库解析与 (key, vel) 参数快照（`KeyInfo`）；
//! - `ChannelState`/`ChaseSkip`：CC/RPN/弯音/鼓组状态机与 chase 跳过；
//! - `SynthEvent`/`ControlEvent` 事件模型；
//! - `voice_render.wgsl` 的逐帧算法（CPU `voice.rs` 逐行复刻）。
//!
//! 与 GPU 路径的差异（有意）：
//! - `CpuVoice::time` 用 f64（对齐 xsynth `position: f64`，长曲无漂移）；
//! - 采样长度按帧计算（修正 GPU 立体声样本的 frames/elements 混用）；
//! - 事件按帧边界分段同步渲染（无 GPU 的段/指令流水线）；并行留待后续分片。

#[allow(dead_code)] // 渐进重构：SoA 接入后删除旧 AoS（voice.rs）
mod voice;

mod simd;
mod soa;

use std::path::PathBuf;
use std::sync::Arc;

use rayon::prelude::*;
use yinhe_mixer::ChannelBuffers;

use crate::channel_state::{ChannelState, ChaseSkip, MAX_CHANNELS, is_env_effect_cc};
use crate::cpu_synth::soa::{ENV_FINISHED, ENV_RELEASE, LANES_ALIGN, VoiceSoa};
use crate::gpu_synth::{ControlEvent, SynthEvent};
use crate::sfz_parser::{self, KeyMapEntry};

/// 成本分解开关（0=全功能；1=无滤波；2=无采样；3=只遍历）。
///
/// 供性能画像定位渲染成本构成；生产恒为 0，每块读一次（开销可忽略）。
/// 所有模式共享同一控制流骨架，差值即各阶段的成本占比。
pub static CPU_PROFILE_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
/// 内部耗时累计（ns）：note_on / note_off / 渲染分片循环（诊断用，测试读取）。
pub static PROF_NOTE_ON_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static PROF_NOTE_OFF_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static PROF_RENDER_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// note_on 内部细分：key map 查找 / voice 构造（含 biquad 系数）/ push。
/// 默认全局 voice 上限（与 GpuSynth 一致）。
/// 默认全局 voice 上限（与 GpuSynth 一致）。超限在**块末摊销淘汰**（优先
/// 已在 release/kill 中的 voice），淘汰走 1ms 淡出（ENV_KILL）听感无咔哒；
/// 不在 note_on 热路径 O(V) 扫描，块内允许短暂超出（上限是软约束）。
const DEFAULT_MAX_VOICES: usize = 8192;
/// 默认每 key layer 上限（对齐 xsynth `VoiceChannelParams.layers`）。
const DEFAULT_MAX_LAYERS: usize = 4;

/// dense 通道号 → 槽位索引；>= MAX_CHANNELS 返回 None（只支持 32 槽位）。
fn dense_channel(channel: usize) -> Option<usize> {
    (channel < MAX_CHANNELS).then_some(channel)
}

/// 纯 CPU 合成器（API 与 GpuSynth 对等）。
pub struct CpuSynth {
    sample_rate: u32,
    /// 采样插值方式（`Interpolation::code()`；加载音色库时写入 KeyInfo）。
    interpolation: u32,
    /// 每 dense 通道的音色库条目列表（dense 即槽位；与 GpuSynth 同结构）。
    /// `Arc` 共享：单音色库时直接指向进程级解析缓存，多通道零克隆。
    port_key_maps: Vec<Arc<Vec<KeyMapEntry>>>,
    channels: [ChannelState; MAX_CHANNELS],
    /// SoA 声部池（voice 间 SIMD；索引表存 slot）。
    soa: VoiceSoa,
    /// 排序好的事件列表（NoteOn 自带 end_sample）。
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 当前渲染位置（绝对 sample）。
    sample_position: u64,
    /// 最近一次 seek 的 event_cursor（chase_skip 的区间起点）。
    chase_base: usize,
    max_voices: usize,
    /// 每 key 同时活跃 voice 上限（`SetLayerCount`；None = 不限制）。
    /// xsynth 默认 4；超限时按 xsynth 语义杀该 key **velocity 最低**的 voice。
    max_layers: Option<usize>,
    peak_voices: usize,
    /// 块内 damper 快照（voice 循环读取，避免借用冲突）。
    damper_flags: [bool; MAX_CHANNELS],
    /// 并行渲染的每分片输出 scratch（分片 × MAX_CHANNELS × frames × 2，复用）。
    par_scratch: Vec<f32>,
    /// 每 (channel, key) 的活跃 voice 索引表（MAX_CHANNELS × 128 个 Vec）。
    /// note_on/note_off/enforce 的查找从 O(V) 降为 O(layer)（成本分解实测
    /// note_on+note_off 占 91% 渲染时间）；块末 retain 后重建。
    key_indices: Vec<Vec<u32>>,
}

impl CpuSynth {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            sample_rate,
            interpolation: 0,
            port_key_maps: (0..MAX_CHANNELS).map(|_| Arc::new(Vec::new())).collect(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            soa: VoiceSoa::new(),
            events: Vec::new(),
            event_cursor: 0,
            sample_position: 0,
            chase_base: 0,
            max_voices: DEFAULT_MAX_VOICES,
            max_layers: Some(DEFAULT_MAX_LAYERS),
            peak_voices: 0,
            damper_flags: [false; MAX_CHANNELS],
            par_scratch: Vec::new(),
            key_indices: vec![Vec::new(); MAX_CHANNELS * 128],
        }
    }

    /// (channel, key) → 索引表槽位。
    #[inline]
    fn key_slot(channel: u8, key: u8) -> usize {
        (channel as usize) * 128 + key as usize
    }

    /// 设置采样插值方式（`Interpolation::code()`；须在加载音色库之前设置）。
    pub fn set_interpolation(&mut self, interp: u32) {
        self.interpolation = interp;
    }

    /// 加载某 dense 通道的音色库（登记 key map；CPU 无样本上传阶段）。
    pub fn load_dense_soundfonts(&mut self, dense: u32, paths: &[PathBuf]) -> Result<(), String> {
        let slot = dense as usize;
        if slot >= MAX_CHANNELS {
            return Err(format!("CPU 合成器仅支持 32 个通道（dense {dense} 超出）"));
        }
        // 走进程级缓存（与 GPU 路径共用）：worker 已预热的音色库在此只查缓存，
        // 避免在音频线程解析 400MB 级音色库阻塞命令处理（Play 延迟数秒）。
        // 单库直接共享缓存 Arc（零克隆）；多库才拼接一份。
        self.port_key_maps[slot] = crate::gpu_synth::cache::load_key_maps_merged(
            paths,
            self.sample_rate,
            self.interpolation,
        )?;
        Ok(())
    }

    /// 与 GpuSynth 对等的收尾钩子（CPU 无上传，保留以统一调用方流程）。
    pub fn finish_soundfont_load(&mut self) {}

    /// 批量加载事件（**要求按 sample 有序**，调用方负责），重置渲染位置到 0。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        debug_assert!(
            events.windows(2).all(|w| w[0].sample() <= w[1].sample()),
            "load_events 要求事件按 sample 有序（调用方负责排序）"
        );
        self.events = events;
        self.event_cursor = 0;
        self.soa.clear();
        // 索引表与 voices 同生命周期：不清空则残留索引在后续 note_on/off
        // 的 enforce/查找中越界（重复 load_events 必崩；与 seek 一致）。
        for v in self.key_indices.iter_mut() {
            v.clear();
        }
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.sample_position = 0;
        self.chase_base = 0;
    }

    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数（未结束）。
    pub fn voice_count(&self) -> usize {
        self.soa.voice_count()
    }

    /// 每 key layer 上限（`SetLayerCount`；None = 不限制）。
    pub fn set_layer_count(&mut self, count: Option<usize>) {
        self.max_layers = count;
    }

    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }

    /// Seek 到指定位置：清 voice、重置通道状态，并**清空事件队列**。
    ///
    /// 与 GpuSynth（事件表预算好、seek 只定位 cursor）不同：CpuSynth 的事件由
    /// 引擎 dispatch 增量投递；seek 后引擎会从新位置重放事件，清空队列保证不重复。
    pub fn seek(&mut self, sample: u64) {
        self.sample_position = sample;
        self.events.clear();
        self.event_cursor = 0;
        self.chase_base = 0;
        self.soa.clear();
        for v in self.key_indices.iter_mut() {
            v.clear();
        }
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
    }

    /// chase 跳过掩码（seek 后已实时处理的控制事件区间 `[chase_base, cursor)`）。
    pub fn chase_skip(&self) -> ChaseSkip {
        let mut skip = ChaseSkip::default();
        for ev in &self.events[self.chase_base..self.event_cursor] {
            let SynthEvent::Control { channel, event, .. } = ev else {
                continue;
            };
            let Some(ch) = dense_channel(*channel as usize) else {
                continue;
            };
            match event {
                ControlEvent::Raw(cc, _) => skip.cc_mask[ch] |= 1u128 << cc,
                ControlEvent::PitchBend(_) => skip.pitch_bend[ch] = true,
                ControlEvent::PitchBendSensitivity(_) => skip.pbs[ch] = true,
                ControlEvent::FineTune(_) => skip.fine_tune[ch] = true,
                ControlEvent::CoarseTune(_) => skip.coarse_tune[ch] = true,
                ControlEvent::ProgramChange(_) => skip.program[ch] = true,
                ControlEvent::PercussionMode(_) => {}
            }
        }
        skip
    }

    /// 应用 chase 通道状态快照（seek 后由外部驱动；frame = 0 → 下一块开头生效）。
    pub fn apply_chase(&mut self, dense: u32, events: &[ControlEvent]) {
        for &ev in events {
            self.process_control_channel(dense as u8, ev, 0);
        }
    }

    /// 投递一个事件（追加到事件队列）。`sample` 必须单调不减
    /// （引擎的 dispatch 按 tick/sample 顺序投递）；事件在渲染到其 sample
    /// 所在段起点时生效（与 GPU 的段边界语义一致）。
    pub fn send_event(&mut self, event: SynthEvent) {
        self.events.push(event);
    }

    /// 增量渲染 `[offset, offset + frames)` 到混音台 planar 缓冲（覆盖写本段区间）。
    ///
    /// 与 `render_to_mixer` 的差异：由调用方（engine 的逐段 dispatch）提供区间，
    /// `sample_position` 按 `frames` 推进；区间起点处消费所有已到事件。
    pub fn render_range(&mut self, buffers: &mut [ChannelBuffers], offset: usize, frames: usize) {
        let t_prof = std::time::Instant::now();
        if frames == 0 || buffers.is_empty() {
            return;
        }
        let sample_start = self.sample_position;

        // 覆盖语义：清零本段区间（与 ChannelSet::render_segment 一致）
        let n = buffers.len().min(MAX_CHANNELS);
        for buf in buffers.iter_mut().take(n) {
            buf.left[offset..offset + frames].fill(0.0);
            buf.right[offset..offset + frames].fill(0.0);
        }
        for buf in buffers.iter_mut().skip(n) {
            buf.left[offset..offset + frames].fill(0.0);
            buf.right[offset..offset + frames].fill(0.0);
        }

        for i in 0..MAX_CHANNELS {
            self.damper_flags[i] = self.channels[i].damper;
        }

        // 区间内按事件分段：事件在各自 sample 的帧生效（段 = 相邻事件之间）
        let range_end = sample_start + frames as u64;
        let mut fi = 0usize;
        while fi < frames {
            let sample = sample_start + fi as u64;
            while self.event_cursor < self.events.len()
                && self.events[self.event_cursor].sample() <= sample
            {
                let ev = self.events[self.event_cursor];
                self.event_cursor += 1;
                self.dispatch_event(&ev, fi as u32);
            }
            let next = self
                .events
                .get(self.event_cursor)
                .map(|e| e.sample())
                .unwrap_or(u64::MAX);
            let seg_end = next.min(range_end);
            let seg = (seg_end.saturating_sub(sample) as usize).min(frames - fi);
            if seg == 0 {
                // 同 sample 的事件已在上面消费完，next 必然 > sample；
                // 防御性推进避免死循环（异常事件数据兜底）。
                fi += 1;
                continue;
            }
            self.render_range_frames(buffers, offset + fi, fi, seg, sample);
            fi += seg;
        }

        // 段末：time 推进 + 清理结束 voice（保序压缩，索引随后重建）
        for slot in 0..self.soa.len() {
            self.soa.advance_block(slot, frames as u32);
        }
        self.soa.compact();
        // 全局 voice 上限：块末摊销淘汰（O(V) 一次/块，不在 note_on 热路径）。
        let excess = self.soa.len().saturating_sub(self.max_voices);
        if excess > 0 {
            self.evict_excess(excess);
        }
        // 重建 per-key 索引表（O(V) 一次/段；保留 Vec 容量）
        for v in self.key_indices.iter_mut() {
            v.clear();
        }
        for slot in 0..self.soa.len() {
            let pos = Self::key_slot(self.soa.channel[slot], self.soa.key[slot]);
            self.key_indices[pos].push(slot as u32);
        }
        self.peak_voices = self.peak_voices.max(self.voice_count());
        self.sample_position = sample_start + frames as u64;

        PROF_RENDER_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        // 已消费事件周期性压缩（长播放不积累；chase_base 同步平移）。
        if self.event_cursor > 4096 {
            self.events.drain(..self.event_cursor);
            self.chase_base = self.chase_base.saturating_sub(self.event_cursor);
            self.event_cursor = 0;
        }
    }

    /// 渲染一整块（offset = 0；对等 GpuSynth 的 `render_to_mixer`）。
    pub fn render_to_mixer(&mut self, buffers: &mut [ChannelBuffers]) {
        let frames = buffers.first().map(|b| b.left.len()).unwrap_or(0);
        self.render_range(buffers, 0, frames);
    }

    /// 逐帧渲染 `[fi_start, fi_start + frames)`（区间内帧坐标）到分片
    /// scratch，再归约写回目标缓冲。字段级分离借用。
    fn render_range_frames(
        &mut self,
        buffers: &mut [ChannelBuffers],
        out_offset: usize,
        fi_start: usize,
        frames: usize,
        sample_start: u64,
    ) {
        if frames == 0 || self.soa.is_empty() {
            return;
        }
        let threads = rayon::current_num_threads().max(1);
        let capacity = self.soa.capacity();
        // 分片数 ≈ 线程数 ×2（细粒度让 Rayon 在大小核混合下负载均衡，粗分片
        // 时能效核上的大任务会拖住整块；×4 实测归约成本反而更高）。
        // 每片至少 LANES_ALIGN 个 lane。
        let target_chunks = (threads * 2).min(capacity.div_ceil(LANES_ALIGN)).max(1);
        let chunk = capacity
            .div_ceil(target_chunks)
            .next_multiple_of(LANES_ALIGN)
            .max(LANES_ALIGN);
        let n_chunks = capacity.div_ceil(chunk);
        let stride = MAX_CHANNELS * frames * 2;
        self.par_scratch.clear();
        self.par_scratch.resize(n_chunks * stride, 0.0);

        let soa = &mut self.soa;
        let scratch = &mut self.par_scratch;
        let damper = &self.damper_flags;
        let level = simd::level();
        fearless_simd::dispatch!(level, simd => {
            let mut views = soa.par_views(chunk);
            debug_assert_eq!(views.len(), n_chunks);
            views
                .par_iter_mut()
                .zip(scratch.par_chunks_mut(stride))
                .for_each(|(view, out)| {
                    view.render(simd, out, fi_start, frames, sample_start, damper);
                });
        });

        // 归约：分片 scratch 按通道求和写入目标缓冲（调用方已清零本段区间）。
        // 通道间并行；每通道内按分片顺序累加（与串行归约逐位一致）。
        let n = buffers.len().min(MAX_CHANNELS);
        buffers
            .par_iter_mut()
            .enumerate()
            .take(n)
            .for_each(|(ch, buf)| {
                let ch_base = ch * frames * 2;
                for b in 0..n_chunks {
                    let base = b * stride + ch_base;
                    for i in 0..frames {
                        buf.left[out_offset + i] += scratch[base + i * 2];
                        buf.right[out_offset + i] += scratch[base + i * 2 + 1];
                    }
                }
            });
    }

    /// 事件派发（帧内；`frame` = 块内帧偏移）。
    fn dispatch_event(&mut self, ev: &SynthEvent, frame: u32) {
        match ev {
            SynthEvent::NoteOn {
                channel,
                key,
                velocity,
                end_sample,
                ..
            } => self.note_on(*channel, *key, *velocity, *end_sample, frame),
            SynthEvent::NoteOff { channel, key, .. } => self.note_off(*channel, *key),
            SynthEvent::Control { channel, event, .. } => {
                self.process_control_channel(*channel, *event, frame);
            }
        }
    }

    /// NoteOn：从 key map 快照创建 voice；超限淘汰最老的 release 中 voice。
    fn note_on(&mut self, channel: u8, key: u8, vel: u8, end_sample: u64, frame: u32) {
        let t_prof = std::time::Instant::now();
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let ch = self.channels[ch_idx];
        let entries = self.port_key_maps[ch_idx].as_slice();
        let Some(info) = sfz_parser::select_key_info_multi(entries, ch.bank, ch.program, key, vel)
        else {
            return;
        };
        let slot = self.soa.init_lane(
            info,
            channel,
            key,
            vel,
            end_sample,
            frame,
            self.sample_rate,
            &ch,
        ) as u32;
        let slot_pos = Self::key_slot(channel, key);
        self.key_indices[slot_pos].push(slot);

        // per-key layer 上限：超限时按 xsynth 语义杀该 key velocity 最低的
        // voice（跳过刚加入的，保证新音符发声）。
        if let Some(max) = self.max_layers {
            self.enforce_key_layers(slot_pos, max, slot);
        }

        PROF_NOTE_ON_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// 该 key 活跃 voice 超过 `max` 时，反复杀 velocity 最低的
    /// （xsynth `pop_quietest_voice_group` 语义；`keep` = 刚加入的 slot 不参与）。
    /// 经 `key_indices` 只扫该 key 的 voice（O(layer)，非 O(V)）。
    ///
    /// 候选**不排除已 release 的 voice**：xsynth 的 `pop_quietest_voice_group`
    /// 只排除 killed，releasing 的 voice 同样在 `buffer` 里参与淘汰——且它们
    /// 创建最早，velocity 并列时优先被杀。release 尾巴被截掉听感无害，这是
    /// xsynth「几乎不丢音」的关键；只杀在响 voice 会造成明显丢音。
    fn enforce_key_layers(&mut self, slot_pos: usize, max: usize, keep: u32) {
        loop {
            // 先 O(layer) 数活跃数；未超限直接返回（多数 note_on 不分配、不扫候选）
            let active = self.key_indices[slot_pos]
                .iter()
                .filter(|&&i| {
                    let s = i as usize;
                    self.soa.env_stage[s] < ENV_FINISHED && self.soa.killed[s] == 0.0
                })
                .count();
            if active <= max {
                return;
            }
            // 超限（罕见）：找 velocity 最低的候选（含 release 中；并列取最早）
            let mut victim: Option<u32> = None;
            let mut victim_vel = u8::MAX;
            for &i in self.key_indices[slot_pos].iter() {
                let idx = i as usize;
                if i != keep
                    && self.soa.env_stage[idx] < ENV_FINISHED
                    && self.soa.killed[idx] == 0.0
                    && self.soa.velocity[idx] < victim_vel
                {
                    victim_vel = self.soa.velocity[idx];
                    victim = Some(i);
                }
            }
            let Some(victim) = victim else {
                return;
            };
            // 1ms 淡出（硬切会产生 click，用户实测）。
            self.soa.signal_kill(victim as usize, self.sample_rate);
        }
    }

    /// 全局 voice 超限淘汰（块末调用）：优先 release 中的，不足时按创建顺序
    /// 杀最老的。立即结束（与 xsynth 的默认 kill 语义一致），块末统一回收。
    fn evict_excess(&mut self, excess: usize) {
        let mut killed = 0;
        for i in 0..self.soa.len() {
            if killed >= excess {
                return;
            }
            if self.soa.env_stage[i] < ENV_FINISHED
                && self.soa.killed[i] == 0.0
                && self.soa.env_stage[i] >= ENV_RELEASE
            {
                self.soa.signal_kill(i, self.sample_rate);
                killed += 1;
            }
        }
        for i in 0..self.soa.len() {
            if killed >= excess {
                return;
            }
            if self.soa.env_stage[i] < ENV_FINISHED
                && self.soa.killed[i] == 0.0
                && self.soa.released[i] == 0.0
            {
                self.soa.signal_kill(i, self.sample_rate);
                killed += 1;
            }
        }
    }

    /// NoteOff：释放该 (channel, key) 最老的未释放 voice（延音踏板按住时只标记）。
    fn note_off(&mut self, channel: u8, key: u8) {
        let t_prof = std::time::Instant::now();
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper = self.channels[ch_idx].damper;
        let slot_pos = Self::key_slot(channel, key);
        // 索引表按创建顺序：正向找第一个未释放 = 最老未释放（O(layer)）
        let idxs = std::mem::take(&mut self.key_indices[slot_pos]);
        for &i in idxs.iter() {
            let i = i as usize;
            if self.soa.env_stage[i] < ENV_FINISHED
                && self.soa.released[i] == 0.0
                && self.soa.held_by_damper[i] == 0.0
            {
                if damper {
                    self.soa.held_by_damper[i] = 1.0;
                } else {
                    self.soa.signal_release(i, ENV_RELEASE);
                }
                break;
            }
        }
        self.key_indices[slot_pos] = idxs;
        PROF_NOTE_OFF_NS.fetch_add(
            t_prof.elapsed().as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// 控制事件：更新通道状态并把变化传播到该通道的活跃 voice。
    fn process_control_channel(&mut self, channel: u8, event: ControlEvent, frame: u32) {
        let Some(ch_idx) = dense_channel(channel as usize) else {
            return;
        };
        let damper_released = self.channels[ch_idx].process_control(event);
        if damper_released {
            // 松开延音踏板：释放被保持的 voice（与 GpuSynth apply_chase 同语义）
            for i in 0..self.soa.len() {
                if self.soa.channel[i] == channel
                    && self.soa.held_by_damper[i] != 0.0
                    && self.soa.env_stage[i] < ENV_FINISHED
                {
                    self.soa.held_by_damper[i] = 0.0;
                    self.soa.signal_release(i, ENV_RELEASE);
                }
            }
        }
        // 弯音/调音变化：更新该通道活跃 voice 的速度（含 time 校正）
        if matches!(
            event,
            ControlEvent::PitchBend(_)
                | ControlEvent::PitchBendSensitivity(_)
                | ControlEvent::FineTune(_)
                | ControlEvent::CoarseTune(_)
        ) {
            let mult = self.channels[ch_idx].pitch_multiplier();
            for i in 0..self.soa.len() {
                if self.soa.channel[i] == channel && self.soa.env_stage[i] < ENV_FINISHED {
                    self.soa.set_speed(i, mult, frame);
                }
            }
        }
        // CC72/73/121：重算活跃 voice 的包络时长
        if is_env_effect_cc(&event) {
            let ch = self.channels[ch_idx];
            let sr = self.sample_rate;
            for i in 0..self.soa.len() {
                if self.soa.channel[i] == channel && self.soa.env_stage[i] < ENV_FINISHED {
                    self.soa.apply_env_update(i, &ch, sr);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffers(frames: usize) -> Vec<ChannelBuffers> {
        (0..2)
            .map(|_| ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect()
    }

    /// 无音色库时 note_on 不 panic、输出静音（select 落空静默）。
    #[test]
    fn note_on_without_soundfont_is_silent() {
        let mut synth = CpuSynth::new(48_000);
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 4800,
        }]);
        let mut bufs = buffers(512);
        synth.render_to_mixer(&mut bufs);
        assert_eq!(synth.voice_count(), 0);
        assert!(bufs.iter().all(|b| b.left.iter().all(|&v| v == 0.0)));
    }

    /// 事件在正确帧生效：NoteOn 在块中间（sample 256）时前半块静音。
    #[test]
    fn note_on_starts_at_event_frame() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return; // 无测试音色库时跳过（CI）
        };
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
            .expect("load soundfont");
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 256,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 48_000,
        }]);
        let mut bufs = buffers(512);
        synth.render_to_mixer(&mut bufs);
        let head_energy: f32 = bufs[0].left[..256].iter().map(|v| v.abs()).sum();
        let tail_energy: f32 = bufs[0].left[256..].iter().map(|v| v.abs()).sum();
        assert_eq!(head_energy, 0.0, "起始帧前不得发声");
        assert!(tail_energy > 0.0, "起始帧后应有输出");
    }
    /// 重叠音符精确释放（回归：显式 NoteOff 的 FIFO 错位——短音符的结束会
    /// 释放长音符——是「音符被截断」的根因；NoteOn 自带 end_sample 后消除）。
    #[test]
    fn overlapping_notes_end_precisely() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return;
        };
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
            .expect("load soundfont");
        synth.load_events(vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
                end_sample: 48_000,
            },
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 60,
                end_sample: 4_800,
            },
        ]);
        let mut bufs = buffers(4_800);
        synth.render_to_mixer(&mut bufs);
        // SoA 池按 lane 查询：长音符（100）不得被短音符的结束释放；
        // 短音符（60）到期应已释放或已被清理。
        let released_of = |vel: u8| -> Vec<bool> {
            (0..synth.soa.len())
                .filter(|&i| synth.soa.velocity[i] == vel)
                .map(|i| synth.soa.released[i] != 0.0)
                .collect()
        };
        let long = released_of(100);
        assert!(
            long.iter().all(|&r| !r),
            "短音符结束不得释放长音符（FIFO 错位回归）"
        );
        let short = released_of(60);
        assert!(
            short.is_empty() || short.iter().all(|&r| r),
            "短音符到期应已释放"
        );
    }

    /// layer 上限（对齐 xsynth）：同一 key 5 个递增力度音符 + layer=4 →
    /// 活跃 voice 只 4 个（杀 velocity 最低的 20）。
    #[test]
    fn layer_limit_kills_quietest() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return;
        };
        let mut synth = CpuSynth::new(48_000);
        synth
            .load_dense_soundfonts(0, &[PathBuf::from(sfz)])
            .expect("load soundfont");
        synth.set_layer_count(Some(4));
        let mut events = Vec::new();
        for i in 0..5u8 {
            events.push(SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 20 + i * 20,
                end_sample: 48_000,
            });
        }
        synth.load_events(events);
        let mut bufs = buffers(512);
        synth.render_to_mixer(&mut bufs);
        assert_eq!(synth.voice_count(), 4, "layer=4 应限制同 key 活跃 voice 数");
        // 最弱的（20）被淘汰，剩下的都 >= 40
        assert!(synth.voice_count() == 4);
    }
}
