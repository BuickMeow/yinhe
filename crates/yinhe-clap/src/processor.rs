//! 渲染线程侧的插件处理器。
//!
//! 本类型是 `Send`（clack 的 PluginAudioProcessor 满足 Send），
//! 激活后在管理线程创建、move 进渲染线程使用。
//! 所有缓冲在创建时一次性分配，process 期间零分配、零锁。

use clack_host::events::io::EventBuffer;
use clack_host::prelude::{InputEvents, OutputEvents};
use clack_host::process::audio_buffers::{
    AudioPortBuffer, AudioPortBufferType, AudioPorts, InputChannel,
};
use clack_host::process::{PluginAudioProcessor, StoppedPluginAudioProcessor};

use crate::error::PluginError;
use crate::events::{ClapInputEvent, push_event};
use crate::host::YinheHost;

/// 已激活的插件音频处理器（立体声进、立体声出模型；乐器输入为静音）。
pub struct ClapProcessor {
    processor: PluginAudioProcessor<YinheHost>,
    input_events: EventBuffer,
    output_events: EventBuffer,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    out_l: Vec<f32>,
    out_r: Vec<f32>,
    frames: usize,
}

impl ClapProcessor {
    pub(crate) fn new(stopped: StoppedPluginAudioProcessor<YinheHost>, frames: usize) -> Self {
        Self {
            processor: PluginAudioProcessor::Stopped(stopped),
            input_events: EventBuffer::with_capacity(4096),
            output_events: EventBuffer::with_capacity(1024),
            input_ports: AudioPorts::with_capacity(2, 1),
            output_ports: AudioPorts::with_capacity(2, 1),
            in_l: vec![0.0; frames],
            in_r: vec![0.0; frames],
            out_l: vec![0.0; frames],
            out_r: vec![0.0; frames],
            frames,
        }
    }

    /// 乐器用法：只喂事件，返回本块立体声输出。
    pub fn process_instrument(
        &mut self,
        events: &[ClapInputEvent],
        steady_time: Option<u64>,
    ) -> Result<(&[f32], &[f32]), PluginError> {
        self.in_l.iter_mut().for_each(|v| *v = 0.0);
        self.in_r.iter_mut().for_each(|v| *v = 0.0);
        self.process_inner(events, steady_time)
    }

    /// 效果器用法（混音台 insert）：就地处理输入音频。
    ///
    /// 输入拷贝进内部缓冲后走 process，输出由调用方拷回（或直接用返回切片覆盖）。
    pub fn process_effect(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: &[ClapInputEvent],
        steady_time: Option<u64>,
    ) -> Result<(), PluginError> {
        let frames = self.frames.min(left.len()).min(right.len());
        self.in_l[..frames].copy_from_slice(&left[..frames]);
        self.in_r[..frames].copy_from_slice(&right[..frames]);
        if frames < self.frames {
            self.in_l[frames..].iter_mut().for_each(|v| *v = 0.0);
            self.in_r[frames..].iter_mut().for_each(|v| *v = 0.0);
        }
        let (out_l, out_r) = self.process_inner(events, steady_time)?;
        left[..frames].copy_from_slice(&out_l[..frames]);
        right[..frames].copy_from_slice(&out_r[..frames]);
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
        self.out_l.iter_mut().for_each(|v| *v = 0.0);
        self.out_r.iter_mut().for_each(|v| *v = 0.0);

        let input_events = InputEvents::from_buffer(&self.input_events);
        let mut output_events = OutputEvents::from_buffer(&mut self.output_events);

        let input_audio = self.input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                [&mut self.in_l, &mut self.in_r]
                    .into_iter()
                    .map(InputChannel::variable),
            ),
        }]);
        let mut output_audio = self.output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                [&mut self.out_l, &mut self.out_r]
                    .into_iter()
                    .map(|b| b.as_mut_slice()),
            ),
        }]);

        let processor = self.processor.ensure_processing_started()?;
        processor.process(
            &input_audio,
            &mut output_audio,
            &input_events,
            &mut output_events,
            steady_time,
            None,
        )?;
        // 输出事件（如插件的 NoteEnd、参数回显）第一期不消费，直接丢弃。

        Ok((&self.out_l, &self.out_r))
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
