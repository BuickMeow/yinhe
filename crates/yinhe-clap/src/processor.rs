//! 渲染线程侧的插件处理器。
//!
//! 本类型是 `Send`（clack 的 PluginAudioProcessor 满足 Send），
//! 激活后在管理线程创建、move 进渲染线程使用。
//! 所有缓冲在创建时一次性分配，process 期间零分配、零锁。
//!
//! 端口模型：按插件声明的**全部** audio ports 供给缓冲（主端口进出接
//! 混音台音频，Aux 输入喂静音、Aux 输出写完丢弃）。只给主端口会让
//! 按声明端口数读 `audio_inputs[i]` 的插件包装层（如 JUCE 的
//! ClapJuceWrapper）越界读到空指针——Element FX（17 进 17 出）实测崩。

use std::sync::Arc;

use clack_host::events::io::EventBuffer;
use clack_host::prelude::{InputEvents, OutputEvents};
use clack_host::process::audio_buffers::{
    AudioPortBuffer, AudioPortBufferType, AudioPorts, InputChannel,
};
use clack_host::process::{PluginAudioProcessor, StoppedPluginAudioProcessor};
use yinhe_mixer::{InstrumentProcessor, PluginEvent};

use crate::error::PluginError;
use crate::events::push_event;
use crate::host::YinheHost;
use yinhe_mixer::ParamQueue;

/// 插件声明的端口布局（activate 时管理线程查询并冻结）。
///
/// 只存每端口声道数；端口顺序即 CLAP 端口索引（index 0 是主端口）。
pub(crate) struct PortLayout {
    pub in_channels: Vec<u32>,
    pub out_channels: Vec<u32>,
}

/// 单端口缓冲：channel → 帧数据。
type PortBuffers = Vec<Vec<f32>>;

fn alloc_ports(channels: &[u32], frames: usize) -> Vec<PortBuffers> {
    channels
        .iter()
        .map(|&ch| vec![vec![0.0; frames]; ch as usize])
        .collect()
}

/// 已激活的插件音频处理器（主端口立体声进、立体声出模型；Aux 端口静音/丢弃）。
pub struct ClapProcessor {
    processor: PluginAudioProcessor<YinheHost>,
    input_events: EventBuffer,
    output_events: EventBuffer,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    in_bufs: Vec<PortBuffers>,
    out_bufs: Vec<PortBuffers>,
    frames: usize,
    /// 插件上报的延迟（采样数；实例侧写入，这里只读；PDC 用）。
    latency: Arc<std::sync::atomic::AtomicU32>,
    /// UI 线程写入的参数变化；每块 drain 成 ParamValue 事件。
    param_queue: Arc<ParamQueue>,
    /// drain 暂存（保留容量，处理期间零分配）。
    param_scratch: Vec<(u32, f64)>,
}

impl ClapProcessor {
    pub(crate) fn new(
        stopped: StoppedPluginAudioProcessor<YinheHost>,
        frames: usize,
        layout: &PortLayout,
        param_queue: Arc<ParamQueue>,
        latency: Arc<std::sync::atomic::AtomicU32>,
    ) -> Self {
        // with_capacity 第一个参数是**声道总数**（所有端口声道数之和）。
        // 给小了会让 clack 内部 Vec 重分配，其重分配后的指针修复路径有 bug
        // （last_len..channel_count 切片范围错误），多端口插件（Element FX
        // 17×2 声道）会得到悬空 data32 指针 → 插件侧空指针解引用崩溃。
        let total_in_ch: usize = layout.in_channels.iter().map(|&c| c as usize).sum();
        let total_out_ch: usize = layout.out_channels.iter().map(|&c| c as usize).sum();
        Self {
            processor: PluginAudioProcessor::Stopped(stopped),
            input_events: EventBuffer::with_capacity(4096),
            output_events: EventBuffer::with_capacity(1024),
            input_ports: AudioPorts::with_capacity(total_in_ch, layout.in_channels.len()),
            output_ports: AudioPorts::with_capacity(total_out_ch, layout.out_channels.len()),
            in_bufs: alloc_ports(&layout.in_channels, frames),
            out_bufs: alloc_ports(&layout.out_channels, frames),
            frames,
            latency,
            param_queue,
            param_scratch: Vec::new(),
        }
    }

    /// 效果器用法（混音台 insert）：就地处理主端口输入音频。
    ///
    /// 输入拷贝进主端口内部缓冲（Aux 端口清零）后走 process，
    /// 输出由调用方拷回主端口（或直接用返回切片覆盖）。
    pub fn process_effect(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: &[PluginEvent],
        steady_time: Option<u64>,
    ) -> Result<(), PluginError> {
        let frames = self.frames.min(left.len()).min(right.len());
        for (port_idx, port) in self.in_bufs.iter_mut().enumerate() {
            for (ch_idx, ch) in port.iter_mut().enumerate() {
                if port_idx == 0 {
                    // 主端口：ch0 ← left，ch1 ← right，其余声道清零。
                    let src = match ch_idx {
                        0 => Some(&left[..frames]),
                        1 => Some(&right[..frames]),
                        _ => None,
                    };
                    match src {
                        Some(s) => {
                            ch[..frames].copy_from_slice(s);
                            ch[frames..].fill(0.0);
                        }
                        None => ch.fill(0.0),
                    }
                } else {
                    // Aux 输入端口：静音。
                    ch.fill(0.0);
                }
            }
        }
        let (out_l, out_r) = self.process_inner(events, steady_time)?;
        if out_l.len() >= frames && out_r.len() >= frames {
            left[..frames].copy_from_slice(&out_l[..frames]);
            right[..frames].copy_from_slice(&out_r[..frames]);
        } else {
            // 插件无输出端口：输出静音。
            left[..frames].fill(0.0);
            right[..frames].fill(0.0);
        }
        Ok(())
    }

    fn process_inner(
        &mut self,
        events: &[PluginEvent],
        steady_time: Option<u64>,
    ) -> Result<(&[f32], &[f32]), PluginError> {
        self.input_events.clear();
        self.output_events.clear();
        for event in events {
            push_event(&mut self.input_events, event);
        }
        // UI 线程写入的参数变化：作为块首（time 0）ParamValue 事件交给插件。
        self.param_queue.take_into(&mut self.param_scratch);
        for &(param_id, value) in &self.param_scratch {
            push_event(
                &mut self.input_events,
                &PluginEvent::ParamValue {
                    time: 0,
                    param_id,
                    value,
                },
            );
        }
        self.param_scratch.clear();
        self.input_events.sort();
        for port in &mut self.out_bufs {
            for ch in port {
                ch.fill(0.0);
            }
        }

        let input_events = InputEvents::from_buffer(&self.input_events);
        let mut output_events = OutputEvents::from_buffer(&mut self.output_events);

        let Self {
            processor,
            input_ports,
            output_ports,
            in_bufs,
            out_bufs,
            ..
        } = self;
        let input_audio = input_ports.with_input_buffers(in_bufs.iter_mut().map(|port| {
            AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_input_only(
                    port.iter_mut()
                        .map(|ch| InputChannel::variable(ch.as_mut_slice())),
                ),
            }
        }));
        let mut output_audio =
            output_ports.with_output_buffers(out_bufs.iter_mut().map(|port| AudioPortBuffer {
                latency: 0,
                channels: AudioPortBufferType::f32_output_only(
                    port.iter_mut().map(|ch| ch.as_mut_slice()),
                ),
            }));

        let processor = processor.ensure_processing_started()?;
        processor.process(
            &input_audio,
            &mut output_audio,
            &input_events,
            &mut output_events,
            steady_time,
            None,
        )?;
        // 输出事件（如插件的 NoteEnd、参数回显）第一期不消费，直接丢弃。

        // 主输出端口：无输出端口（纯 MIDI 插件）返回空切片（调用方按
        // 长度不足处理为静音）；单声道时右声道复用左声道数据。
        let Some(main) = out_bufs.first() else {
            return Ok((&[], &[]));
        };
        let out_l: &[f32] = main.first().map(Vec::as_slice).unwrap_or(&[]);
        let out_r: &[f32] = main.get(1).map(Vec::as_slice).unwrap_or(out_l);
        Ok((out_l, out_r))
    }

    /// 清空插件内部处理状态（envelope、delay 尾音等）。seek 后调用。
    pub fn reset(&mut self) {
        match &mut self.processor {
            PluginAudioProcessor::Started(p) => p.reset(),
            PluginAudioProcessor::Stopped(_) => {}
        }
    }

    /// 插件上报的延迟（采样数；实例侧在 activate/延迟变化时更新）。
    pub fn latency_samples(&self) -> u32 {
        self.latency.load(std::sync::atomic::Ordering::Acquire)
    }

    /// 是否有待发参数变化（暂停 flush 前的快速检查）。
    pub fn has_pending_params(&self) -> bool {
        !self.param_queue.is_empty()
    }

    /// 暂停/停止时把待发参数送达插件：跑一个静音块，参数经 ParamValue 事件应用，
    /// 音频输出丢弃。播放时无需调用（`process` 每块自然携带参数事件）。
    pub fn flush_pending_params(&mut self, position_samples: u64) {
        if !self.has_pending_params() {
            return;
        }
        // 输入端口清零（乐器本就无音频输入；效果器此处视为静音源）。
        for port in &mut self.in_bufs {
            for ch in port {
                ch.fill(0.0);
            }
        }
        if let Err(e) = self.process_inner(&[], Some(position_samples)) {
            tracing::warn!(target: "clap-plugin", "暂停参数 flush 失败: {e}");
        }
    }

    /// 停止处理并返回可传回主线程的句柄（供 deactivate）。
    pub fn into_stopped(mut self) -> StoppedPluginAudioProcessor<YinheHost> {
        self.processor.ensure_processing_stopped();
        match self.processor {
            PluginAudioProcessor::Stopped(stopped) => stopped,
            PluginAudioProcessor::Started(started) => started.stop_processing(),
        }
    }
}

impl InstrumentProcessor for ClapProcessor {
    fn process(
        &mut self,
        events: &[PluginEvent],
        out_l: &mut [f32],
        out_r: &mut [f32],
        position_samples: u64,
    ) {
        // 乐器输入无音频：清空全部输入端口（Aux 也清），只喂事件。
        for port in &mut self.in_bufs {
            for ch in port {
                ch.fill(0.0);
            }
        }
        match self.process_inner(events, Some(position_samples)) {
            Ok((l, r)) => {
                let frames = out_l.len().min(out_r.len());
                let n = l.len().min(r.len()).min(frames);
                out_l[..n].copy_from_slice(&l[..n]);
                out_r[..n].copy_from_slice(&r[..n]);
                out_l[n..frames].fill(0.0);
                out_r[n..frames].fill(0.0);
            }
            Err(e) => {
                tracing::warn!(target: "clap-instrument", "乐器处理失败，本块静音: {e}");
                out_l.fill(0.0);
                out_r.fill(0.0);
            }
        }
    }

    fn reset(&mut self) {
        ClapProcessor::reset(self);
    }

    fn flush_pending_params(&mut self, position_samples: u64) {
        ClapProcessor::flush_pending_params(self, position_samples);
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }
}
