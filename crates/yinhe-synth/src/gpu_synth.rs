//! GPU 合成器高层封装 — 统一播放和导出接口。
//!
//! 和 xsynth 的 ChannelGroup 对等，但**只做音源层**：
//! - `note_on` / `note_off` / 控制事件接收（音量/声像/滤波等 DSP CC 由
//!   yinhe-dsp 效果器处理，本合成器忽略；见 `docs/spec-yinhe-dsp.md`）
//! - 32 通道 MIDI 状态机（pitch bend/RPN 调音、damper、ADSR CC、bank/program）
//! - `render` 一次性渲染整个 block（输出无限幅：限幅由调用方统一处理）
//! - `load_events` 批量加载预排序事件列表（用于导出/Seek）
//!
//! voice 管理、通道状态、ADSR 推进封装在内部。

use std::collections::HashMap;
use std::sync::Arc;

use crate::sfz_parser;
use crate::synth::GpuAudioRenderer;
use crate::synth::buffers::MAX_VOICE_SLOTS;
use crate::synth::{GpuVoiceState, RENDER_SEGMENT_FRAMES, RenderSegment};
use crate::wgpu;

use crate::channel_state::ChannelState;
pub use crate::channel_state::ChaseSkip;

mod schedule;

use schedule::{SegBuffers, Voice};

pub(crate) mod cache;

pub use cache::prefetch_key_maps;
use cache::{SampleBundle, cached_sample_bundle, load_key_maps_merged, store_sample_bundle};

/// MIDI 通道数（dense 通道 = port×16+ch，支持 2 端口 32 通道）。
pub use crate::channel_state::MAX_CHANNELS;

/// 合成器事件（sample 域，按 sample 排序后由 `load_events` 加载）。
#[derive(Clone, Copy, Debug)]
pub enum SynthEvent {
    /// NoteOn 携带音符结束时间 `end_sample`：voice 到期自行 release。
    /// 引擎的事件表**不再生成 NoteOff**（分页装载时 NoteOff 的时间戳跨窗口会
    /// 破坏事件顺序；自带 end 后事件量也减半）。
    NoteOn {
        sample: u64,
        channel: u8,
        key: u8,
        velocity: u8,
        end_sample: u64,
    },
    /// 立即释放该 (channel, key) 最老的未释放 voice。
    /// 引擎事件表不使用（NoteOn 自带 end 取代）；保留给实时 MIDI 输入/提前释放。
    NoteOff { sample: u64, channel: u8, key: u8 },
    Control {
        sample: u64,
        channel: u8,
        event: ControlEvent,
    },
}

impl SynthEvent {
    pub fn sample(&self) -> u64 {
        match self {
            SynthEvent::NoteOn { sample, .. }
            | SynthEvent::NoteOff { sample, .. }
            | SynthEvent::Control { sample, .. } => *sample,
        }
    }
}

/// 通道控制事件（语义与 xsynth `ControlEvent` 对齐）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlEvent {
    /// 原始 MIDI CC 事件 (controller, value)。
    Raw(u8, u8),
    /// 弯音值 -1..1。
    PitchBend(f32),
    /// 弯音灵敏度（半音）。
    PitchBendSensitivity(f32),
    /// 微调（音分）。
    FineTune(f32),
    /// 粗调（半音）。
    CoarseTune(f32),
    /// 音色更换：选择该通道的音色库条目（bank, preset 语义见 `PercussionMode`）。
    ProgramChange(u8),
    /// 鼓组模式（等价 xsynth `SetPercussionMode` 配置）：true 置 bank=128，false 置 bank=0。
    /// 由 yinhe-audio 在模型加载/seek 时根据通道声明注入，后续 CC0 不能改鼓组 bank。
    PercussionMode(bool),
}

/// GPU 合成器 — 封装 GPU 渲染器 + voice 管理 + 通道状态 + 事件调度 + 限幅。
///
/// 接口设计参照 xsynth ChannelGroup：
/// - 播放时通过 `load_events` 加载预排序事件，`render` 逐块渲染
/// - 通道状态机处理 CC7/10/11/64/100/101/6/38 + pitch bend + RPN
pub struct GpuSynth {
    renderer: GpuAudioRenderer,
    /// 每 port 的音色库条目列表（bank/preset → key map），`channel_port` 决定通道用哪个 port。
    /// `Arc` 共享：单音色库时直接指向进程级解析缓存，多通道零克隆。
    port_key_maps: Vec<Arc<Vec<sfz_parser::KeyMapEntry>>>,
    /// dense 通道 → port 映射（由 `load_port_soundfonts` 按 layout 填表）。
    channel_port: [u8; MAX_CHANNELS],
    /// 已加载过的音色库路径（拼接缓存的 key 组成，排序去重后使用）。
    sample_paths: Vec<std::path::PathBuf>,
    /// 采样数据在 GPU 上传块中的 (offset, len)，按 Arc 身份（指针 as usize）去重
    sample_offsets: HashMap<usize, (u32, u32)>,
    voices: Vec<Voice>,
    /// 紧凑 env_stage 读回缓冲（voice 清理用；全长缓冲复用）
    voice_stage_buf: Vec<u32>,
    /// 压缩时的全字段读回缓冲（复用，避免每块分配）
    /// 每通道上次的 pitch_multiplier（块起点比对；变化才产生段 0 的 ChState）
    channel_speed_cache: [f32; MAX_CHANNELS],
    /// 32 通道混音缓冲（GPU 输出读回，CPU 通道滤波 + 求和）
    channel_mix: Vec<f32>,
    /// 32 通道 MIDI 控制状态
    channels: [ChannelState; MAX_CHANNELS],
    /// 全局 voice 上限（黑乐谱长 sustain/无 note_off 的 voice 会累积，
    /// 超限时淘汰最老的 release 中 voice，否则最老的 active——与 xsynth voice 限制同思路）
    max_voices: usize,
    /// 每 key 同时活跃 voice 上限（`SetLayerCount`；None = 不限制）。
    /// xsynth 默认 4；超限时按 xsynth 语义杀该 key velocity 最低的 voice。
    max_layers: Option<usize>,
    /// 峰值 voice 数统计（诊断用）
    peak_voices: usize,
    sample_rate: u32,
    /// 采样插值方式（`Interpolation::code()`；加载音色库时写入 KeyInfo）。
    interpolation: u32,
    /// 排序好的事件列表（导出/Seek 用）
    events: Vec<SynthEvent>,
    event_cursor: usize,
    /// 本块内 CC 事件位置（升序，含重复 sample；collect_block 每块收集一次，
    /// 替代逐段从 event_cursor 重扫全部事件）
    cc_scratch: Vec<u64>,
    /// 分段渲染的段缓冲（复用，消除每块 4×段数 次 Vec 分配）
    seg_scratch: Vec<SegBuffers>,
    /// 当前渲染位置
    sample_position: u64,
    /// 提交游标（下一个待提交块的起点绝对 sample）。
    render_position: u64,
    /// 流水线：已提交未收割的块（FIFO）。
    pending: std::collections::VecDeque<PendingGpuBlock>,
    /// GPU 权威全字段 voice 状态（每次收割读回；compact 重传前用它覆盖
    /// CPU 镜像，否则重传过期位置/包络会把 voice 状态重置）。
    states_buf: Vec<GpuVoiceState>,
    /// 输出 ring：已渲染（GPU 读回）但未交给调用方的交错 PCM
    /// （帧 × MAX_CHANNELS × 2 f32）。预渲染的块先入 ring，调用方按所需
    /// 帧数取用——因此支持块大小变化（测试尾块/导出块）。
    ring: std::collections::VecDeque<f32>,
    /// 最近一次 seek 时的 event_cursor：`chase_skip` 用它计算"seek 后已处理
    /// 的控制事件区间"，chase 应用时跳过这些控制器（与 yinhe-audio 的
    /// `chase_cc_base` 对称，避免异步 chase 覆盖 seek 后已生效的新值）。
    chase_base: usize,
}

/// 流水线深度：已提交未收割的块数（提交与等待分离，见 `render_to_mixer`）。
/// 2 = 收割当前块时下一块已在 GPU 上执行，同步等待被重叠（实测 352 voice
/// 2.79ms→1.02ms，-63%）。
const PIPELINE_DEPTH: usize = 2;

/// 已提交未收割的块。
struct PendingGpuBlock {
    readback: crate::synth::renderer::PendingReadback,
    frames: usize,
    /// 提交时的 voice 数（槽位一一对应；收割时只更新前 N 个）。
    voice_count: usize,
}

impl GpuSynth {
    /// 创建合成器（自动创建 wgpu device/queue）。音色库稍后按 port 加载。
    pub fn new_default(sample_rate: u32) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new_default()
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, sample_rate)
    }

    /// 创建合成器（使用指定的 wgpu device/queue）。音色库稍后按 port 加载。
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        sample_rate: u32,
    ) -> Result<Self, String> {
        let renderer = GpuAudioRenderer::new(device, queue)
            .map_err(|e| format!("GPU renderer init failed: {}", e))?;
        Self::from_renderer(renderer, sample_rate)
    }

    fn from_renderer(renderer: GpuAudioRenderer, sample_rate: u32) -> Result<Self, String> {
        Ok(Self {
            renderer,
            // 每 dense 通道一个音色库条目列表（dense = port×16+ch，最多 MAX_CHANNELS）
            port_key_maps: (0..MAX_CHANNELS).map(|_| Arc::new(Vec::new())).collect(),
            channel_port: [0; MAX_CHANNELS],
            sample_paths: Vec::new(),
            sample_offsets: HashMap::new(),
            voices: Vec::new(),
            voice_stage_buf: Vec::new(),
            channel_speed_cache: [0.0; MAX_CHANNELS],
            channel_mix: Vec::new(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            max_voices: 8192,
            max_layers: Some(4),
            peak_voices: 0,
            sample_rate,
            interpolation: 0,
            events: Vec::new(),
            event_cursor: 0,
            cc_scratch: Vec::new(),
            seg_scratch: Vec::new(),
            sample_position: 0,
            render_position: 0,
            pending: std::collections::VecDeque::new(),
            states_buf: Vec::new(),
            ring: std::collections::VecDeque::new(),
            chase_base: 0,
        })
    }

    /// 加载一个 dense 通道的音色库列表（多文件 = 多个 (bank, preset) 条目，
    /// ProgramChange 在它们之间切换）。可多次调用（逐通道加载）。
    ///
    /// 只登记 key map，不上传样本——全部通道加载完成后由调用方调一次
    /// [`finish_soundfont_load`](Self::finish_soundfont_load) 统一上传
    /// （逐通道上传会退化成 O(n²) 全量重传）。
    /// `MAX_CHANNELS`（32）是 GPU 侧的 dense 槽位上限（2 个 MIDI 端口）；
    /// `dense >= MAX_CHANNELS` 返回错误（不支持折叠复用槽位）。
    pub fn load_dense_soundfonts(
        &mut self,
        dense: u32,
        paths: &[std::path::PathBuf],
    ) -> Result<(), String> {
        let slot = dense as usize;
        if slot >= MAX_CHANNELS {
            return Err(format!("GPU 合成器仅支持 32 个通道（dense {dense} 超出）"));
        }
        self.sample_paths.extend(paths.iter().cloned());
        // 单库直接共享缓存 Arc（零克隆）；多库才拼接一份。
        self.port_key_maps[slot] =
            load_key_maps_merged(paths, self.sample_rate, self.interpolation)?;
        self.channel_port[slot] = slot as u8;
        Ok(())
    }

    /// 全部通道音色加载完成后调用一次：把样本统一上传 GPU。
    pub fn finish_soundfont_load(&mut self) {
        self.rebuild_sample_upload();
    }

    /// 预热 GPU 缓冲与管线（加载阶段调用，`finish_soundfont_load` 之后）：
    /// 按最大 voice 容量与段长一次性分配并跑一次哑渲染，
    /// 播放中不再扩容重建、首块也不再触发 GPU 冷启动。
    pub fn prewarm(&mut self, frames: u32) {
        let sample_rate = self.sample_rate;
        self.renderer.prewarm(frames, sample_rate);
    }

    /// 把所有 port 的采样按 Arc 身份去重后拼成大块上传 GPU。
    /// 拼接结果按"音色库路径集合 + 采样率"缓存（只保留最近一份），
    /// 引擎重建导致的重复加载直接复用，跳过 500MB 级重拼。
    fn rebuild_sample_upload(&mut self) {
        let mut paths = self.sample_paths.clone();
        paths.sort();
        paths.dedup();
        let key = (paths, self.sample_rate);

        // 缓存命中：复用拼接数据（样本 Arc 与解析缓存共享，offsets 指针一致）
        if let Some((data, offsets)) = cached_sample_bundle(&key) {
            let mb = data.len() as f64 * 4.0 / (1024.0 * 1024.0);
            self.sample_offsets = offsets;
            self.renderer.upload_samples(data);
            eprintln!("[gpu] 采样拼接命中缓存（{mb:.0}MB，跳过重拼）");
            return;
        }

        // 未命中：按 Arc 身份去重 + 统计总长后一次性预分配拼接
        let t = std::time::Instant::now();
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut unique: Vec<&Arc<[f32]>> = Vec::new();
        for entries in &self.port_key_maps {
            for entry in entries.iter() {
                for key_layers in &entry.map {
                    for info in key_layers {
                        if seen.insert(info.sample_data.as_ptr() as usize) {
                            unique.push(&info.sample_data);
                        }
                    }
                }
            }
        }
        let mut data: Vec<f32> = Vec::with_capacity(unique.iter().map(|s| s.len()).sum());
        let mut offsets: HashMap<usize, (u32, u32)> = HashMap::with_capacity(unique.len());
        for sample in &unique {
            let offset = data.len() as u32;
            let len = sample.len() as u32;
            data.extend_from_slice(sample);
            offsets.insert(sample.as_ptr() as usize, (offset, len));
        }
        let mb = data.len() as f64 * 4.0 / (1024.0 * 1024.0);
        let chunk_count = data.len().div_ceil(crate::synth::types::CHUNK_SIZE);
        let data = Arc::new(data);
        self.sample_offsets = offsets.clone();
        store_sample_bundle(
            key,
            SampleBundle {
                data: Arc::clone(&data),
                offsets,
            },
        );
        self.renderer.upload_samples(data);
        eprintln!(
            "[gpu] 采样拼接={:?}（{chunk_count} 个 chunk，{mb:.0}MB），已缓存复用",
            t.elapsed()
        );
    }

    /// 批量加载排序好的事件列表（导出/Seek 用）。重置渲染位置到 0。
    pub fn load_events(&mut self, events: Vec<SynthEvent>) {
        debug_assert!(
            events.windows(2).all(|w| w[0].sample() <= w[1].sample()),
            "load_events 要求事件按 sample 有序（调用方负责排序）"
        );
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
        self.drain_pending(false);
        self.ring.clear();
        self.sample_position = 0;
        self.render_position = 0;
        self.chase_base = 0;
    }

    /// 当前渲染位置
    pub fn sample_position(&self) -> u64 {
        self.sample_position
    }

    /// 当前活跃 voice 数量（含 release 阶段，不含墓碑）。导出余韵循环用它早退。
    pub fn voice_count(&self) -> usize {
        self.voices.iter().filter(|v| v.state.env_stage < 6).count()
    }

    /// 设置全局 voice 上限（默认 8192）。超过时淘汰最老的 release 中 voice。
    pub fn set_max_voices(&mut self, max: usize) {
        self.max_voices = max;
    }

    /// 设置采样插值方式（`Interpolation::code()`；须在加载音色库之前设置）。
    pub fn set_interpolation(&mut self, interp: u32) {
        self.interpolation = interp;
    }

    /// 每 key layer 上限（`SetLayerCount`；None = 不限制；默认 4，对齐 xsynth）。
    pub fn set_layer_count(&mut self, count: Option<usize>) {
        self.max_layers = count;
    }

    /// 渲染期间的峰值 voice 数（诊断用）
    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }

    /// Seek 到指定位置
    pub fn seek(&mut self, sample: u64) {
        // 丢弃在途块与 ring（seek 后不再输出旧内容）
        self.drain_pending(false);
        self.ring.clear();
        self.sample_position = sample;
        self.render_position = sample;
        self.event_cursor = self.events.partition_point(|e| e.sample() < sample);
        // 记录 seek 点，供 chase_skip 计算"seek 后已处理的控制事件区间"。
        self.chase_base = self.event_cursor;
        self.voices.clear();
        // 通道状态在 seek 时重置（chase 由 yinhe-audio 的 cc_events 重建保证）
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        // speed 缓存清空：下一块起点重新下发全部通道的 pitch_multiplier。
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
    }

    /// 渲染一块到混音台的 planar 通道缓冲（覆盖写，与 CPU 路径
    /// `ChannelSet::render_segment` 同格式）：GPU 槽位 `ch` 写入 `buffers[ch]`，
    /// 超出 `MAX_CHANNELS` 的 dense 通道清零（GPU 合成器只支持前 32 个通道）。
    ///
    /// 块内事件（CC 段边界、note on/off、release/env 指令）在 CPU 收集为段结构，
    /// **一次 GPU 提交**渲染整块；voice 状态在 GPU 内逐帧推进（块末全字段读回）。
    pub fn render_to_mixer(&mut self, buffers: &mut [yinhe_mixer::ChannelBuffers]) {
        let frames = buffers.first().map(|b| b.left.len()).unwrap_or(0);
        if frames == 0 {
            return;
        }
        let per_frame = MAX_CHANNELS * 2;
        let need = frames * per_frame;
        // 预渲染：ring 不足时提交/收割（提交超前、收割入 ring；块大小可变化）
        while self.ring.len() < need {
            if self.compact_needed() {
                self.drain_pending(true);
                self.compact_voices();
            }
            let mut progressed = false;
            while self.pending.len() < PIPELINE_DEPTH && self.has_content() {
                if !self.submit_one_block(frames) {
                    break;
                }
                progressed = true;
            }
            if let Some(p) = self.pending.pop_front() {
                self.harvest(&p);
                self.push_block_to_ring(p.frames);
                progressed = true;
            }
            if !progressed {
                break;
            }
        }

        // 输出 frames 帧：ring 中的先给，不足部分静音补齐
        let avail = (self.ring.len() / per_frame).min(frames);
        for (ch_idx, buf) in buffers.iter_mut().enumerate() {
            if ch_idx < MAX_CHANNELS {
                for f in 0..avail {
                    let base = f * per_frame + ch_idx * 2;
                    buf.left[f] = self.ring[base];
                    buf.right[f] = self.ring[base + 1];
                }
                for f in avail..frames {
                    buf.left[f] = 0.0;
                    buf.right[f] = 0.0;
                }
            } else {
                buf.left.fill(0.0);
                buf.right.fill(0.0);
            }
        }
        self.ring.drain(..avail * per_frame);
        // 已输出位置（外部可见的播放进度）
        self.sample_position += frames as u64;
        // 无内容且 ring 已耗尽：提交游标与输出对齐（避免无限积压）
        if self.ring.is_empty() && !self.has_content() {
            self.render_position = self.sample_position;
        }
        self.peak_voices = self
            .peak_voices
            .max(self.voices.iter().filter(|v| v.state.env_stage < 6).count());
    }

    /// 是否还有可渲染内容（活跃 voice 或未消费事件）。
    fn has_content(&self) -> bool {
        !self.voices.is_empty() || self.event_cursor < self.events.len()
    }

    /// 压缩预判（基于最近收割的 env_stage）：墓碑占多数或接近槽位上限。
    fn compact_needed(&self) -> bool {
        self.voices.len() >= MAX_VOICE_SLOTS as usize
            || (!self.voices.is_empty()
                && self
                    .voices
                    .iter()
                    .filter(|v| v.state.env_stage >= 6)
                    .count()
                    * 2
                    >= self.voices.len())
    }

    /// 压缩：清理已结束 voice（tombstone）并全量重传槽位状态。
    fn compact_voices(&mut self) {
        self.voices.retain(|v| v.state.env_stage < 6);
        for (i, v) in self.voices.iter().enumerate() {
            self.renderer.write_voice_state(i as u32, &v.state);
        }
    }

    /// 提交一个块（不等待）：collect + 上传新 voice + renderer.submit_block。
    /// 返回 false 表示无可提交内容（无 voice / 无 GPU 缓冲）。
    fn submit_one_block(&mut self, frames: usize) -> bool {
        let block_start = self.render_position;
        let block_end = block_start + frames as u64;
        let upload_from = self.voices.len();
        let mut seg_data = std::mem::take(&mut self.seg_scratch);
        let mut seg_used = 0usize;
        let mut offset = 0usize;
        while offset < frames {
            let seg_frames = (frames - offset).min(RENDER_SEGMENT_FRAMES as usize);
            let s0 = block_start + offset as u64;
            let s1 = s0 + seg_frames as u64;
            if seg_used == seg_data.len() {
                seg_data.push(SegBuffers::default());
            }
            let sb = &mut seg_data[seg_used];
            sb.frame_start = offset as u32;
            sb.frame_length = seg_frames as u32;
            sb.segs.clear();
            sb.ch_updates.clear();
            sb.releases.clear();
            sb.env_cmds.clear();
            let new_from = self.voices.len();
            self.collect_block(
                s0,
                s1,
                &mut sb.segs,
                &mut sb.ch_updates,
                &mut sb.releases,
                &mut sb.env_cmds,
            );
            // 本段新建 voice 的 start_offset（段内帧）转**全局块内帧**：
            // shader 段末按段长右移未开始 voice 的偏移，跨段后回到段内相对值。
            for v in &mut self.voices[new_from..] {
                v.state.start_offset += offset as u32;
            }
            seg_used += 1;
            offset += seg_frames;
        }

        // 只上传本块新增的 voice 槽位（状态常驻 GPU，不再整块重传）。
        for (i, v) in self.voices.iter().enumerate().skip(upload_from) {
            self.renderer.write_voice_state(i as u32, &v.state);
        }

        self.channel_mix.resize(MAX_CHANNELS * frames * 2, 0.0);
        if self.voices.is_empty() {
            // 本块无 voice：输出静音，但提交游标必须前进（时间在流逝，事件可能
            // 在后续块；原实现在无 voice 时也无条件推进 sample_position）。
            self.channel_mix.fill(0.0);
            self.render_position = block_end;
            self.seg_scratch = seg_data;
            return false;
        }
        let submitted = {
            self.voice_stage_buf.resize(self.voices.len(), 0);
            let segments: Vec<RenderSegment<'_>> = seg_data[..seg_used]
                .iter()
                .map(|s| RenderSegment {
                    frame_start: s.frame_start,
                    frame_length: s.frame_length,
                    segs: &s.segs,
                    ch_updates: &s.ch_updates,
                    releases: &s.releases,
                    env_cmds: &s.env_cmds,
                })
                .collect();
            // 全字段读回：收割时用 GPU 权威状态覆盖 CPU 镜像。
            let rb = self.renderer.submit_block(
                self.voices.len() as u32,
                frames as u32,
                true,
                &segments,
                self.sample_rate,
            );
            drop(segments);
            rb
        };
        self.seg_scratch = seg_data;
        match submitted {
            Some(readback) => {
                self.pending.push_back(PendingGpuBlock {
                    readback,
                    frames,
                    voice_count: self.voices.len(),
                });
                self.render_position = block_end;
                true
            }
            None => false,
        }
    }

    /// 把一块已收割的 `channel_mix`（**通道优先** `[ch][frame][lr]`）重排为
    /// ring 的**帧优先**布局 `[frame][ch][lr]`（ring 跨块拼接只按帧消费）。
    fn push_block_to_ring(&mut self, frames: usize) {
        let ch = MAX_CHANNELS;
        for f in 0..frames {
            for c in 0..ch {
                let src = c * frames * 2 + f * 2;
                if src + 1 < self.channel_mix.len() {
                    self.ring.push_back(self.channel_mix[src]);
                    self.ring.push_back(self.channel_mix[src + 1]);
                } else {
                    self.ring.push_back(0.0);
                    self.ring.push_back(0.0);
                }
            }
        }
    }

    /// 收割读回（等待 + 拷贝），并用 GPU 权威 env_stage 更新 CPU 镜像。
    fn harvest(&mut self, p: &PendingGpuBlock) -> u32 {
        self.states_buf
            .resize(self.voices.len(), GpuVoiceState::default());
        let n = self.renderer.finish_block(
            &p.readback,
            &mut self.channel_mix,
            &mut self.voice_stage_buf,
            Some(self.states_buf.as_mut_slice()),
        );
        let cnt = p.voice_count.min(self.voices.len());
        // GPU 权威状态覆盖 CPU 镜像（仅该块提交时的前 N 个槽位；之后新 push 的
        // voice 保持 CPU 侧初值）。compact 重传前必须一致。
        for (v, st) in self.voices[..cnt].iter_mut().zip(self.states_buf.iter()) {
            v.state = *st;
        }
        n
    }

    /// 排空流水线：`update_stage` 时收割入 ring（不丢音频），否则直接丢弃
    /// （seek/换事件时旧内容不应再输出）。
    fn drain_pending(&mut self, update_stage: bool) {
        while let Some(p) = self.pending.pop_front() {
            if update_stage {
                self.harvest(&p);
                self.push_block_to_ring(p.frames);
            } else {
                self.renderer.discard_block(&p.readback);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GpuVoiceState;

    fn first_sample_ptr(s: &GpuSynth) -> *const f32 {
        s.port_key_maps[0]
            .iter()
            .flat_map(|e| e.map.iter())
            .flatten()
            .map(|info| info.sample_data.as_ptr())
            .next()
            .unwrap_or(std::ptr::null())
    }

    /// 进程级解析缓存：同路径第二次加载命中缓存，样本 Arc 跨实例共享。
    #[test]
    fn soundfont_parse_cache_shared_across_instances() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        // 预热缓存：并行测试下也保证后续两次都是命中（不依赖执行顺序）。
        let mut warm = GpuSynth::new_default(44_100).expect("GpuSynth warm");
        warm.load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("warm load");

        let mut a = GpuSynth::new_default(44_100).expect("GpuSynth a");
        a.load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load a");
        let mut b = GpuSynth::new_default(44_100).expect("GpuSynth b");
        b.load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load b");

        let ptr_a = first_sample_ptr(&a);
        let ptr_b = first_sample_ptr(&b);
        assert!(!ptr_a.is_null(), "样本指针不应为空");
        assert_eq!(ptr_a, ptr_b, "同路径两次加载应共享同一份样本内存（Arc）");
    }

    /// 回归：seek 清空 voices 后到下一个音符之间必须静音——复用缓冲的残留
    /// 音频会被原样写进混音台，导致空白区循环播放上一块（4096 帧）的余韵。
    #[test]
    fn seek_to_silence_without_voices() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 44_100,
        }]);

        let peak = |buffers: &[yinhe_mixer::ChannelBuffers]| {
            buffers
                .iter()
                .flat_map(|b| b.left.iter().chain(b.right.iter()))
                .fold(0.0f32, |m, v| m.max(v.abs()))
        };
        let frames = 512;
        let mut buffers: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();

        synth.render_to_mixer(&mut buffers);
        synth.render_to_mixer(&mut buffers);
        assert!(peak(&buffers) > 0.0, "音符期间应有输出");

        // seek 到音符之后：voices 清空、无新音符 → 必须静音
        synth.seek(4_000_000);
        synth.render_to_mixer(&mut buffers);
        assert_eq!(peak(&buffers), 0.0, "voices 清空后不得循环输出残留音频");
    }

    /// 回归：连续同 key 音符 + 短 end_sample（前一批还在 release 中就继续
    /// 触发）——layer 淘汰候选必须包含 release 中的 voice，否则无候选会无限
    /// 堆积，列表超过 MAX_VOICE_SLOTS 后新 voice 无 GPU 槽位（后面的音符永不
    /// 发声，用户实测现象）。同时验证全局淘汰跳过墓碑。
    #[test]
    fn consecutive_same_key_notes_do_not_pile_up() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.set_layer_count(Some(4));
        // 每 64 帧一个同 key 音符，128 帧后到期（release 尾巴 ~441 帧）→
        // 任意时刻同 key 在 release 中的 voice 远多于 layer 上限。
        let events: Vec<SynthEvent> = (0..600)
            .map(|i| SynthEvent::NoteOn {
                sample: i * 64,
                channel: 0,
                key: 60,
                velocity: 100,
                end_sample: i * 64 + 128,
            })
            .collect();
        synth.load_events(events);
        let frames = 512usize;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        for _ in 0..80 {
            synth.render_to_mixer(&mut bufs);
        }
        // 修复前：同 key voice 堆积数百（列表持续增长）
        assert!(
            synth.voice_count() <= 32,
            "连续同 key 音符不应堆积（实际 {} 个 voice）",
            synth.voice_count()
        );
    }

    /// layer 上限（对齐 xsynth）：同一 key 5 个递增力度音符 + layer=4 →
    /// 活跃 voice 只 4 个（杀 velocity 最低的）。
    #[test]
    fn layer_limit_kills_quietest() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.set_layer_count(Some(4));
        let mut events = Vec::new();
        for i in 0..5u8 {
            events.push(SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 20 + i * 20,
                end_sample: 44_100,
            });
        }
        synth.load_events(events);
        let frames = 512;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..1)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_to_mixer(&mut bufs);
        assert_eq!(synth.voice_count(), 4, "layer=4 应限制同 key 活跃 voice 数");
    }

    /// 回归：内部分段渲染（外层块 4096 = 8×512 段）与小块（512，单段）
    /// 输出一致，验证跨段 voice 状态（time/包络/滤波）与段间事件推进连续。
    #[test]
    fn segmented_render_matches_small_blocks() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut events = vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
                end_sample: 96_000,
            },
            SynthEvent::NoteOn {
                sample: 20_000,
                channel: 0,
                key: 64,
                velocity: 90,
                end_sample: 30_000,
            },
            // 踩/松延音踏板（跨段事件）
            SynthEvent::Control {
                sample: 10_000,
                channel: 0,
                event: ControlEvent::Raw(64, 127),
            },
            SynthEvent::Control {
                sample: 50_000,
                channel: 0,
                event: ControlEvent::Raw(64, 0),
            },
        ];
        events.sort_by_key(|e| e.sample());
        let render = |frames: usize| -> Vec<f32> {
            let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
            synth
                .load_dense_soundfonts(0, std::slice::from_ref(&path))
                .expect("load");
            synth.finish_soundfont_load();
            synth.load_events(events.clone());
            let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            let mut out = Vec::with_capacity(120_000 * 2);
            while out.len() < 120_000 * 2 {
                synth.render_to_mixer(&mut bufs);
                for i in 0..frames {
                    out.push(bufs[0].left[i]);
                    out.push(bufs[0].right[i]);
                }
            }
            out
        };
        let a = render(512);
        let b = render(4096);
        let n = a.len().min(b.len());
        let max_diff = a[..n]
            .iter()
            .zip(&b[..n])
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.0, "应有输出");
        assert!(
            max_diff < peak * 0.01,
            "分段（4096）与小块（512）输出不一致: max_diff={max_diff} peak={peak}"
        );
    }

    /// 回归：密集 pitch bend（段内反复换 speed）+ 音符在段内起始时，
    /// 分段（4096=8×512）与小块（512）输出一致（验证跨段时间推进连续）。
    #[test]
    fn segmented_render_matches_with_dense_bend() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut events: Vec<SynthEvent> = Vec::new();
        // 音符在段内多个位置创建（start_offset 非 0），跨多个渲染段
        for (i, start) in [100usize, 700, 1500, 2600, 3900].iter().enumerate() {
            events.push(SynthEvent::NoteOn {
                sample: *start as u64,
                channel: 0,
                key: 60 + i as u8,
                velocity: 100,
                end_sample: (*start + 3000) as u64,
            });
        }
        // 每 64 帧一次 pitch bend（段内反复换 speed，触发段边界 time 修正）
        for k in 0..180 {
            let v = ((k % 40) as f32 - 20.0) / 20.0 * 0.5;
            events.push(SynthEvent::Control {
                sample: (64 * k) as u64,
                channel: 0,
                event: ControlEvent::PitchBend(v),
            });
        }
        events.sort_by_key(|e| e.sample());

        let render = |frames: usize| -> Vec<f32> {
            let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
            synth
                .load_dense_soundfonts(0, std::slice::from_ref(&path))
                .expect("load");
            synth.finish_soundfont_load();
            synth.load_events(events.clone());
            let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            let mut out = Vec::with_capacity(120_000 * 2);
            while out.len() < 120_000 * 2 {
                synth.render_to_mixer(&mut bufs);
                for i in 0..frames {
                    out.push(bufs[0].left[i]);
                    out.push(bufs[0].right[i]);
                }
            }
            out
        };
        let a = render(512);
        let b = render(4096);
        let n = a.len().min(b.len());
        let max_diff = a[..n]
            .iter()
            .zip(&b[..n])
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.0, "应有输出");
        assert!(
            max_diff < peak * 0.01,
            "密集 bend 下分段与小块输出不一致: max_diff={max_diff} peak={peak}"
        );
    }

    /// 预热（含哑渲染）不破坏后续正常渲染：加载 → finish → prewarm → 音符正常出声。
    #[test]
    fn prewarm_then_render_ok() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        // 预热（分配 + 哑渲染）：不应 panic，也不污染后续 voice 槽位
        synth.prewarm(4096);
        synth.load_events(vec![SynthEvent::NoteOn {
            sample: 0,
            channel: 0,
            key: 60,
            velocity: 100,
            end_sample: 44_100,
        }]);
        let frames = 4096;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_to_mixer(&mut bufs);
        let peak = bufs
            .iter()
            .flat_map(|b| b.left.iter().chain(b.right.iter()))
            .fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.0, "预热后正常渲染应有输出（peak={peak}）");
    }

    /// 测试用 voice（sustain 阶段），只填被 chase 路径读取的字段。
    fn test_voice(stage: u32) -> Voice {
        Voice {
            state: GpuVoiceState {
                env_stage: stage,
                ..Default::default()
            },
            key: 60,
            channel: 0,
            velocity: 0,
            end_sample: u64::MAX,
            orig_attack_frames: 0.0,
            orig_release_frames: 0.0,
            held_by_damper: false,
            release_pending: false,
        }
    }

    /// chase 应用：damper 松开把 held voice 置入 release（块外直接写状态路径）。
    #[test]
    fn apply_chase_damper_release_marks_held_voices() {
        let Ok(mut synth) = GpuSynth::new_default(44_100) else {
            eprintln!("no GPU, skipping");
            return;
        };
        let mut v = test_voice(4);
        v.state.envelope = 0.7;
        v.held_by_damper = true;
        synth.voices.push(v);

        // 踩下再松开延音踏板：held voice 进入 release
        synth.apply_chase(0, &[ControlEvent::Raw(64, 127), ControlEvent::Raw(64, 0)]);
        let v = &synth.voices[0];
        assert_eq!(v.state.env_stage, 5, "held voice 应进入 release");
        assert_eq!(v.state.env_start, 0.7, "release 起点 = 当前 amp");
        assert_eq!(v.state.stage_progress, 0.0);
        assert!(v.release_pending);
        assert!(!v.held_by_damper);
    }

    /// chase 应用：CC73 修改 attack 时长后按 shader 规则重走当前阶段。
    #[test]
    fn apply_chase_env_cc_rewalks_stage() {
        let Ok(mut synth) = GpuSynth::new_default(44_100) else {
            eprintln!("no GPU, skipping");
            return;
        };
        let mut v = test_voice(3);
        v.state.envelope = 0.5;
        v.state.decay_start = 0.7;
        v.state.stage_progress = 10.0;
        v.state.attack_frames = 1000.0;
        v.state.release_frames = 2000.0;
        v.orig_attack_frames = 4410.0;
        synth.voices.push(v);

        synth.apply_chase(0, &[ControlEvent::Raw(0x49, 100)]);
        let v = &synth.voices[0];
        let expected = crate::channel_state::env_curve_frames(100, 4410.0, 44_100, false);
        assert!(
            (v.state.attack_frames - expected).abs() < 1e-3,
            "CC73 应重算 attack 时长: {} vs {expected}",
            v.state.attack_frames
        );
        assert_eq!(v.state.release_frames, 2000.0, "未修改的 release 保持原值");
        assert_eq!(v.state.decay_start, 0.5, "Decay 重走起点 = 当前 amp");
        assert_eq!(v.state.stage_progress, 0.0);
    }

    /// 临时诊断：dump 高音区采样的包络参数。
    #[test]
    #[ignore = "需要 YINHE_TEST_SFZ"]
    fn tmp_dump_envelope_params() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let mut synth = GpuSynth::new_default(48_000).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&path))
            .expect("load");
        synth.finish_soundfont_load();
        for key in [60u8, 107, 108, 120, 127] {
            synth.voices.clear();
            synth.load_events(vec![SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key,
                velocity: 127,
                end_sample: 44_100,
            }]);
            let frames = 512;
            let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..1)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            synth.render_to_mixer(&mut bufs);
            if let Some(v) = synth.voices.first() {
                eprintln!(
                    "key={key}: attack={:.1} hold={:.1} decay={:.0} release={:.1} sustain={:.4} env_level={} speed={:.4} sample_len={}",
                    v.state.attack_frames,
                    v.state.hold_frames,
                    v.state.decay_frames,
                    v.state.release_frames,
                    v.state.sustain_level,
                    v.state.env_level,
                    v.state.speed,
                    v.state.sample_length,
                );
            } else {
                eprintln!("key={key}: 无 voice（选不到音色？）");
            }
        }
    }

    /// 临时诊断：5 批高音簇的 voice 状态 dump。
    #[test]
    #[ignore = "需要本地环境"]
    fn tmp_multi_batch_voice_dump() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
        for i in 0..480_000u32 {
            let v = ((i as f32) * 0.05).sin() * 20_000.0;
            w.write_sample(v as i16).expect("write");
        }
        w.finalize().expect("finalize");
        std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nlokey=0 hikey=127\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

        let sr = 48_000u32;
        let mut events = Vec::new();
        for b in 0..5u64 {
            let t0 = b * 5_294;
            for key in 60u8..65 {
                events.push(SynthEvent::NoteOn {
                    sample: t0,
                    channel: 0,
                    key,
                    velocity: 127,
                    end_sample: t0 + 4_963,
                });
            }
        }
        let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(events);
        synth.seek(0);
        let frames = 4096; // 与生产块一致（多段渲染路径）
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }];
        for i in 0..30 {
            synth.render_to_mixer(&mut bufs);
            let pos = (i + 1) * frames;
            if [1, 2, 3, 4, 5, 6, 7, 8, 10, 15, 20, 25].contains(&i) {
                let vc: Vec<String> = synth
                    .voices
                    .iter()
                    .map(|v| format!("k{}:s{}", v.key, v.state.env_stage))
                    .collect();
                eprintln!("块{}（帧{}）: n={} {}", i + 1, pos, vc.len(), vc.join(" "));
            }
        }
    }

    /// 最小复现：批 0 on@0/off@4963 + 批 1 on@4096/off@9059（块 4096）。
    #[test]
    #[ignore = "诊断"]
    fn tmp_min_repro_batches() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
        for i in 0..480_000u32 {
            let v = ((i as f32) * 0.05).sin() * 20_000.0;
            w.write_sample(v as i16).expect("write");
        }
        w.finalize().expect("finalize");
        std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

        let sr = 48_000u32;
        let events = vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 127,
                end_sample: 4_963,
            },
            SynthEvent::NoteOn {
                sample: 4_096,
                channel: 0,
                key: 61,
                velocity: 127,
                end_sample: 9_059,
            },
        ];
        // 验证 sfz 的 keyrange
        {
            let maps = crate::sfz_parser::build_key_maps(&sfz_path, sr, 0).expect("build maps");
            for key in [60u8, 61] {
                let ok = crate::sfz_parser::select_key_info(&maps[0].map, key, 127).is_some();
                eprintln!("  key={key} region存在={ok}");
            }
        }
        let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(events);
        synth.seek(0);
        let frames = 4096;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }];
        for i in 0..4 {
            synth.render_to_mixer(&mut bufs);
            let rms =
                (bufs[0].left.iter().map(|x| (x * x) as f64).sum::<f64>() / frames as f64).sqrt();
            let vc: Vec<String> = synth
                .voices
                .iter()
                .enumerate()
                .map(|(idx, v)| {
                    format!(
                        "[{idx}]k{}:s{}/so{}/rp{}",
                        v.key, v.state.env_stage, v.state.start_offset, v.release_pending as u8
                    )
                })
                .collect();
            eprintln!("块{}: rms={rms:.5} voices={vc:?}", i + 1);
        }
    }

    /// 临时诊断：同 key 3 批音符的 voice 释放顺序（note_off 是否释放"最老"）。
    #[test]
    #[ignore = "诊断"]
    fn tmp_three_batches_release_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav");
        for i in 0..480_000u32 {
            let v = ((i as f32) * 0.05).sin() * 20_000.0;
            w.write_sample(v as i16).expect("write");
        }
        w.finalize().expect("finalize");
        std::fs::write(
            &sfz_path,
            "<region>\nsample=tone.wav\nampeg_hold=0.6\nampeg_decay=89.88\nampeg_sustain=1.778\nampeg_release=3.5\n",
        )
        .expect("sfz");

        let sr = 48_000u32;
        // 3 批同 key：on 0 / off 4963 / on 5294 / off 10257 / on 10588 / off 15551
        let events = vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
                end_sample: 4_963,
            },
            SynthEvent::NoteOn {
                sample: 5_294,
                channel: 0,
                key: 60,
                velocity: 110,
                end_sample: 10_257,
            },
            SynthEvent::NoteOn {
                sample: 10_588,
                channel: 0,
                key: 60,
                velocity: 120,
                end_sample: 15_551,
            },
        ];
        let mut synth = GpuSynth::new_default(sr).expect("GpuSynth");
        synth
            .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
            .expect("load");
        synth.finish_soundfont_load();
        synth.load_events(events);
        synth.seek(0);
        let frames = 4096;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = vec![yinhe_mixer::ChannelBuffers {
            left: vec![0.0; frames],
            right: vec![0.0; frames],
        }];
        for i in 0..6 {
            synth.render_to_mixer(&mut bufs);
            let vc: Vec<String> = synth
                .voices
                .iter()
                .enumerate()
                .map(|(idx, v)| format!("[{idx}]vel?/stage{}", v.state.env_stage))
                .collect();
            eprintln!(
                "块{}（帧{}）: n={} {}",
                i + 1,
                (i + 1) * frames,
                vc.len(),
                vc.join(" ")
            );
        }
    }

    /// chase_skip：只标记 seek 之后被实时处理过的控制事件（区间 [chase_base, cursor)）。
    #[test]
    fn chase_skip_marks_only_post_seek_controls() {
        let Ok(mut synth) = GpuSynth::new_default(44_100) else {
            eprintln!("no GPU, skipping");
            return;
        };
        synth.load_events(vec![
            SynthEvent::Control {
                sample: 100,
                channel: 0,
                event: ControlEvent::Raw(7, 100),
            },
            SynthEvent::Control {
                sample: 200,
                channel: 0,
                event: ControlEvent::PitchBend(0.5),
            },
            SynthEvent::NoteOn {
                sample: 300,
                channel: 0,
                key: 60,
                velocity: 100,
                end_sample: 100_000,
            },
            SynthEvent::Control {
                sample: 400,
                channel: 0,
                event: ControlEvent::Raw(64, 127),
            },
        ]);
        synth.seek(300);
        // 渲染一块推进 cursor 过 300（音符）与 400（CC64）
        let frames = 512;
        let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
            .map(|_| yinhe_mixer::ChannelBuffers {
                left: vec![0.0; frames],
                right: vec![0.0; frames],
            })
            .collect();
        synth.render_to_mixer(&mut bufs);

        let skip = synth.chase_skip();
        assert!(
            skip.cc_mask[0] & (1u128 << 64) != 0,
            "seek 后被实时处理的 CC64 应标记"
        );
        assert!(
            skip.cc_mask[0] & (1u128 << 7) == 0,
            "seek 前的 CC7 不应标记"
        );
        assert!(!skip.pitch_bend[0], "seek 前的 PitchBend 不应标记");
    }

    /// 诊断/回归：块内任意帧开始的音符必须在正确帧出声。
    /// 生产块长 4096（8×512 渲染段），而多数测试用 512 块（单段）——
    /// 段偏移（start_offset 的段内相对 vs 块内帧语义）只在多段块暴露。
    #[test]
    fn note_starts_on_time_in_later_segments() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wav_path = dir.path().join("tone.wav");
        let sfz_path = dir.path().join("tone.sfz");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&wav_path, spec).expect("wav create");
        for _ in 0..44_100 {
            w.write_sample(16_000i16).expect("wav write");
        }
        w.finalize().expect("wav finalize");
        std::fs::write(&sfz_path, "<region>\nsample=tone.wav key=60\n").expect("sfz write");

        let onset_for = |note_sample: u64| -> Option<usize> {
            let mut synth = GpuSynth::new_default(44_100).expect("GpuSynth");
            synth
                .load_dense_soundfonts(0, std::slice::from_ref(&sfz_path))
                .expect("load");
            synth.finish_soundfont_load();
            synth.load_events(vec![SynthEvent::NoteOn {
                sample: note_sample,
                channel: 0,
                key: 60,
                velocity: 127,
                end_sample: note_sample + 44_100,
            }]);
            let frames = 2048usize;
            let mut bufs: Vec<yinhe_mixer::ChannelBuffers> = (0..2)
                .map(|_| yinhe_mixer::ChannelBuffers {
                    left: vec![0.0; frames],
                    right: vec![0.0; frames],
                })
                .collect();
            synth.render_to_mixer(&mut bufs);
            bufs[0].left.iter().position(|&x| x.abs() > 0.001)
        };

        let mut onsets = Vec::new();
        for note_sample in [100u64, 600, 1500, 2000] {
            let onset = onset_for(note_sample).expect("应有输出");
            onsets.push((note_sample, onset));
        }
        for (note_sample, onset) in onsets {
            let diff = onset as i64 - note_sample as i64;
            assert!(
                diff.abs() < 64,
                "块内帧 {note_sample} 的音符 onset 错位（实际 {onset}，差 {diff}）"
            );
        }
    }
}
