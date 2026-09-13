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
    pub fn process(
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

        // 输入事件。
        self.in_events.clear();
        for e in events {
            if let Some(ev) = plugin_event_to_vst(e) {
                self.in_events.push(ev);
            }
        }

        // 参数变化（UI 队列 → IParameterChanges）。
        self.in_params.clear();
        self.param_queue.take_into(&mut self.param_scratch);
        for &(id, value) in &self.param_scratch {
            self.in_params.push_point(id, 0, value);
        }
        self.param_scratch.clear();

        // 处理上下文。
        self.context.projectTimeSamples = position_samples as i64;
        self.context.state = kPlaying | kTempoValid | kTimeSigValid;
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
