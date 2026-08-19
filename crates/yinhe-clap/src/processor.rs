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

use clack_host::events::io::EventBuffer;
use clack_host::prelude::{InputEvents, OutputEvents};
use clack_host::process::audio_buffers::{
    AudioPortBuffer, AudioPortBufferType, AudioPorts, InputChannel,
};
use clack_host::process::{PluginAudioProcessor, StoppedPluginAudioProcessor};

use crate::error::PluginError;
use crate::events::{ClapInputEvent, push_event};
use crate::host::YinheHost;

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
}

impl ClapProcessor {
    pub(crate) fn new(
        stopped: StoppedPluginAudioProcessor<YinheHost>,
        frames: usize,
        layout: &PortLayout,
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
        }
    }

    /// 乐器用法：只喂事件，返回本块主端口立体声输出。
    pub fn process_instrument(
        &mut self,
        events: &[ClapInputEvent],
        steady_time: Option<u64>,
    ) -> Result<(&[f32], &[f32]), PluginError> {
        for port in &mut self.in_bufs {
            for ch in port {
                ch.fill(0.0);
            }
        }
        self.process_inner(events, steady_time)
    }

    /// 效果器用法（混音台 insert）：就地处理主端口输入音频。
    ///
    /// 输入拷贝进主端口内部缓冲（Aux 端口清零）后走 process，
    /// 输出由调用方拷回主端口（或直接用返回切片覆盖）。
    pub fn process_effect(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: &[ClapInputEvent],
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
        events: &[ClapInputEvent],
        steady_time: Option<u64>,
    ) -> Result<(&[f32], &[f32]), PluginError> {
        self.input_events.clear();
        self.output_events.clear();
        for event in events {
            push_event(&mut self.input_events, event);
        }
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

    /// 停止处理并返回可传回主线程的句柄（供 deactivate）。
    pub fn into_stopped(mut self) -> StoppedPluginAudioProcessor<YinheHost> {
        self.processor.ensure_processing_stopped();
        match self.processor {
            PluginAudioProcessor::Stopped(stopped) => stopped,
            PluginAudioProcessor::Started(started) => started.stop_processing(),
        }
    }
}
