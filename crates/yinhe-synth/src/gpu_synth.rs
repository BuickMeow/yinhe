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

mod channel;

use channel::ChannelState;
pub use channel::ChaseSkip;

mod schedule;

use schedule::{SegBuffers, Voice};

mod cache;

pub use cache::prefetch_key_maps;
use cache::{SampleBundle, cached_sample_bundle, load_key_maps, store_sample_bundle};

/// MIDI 通道数（dense 通道 = port×16+ch，支持 2 端口 32 通道）。
pub const MAX_CHANNELS: usize = 32;

/// 合成器事件（sample 域，按 sample 排序后由 `load_events` 加载）。
#[derive(Clone, Copy, Debug)]
pub enum SynthEvent {
    NoteOn {
        sample: u64,
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        sample: u64,
        channel: u8,
        key: u8,
    },
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
    port_key_maps: Vec<Vec<sfz_parser::KeyMapEntry>>,
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
    states_buf: Vec<GpuVoiceState>,
    /// 每通道上次的 pitch_multiplier（块起点比对；变化才产生段 0 的 ChState）
    channel_speed_cache: [f32; MAX_CHANNELS],
    /// 32 通道混音缓冲（GPU 输出读回，CPU 通道滤波 + 求和）
    channel_mix: Vec<f32>,
    /// 32 通道 MIDI 控制状态
    channels: [ChannelState; MAX_CHANNELS],
    /// 全局 voice 上限（黑乐谱长 sustain/无 note_off 的 voice 会累积，
    /// 超限时淘汰最老的 release 中 voice，否则最老的 active——与 xsynth voice 限制同思路）
    max_voices: usize,
    /// 峰值 voice 数统计（诊断用）
    peak_voices: usize,
    sample_rate: u32,
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
    /// 最近一次 seek 时的 event_cursor：`chase_skip` 用它计算"seek 后已处理
    /// 的控制事件区间"，chase 应用时跳过这些控制器（与 yinhe-audio 的
    /// `chase_cc_base` 对称，避免异步 chase 覆盖 seek 后已生效的新值）。
    chase_base: usize,
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
            port_key_maps: vec![Vec::new(); MAX_CHANNELS],
            channel_port: [0; MAX_CHANNELS],
            sample_paths: Vec::new(),
            sample_offsets: HashMap::new(),
            voices: Vec::new(),
            voice_stage_buf: Vec::new(),
            states_buf: Vec::new(),
            channel_speed_cache: [0.0; MAX_CHANNELS],
            channel_mix: Vec::new(),
            channels: [ChannelState::new(sample_rate); MAX_CHANNELS],
            max_voices: 8192,
            peak_voices: 0,
            sample_rate,
            events: Vec::new(),
            event_cursor: 0,
            cc_scratch: Vec::new(),
            seg_scratch: Vec::new(),
            sample_position: 0,
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
        let mut entries: Vec<sfz_parser::KeyMapEntry> = Vec::new();
        for path in paths {
            let built = load_key_maps(path, self.sample_rate)?;
            entries.extend(built.iter().cloned());
        }
        self.port_key_maps[slot] = entries;
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
            for entry in entries {
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
        self.events = events;
        self.event_cursor = 0;
        self.voices.clear();
        self.channels = [ChannelState::new(self.sample_rate); MAX_CHANNELS];
        self.channel_speed_cache = [0.0; MAX_CHANNELS];
        self.sample_position = 0;
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

    /// 渲染期间的峰值 voice 数（诊断用）
    pub fn peak_voices(&self) -> usize {
        self.peak_voices
    }

    /// Seek 到指定位置
    pub fn seek(&mut self, sample: u64) {
        self.sample_position = sample;
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

        let block_start = self.sample_position;
        let block_end = block_start + frames as u64;

        // 压缩预判（基于上一块读回的 env_stage）：墓碑占多数或接近槽位上限。
        // 压缩必须用 GPU 权威状态（time/envelope 由 GPU 推进），在本块读回后执行。
        let need_compact = self.voices.len() >= MAX_VOICE_SLOTS as usize
            || (!self.voices.is_empty()
                && self
                    .voices
                    .iter()
                    .filter(|v| v.state.env_stage >= 6)
                    .count()
                    * 2
                    >= self.voices.len());

        // 按渲染段 collect（段内帧索引相对段起点）：每段独立结构，
        // 供 renderer 分段跑 pass1/pass2（partial 只需 voices × 段长）。
        // 段缓冲跨块复用（take 出来，渲染完放回），只 clear 不重新分配。
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
        if !self.voices.is_empty() {
            self.voice_stage_buf.resize(self.voices.len(), 0);
            // 需要压缩时额外读回全字段（GPU 权威状态），否则零开销紧凑读回。
            if need_compact {
                self.states_buf
                    .resize(self.voices.len(), GpuVoiceState::default());
            }
            let readback = need_compact.then_some(self.states_buf.as_mut_slice());
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
            let n = self.renderer.render_block(
                self.voices.len() as u32,
                readback,
                &mut self.channel_mix,
                &mut self.voice_stage_buf,
                &segments,
                self.sample_rate,
            );
            debug_assert_eq!(n as usize, self.voices.len().min(MAX_VOICE_SLOTS as usize));
            drop(segments);
            // 读回紧凑 env_stage（voice 清理/墓碑标记用；其余状态常驻 GPU）。
            for (v, &stage) in self.voices.iter_mut().zip(self.voice_stage_buf.iter()) {
                v.state.env_stage = stage;
            }
            // 块末压缩：以刚读回的全字段状态为准 retain + 全量重传（索引重排）。
            if need_compact {
                for (v, st) in self.voices.iter_mut().zip(self.states_buf.iter()) {
                    v.state = *st;
                }
                self.voices.retain(|v| v.state.env_stage < 6);
                for (i, v) in self.voices.iter().enumerate() {
                    self.renderer.write_voice_state(i as u32, &v.state);
                }
            }
        } else {
            // voice 清空（seek/Stop 后到首音符之间）：必须清零复用缓冲，
            // 否则上一块的残留音频会被原样重写进混音台（空白区一直响旧余韵，
            // 直到下一个音符触发正常渲染覆盖它）。
            self.channel_mix.fill(0.0);
        }

        // 各通道去交错写入混音台 planar 缓冲（覆盖写；dense >= MAX_CHANNELS 清零）。
        let n = buffers.len().min(MAX_CHANNELS);
        for (ch_idx, buf) in buffers.iter_mut().enumerate().take(n) {
            let base = ch_idx * frames * 2;
            let ch_mix = &self.channel_mix[base..base + frames * 2];
            for i in 0..frames {
                buf.left[i] = ch_mix[i * 2];
                buf.right[i] = ch_mix[i * 2 + 1];
            }
        }
        for buf in buffers.iter_mut().skip(n) {
            buf.left.fill(0.0);
            buf.right.fill(0.0);
        }

        // 注意：死 voice（env_stage >= 6）保留在列表中作为墓碑，索引与 GPU 槽位
        // 严格一一对应；清理统一由块末压缩（need_compact）做 retain + 重传。
        self.peak_voices = self
            .peak_voices
            .max(self.voices.iter().filter(|v| v.state.env_stage < 6).count());

        // 段缓冲放回复用池（只保留容量，下块 clear 复用）
        self.seg_scratch = seg_data;
        self.sample_position = block_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        synth.load_events(vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
            },
            SynthEvent::NoteOff {
                sample: 44_100,
                channel: 0,
                key: 60,
            },
        ]);

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

    /// 回归：内部分段渲染（外层块 4096 = 8×512 段）与小块（512，单段）
    /// 输出一致，验证跨段 voice 状态（time/包络/滤波）与段间事件推进连续。
    #[test]
    fn segmented_render_matches_small_blocks() {
        let Some(sfz) = std::env::var_os("YINHE_TEST_SFZ") else {
            eprintln!("YINHE_TEST_SFZ not set, skipping");
            return;
        };
        let path = std::path::PathBuf::from(&sfz);
        let events = vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
            },
            SynthEvent::NoteOff {
                sample: 96_000,
                channel: 0,
                key: 60,
            },
            SynthEvent::NoteOn {
                sample: 20_000,
                channel: 0,
                key: 64,
                velocity: 90,
            },
            SynthEvent::NoteOff {
                sample: 30_000,
                channel: 0,
                key: 64,
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
            });
            events.push(SynthEvent::NoteOff {
                sample: (*start + 3000) as u64,
                channel: 0,
                key: 60 + i as u8,
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
        synth.load_events(vec![
            SynthEvent::NoteOn {
                sample: 0,
                channel: 0,
                key: 60,
                velocity: 100,
            },
            SynthEvent::NoteOff {
                sample: 44_100,
                channel: 0,
                key: 60,
            },
        ]);
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
}
