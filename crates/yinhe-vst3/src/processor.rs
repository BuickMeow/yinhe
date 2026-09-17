//! VST3 音频处理器（渲染线程侧）。
//!
//! 管理线程 `Vst3PluginInstance::activate_audio` 产出本类型（`Send`），
//! move 进渲染线程后每块调用 [`Vst3Processor::process`]。
//! 第一版只支持主音频端口（立体声进/出）+ 单事件总线；其余总线不激活。

use std::ptr;
use std::sync::Arc;

use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentTrait, IEventList, IParameterChanges, ProcessContext,
    ProcessContext_::StatesAndFlags_::{kPlaying, kTempoValid, kTimeSigValid},
    ProcessData,
    ProcessModes_::kRealtime,
    SymbolicSampleSizes_::kSample32,
};
use vst3::{ComPtr, ComWrapper};
use yinhe_mixer::{ParamQueue, PluginEvent};

use crate::event_list::{HostEventList, HostParamChanges};
use crate::events::plugin_event_to_vst;

/// 音频处理失败。
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error("插件 process 返回失败（{0:#x}）")]
    ProcessFailed(i32),
}

/// 渲染线程侧处理器（管理线程激活产出、独占调用）。
///
/// 实时约束：`process` 内不分配（host 对象/缓冲全部构造时分配）。
pub struct Vst3Processor {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
    // host 侧 COM 对象与其接口指针（保持引用计数）。
    in_events: ComWrapper<HostEventList>,
    in_event_ptr: ComPtr<IEventList>,
    out_events: ComWrapper<HostEventList>,
    out_event_ptr: ComPtr<IEventList>,
    in_params: ComWrapper<HostParamChanges>,
    in_param_ptr: ComPtr<IParameterChanges>,
    /// 输出参数变化对象（持有引用计数；第一版不消费）。
    _out_params: ComWrapper<HostParamChanges>,
    out_param_ptr: ComPtr<IParameterChanges>,
    // 音频缓冲：主端口立体声 planar（ch0 = [0..frames]，ch1 = [frames..2*frames]）。
    in_buf: Vec<f32>,
    out_buf: Vec<f32>,
    /// 每 bus 的 channel data 指针数组（指向 in_buf/out_buf 的堆数据）。
    in_channels: Vec<*mut f32>,
    out_channels: Vec<*mut f32>,
    buses_in: Vec<AudioBusBuffers>,
    buses_out: Vec<AudioBusBuffers>,
    context: Box<ProcessContext>,
    frames: usize,
    sample_rate: f64,
    /// UI → 渲染线程的参数变化队列。
    param_queue: Arc<ParamQueue>,
    param_scratch: Vec<(u32, f64)>,
}

// SAFETY: 裸指针只指向本结构持有的缓冲；处理器整体 move 进渲染线程后独占访问。
unsafe impl Send for Vst3Processor {}

impl Vst3Processor {
    /// 构造处理器（由 `activate_audio` 调用；缓冲一次分配）。
    pub(crate) fn new(
        component: ComPtr<IComponent>,
        processor: ComPtr<IAudioProcessor>,
        sample_rate: f64,
        frames: usize,
        param_queue: Arc<ParamQueue>,
    ) -> Option<Self> {
        let in_events = ComWrapper::new(HostEventList::new());
        let in_event_ptr = in_events.to_com_ptr::<IEventList>()?;
        let out_events = ComWrapper::new(HostEventList::new());
        let out_event_ptr = out_events.to_com_ptr::<IEventList>()?;
        let in_params = ComWrapper::new(HostParamChanges::new());
        let in_param_ptr = in_params.to_com_ptr::<IParameterChanges>()?;
        let out_params = ComWrapper::new(HostParamChanges::new());
        let out_param_ptr = out_params.to_com_ptr::<IParameterChanges>()?;

        let mut in_buf = vec![0.0f32; frames * 2];
        let mut out_buf = vec![0.0f32; frames * 2];
        let mut in_channels = vec![in_buf.as_mut_ptr(), unsafe {
            in_buf.as_mut_ptr().add(frames)
        }];
        let mut out_channels = vec![out_buf.as_mut_ptr(), unsafe {
            out_buf.as_mut_ptr().add(frames)
        }];
        let buses_in = vec![AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: in_channels.as_mut_ptr(),
            },
        }];
        let buses_out = vec![AudioBusBuffers {
            numChannels: 2,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: out_channels.as_mut_ptr(),
            },
        }];
        let context = Box::new(unsafe { std::mem::zeroed::<ProcessContext>() });

        Some(Self {
            component,
            processor,
            in_events,
            in_event_ptr,
            out_events,
            out_event_ptr,
            in_params,
            in_param_ptr,
            _out_params: out_params,
            out_param_ptr,
            in_buf,
            out_buf,
            in_channels,
            out_channels,
            buses_in,
            buses_out,
            context,
            frames,
            sample_rate,
            param_queue,
            param_scratch: Vec::new(),
        })
    }

    /// 处理一块。
    ///
    /// `input`：效果器用法时为主端口立体声输入；乐器传 `None`（输入静音）。
    /// `position_samples`：本块起始的工程时间（采样数）。
    pub fn process_block(
        &mut self,
        events: &[PluginEvent],
        position_samples: u64,
        input: Option<(&[f32], &[f32])>,
    ) -> Result<(), ProcessError> {
        let frames = self.frames;
        // 输入。
        match input {
            Some((l, r)) => {
                let n = l.len().min(frames);
                self.in_buf[..n].copy_from_slice(&l[..n]);
                self.in_buf[n..frames].fill(0.0);
                let n = r.len().min(frames);
                self.in_buf[frames..frames + n].copy_from_slice(&r[..n]);
                self.in_buf[frames + n..frames * 2].fill(0.0);
            }
            None => self.in_buf.fill(0.0),
        }

        // 输入事件（ParamValue 走 IParameterChanges，其余走事件列表）。
        self.in_events.clear();
        self.in_params.clear();
        for e in events {
            match e {
                PluginEvent::ParamValue {
                    time,
                    param_id,
                    value,
                } => self.in_params.push_point(*param_id, *time as i32, *value),
                _ => {
                    if let Some(ev) = plugin_event_to_vst(e) {
                        self.in_events.push(ev);
                    }
                }
            }
        }

        // 参数变化（UI 队列 → IParameterChanges；值已是归一化）。
        self.param_queue.take_into(&mut self.param_scratch);
        for &(id, value) in &self.param_scratch {
            self.in_params.push_point(id, 0, value);
        }
        self.param_scratch.clear();

        // 处理上下文。
        self.context.projectTimeSamples = position_samples as i64;
        // 常量类型随平台绑定不同（i32/u32），统一转换。
        #[allow(clippy::unnecessary_cast)]
        let state = (kPlaying | kTempoValid | kTimeSigValid) as u32;
        self.context.state = state;
        self.context.sampleRate = self.sample_rate;
        self.context.tempo = 120.0;
        self.context.timeSigNumerator = 4;
        self.context.timeSigDenominator = 4;

        let mut data = ProcessData {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            numSamples: frames as i32,
            numInputs: self.buses_in.len() as i32,
            numOutputs: self.buses_out.len() as i32,
            inputs: self.buses_in.as_mut_ptr(),
            outputs: self.buses_out.as_mut_ptr(),
            inputParameterChanges: self.in_param_ptr.as_ptr(),
            outputParameterChanges: self.out_param_ptr.as_ptr(),
            inputEvents: self.in_event_ptr.as_ptr(),
            outputEvents: self.out_event_ptr.as_ptr(),
            processContext: self.context.as_mut(),
        };
        let r = unsafe { self.processor.process(&mut data) };
        if r != vst3::Steinberg::kResultOk {
            return Err(ProcessError::ProcessFailed(r));
        }
        Ok(())
    }

    /// 主端口输出（planar 立体声）。
    pub fn output(&self) -> (&[f32], &[f32]) {
        let frames = self.frames;
        (&self.out_buf[..frames], &self.out_buf[frames..frames * 2])
    }

    /// 取出插件输出事件（当前仅保留，供后续 NoteEnd/CC 回传使用）。
    pub fn take_output_events(&self) -> Vec<vst3::Steinberg::Vst::Event> {
        self.out_events.take()
    }

    /// 停止处理（管理线程 deactivate 前调用；渲染线程不可调）。
    pub fn stop(&self) {
        unsafe {
            let _ = self.processor.setProcessing(0);
            let _ = self.component.setActive(0);
        }
    }
}

impl Drop for Vst3Processor {
    fn drop(&mut self) {
        // 只释放引用；插件状态（setActive/setProcessing）由管理线程 stop() 处理。
        let _ = (&self.in_channels, &self.out_channels, ptr::null::<u8>());
    }
}

impl yinhe_mixer::InstrumentProcessor for Vst3Processor {
    fn max_block_frames(&self) -> usize {
        self.frames
    }

    fn process(
        &mut self,
        events: &[PluginEvent],
        out_l: &mut [f32],
        out_r: &mut [f32],
        position_samples: u64,
    ) {
        if let Err(e) = self.process_block(events, position_samples, None) {
            tracing::warn!(target: "vst3-instrument", "乐器处理失败，本块静音: {e}");
            out_l.fill(0.0);
            out_r.fill(0.0);
            return;
        }
        let (ol, or) = self.output();
        let n = out_l.len().min(out_r.len()).min(ol.len()).min(or.len());
        out_l[..n].copy_from_slice(&ol[..n]);
        out_r[..n].copy_from_slice(&or[..n]);
        out_l[n..].fill(0.0);
        out_r[n..].fill(0.0);
    }

    fn reset(&mut self) {
        // VST3 无 reset API：setActive 循环让插件重置内部状态（尾音/延迟清零）。
        unsafe {
            let _ = self.processor.setProcessing(0);
            let _ = self.component.setActive(0);
            let _ = self.component.setActive(1);
            let _ = self.processor.setProcessing(1);
        }
    }

    fn flush_pending_params(&mut self, position_samples: u64) {
        if self.param_queue.is_empty() {
            return;
        }
        // 跑一个静音块把参数送达插件（输出丢弃）。
        let _ = self.process_block(&[], position_samples, None);
    }

    fn latency_samples(&self) -> u32 {
        unsafe { self.processor.getLatencySamples() }
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any + Send> {
        self
    }
}

/// VST3 效果器 → 混音台 insert 适配器。
///
/// 与 CLAP 的 `ClapInsert` 同模型：处理器由管理线程激活产出，
/// 渲染线程独占调用，回收时 move 回管理线程 stop()/销毁。
pub struct Vst3Insert {
    processor: Vst3Processor,
}

impl Vst3Insert {
    pub fn new(processor: Vst3Processor) -> Self {
        Self { processor }
    }

    /// 拆回处理器（回收路径：交还实例/管理线程）。
    pub fn into_processor(self) -> Vst3Processor {
        self.processor
    }
}

impl yinhe_mixer::InsertProcessor for Vst3Insert {
    fn max_block_frames(&self) -> usize {
        self.processor.frames
    }

    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Err(e) = self
            .processor
            .process_block(&[], 0, Some((&*left, &*right)))
        {
            tracing::warn!(target: "vst3-insert", "insert 处理失败，本块旁通: {e}");
            return;
        }
        let (ol, or) = self.processor.output();
        let n = left.len().min(right.len()).min(ol.len()).min(or.len());
        left[..n].copy_from_slice(&ol[..n]);
        right[..n].copy_from_slice(&or[..n]);
    }

    fn reset(&mut self) {
        yinhe_mixer::InstrumentProcessor::reset(&mut self.processor);
    }

    fn flush_pending_params(&mut self, position_samples: u64) {
        yinhe_mixer::InstrumentProcessor::flush_pending_params(
            &mut self.processor,
            position_samples,
        );
    }

    fn latency_samples(&self) -> u32 {
        yinhe_mixer::InstrumentProcessor::latency_samples(&self.processor)
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}
