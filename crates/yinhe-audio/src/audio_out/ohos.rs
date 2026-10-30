//! OHAudio 后端（OpenHarmony / HarmonyOS）。
//!
//! 只支持默认输出设备：OHAudio 没有"枚举设备"的概念（路由由系统决定）。
//! 采样率 / 声道由调用方给定，写入回调直接从 ring 里取已渲染样本。

use std::ffi::c_void;

use super::{BufferSize, OutputFormat, StreamError};

type OhBuilder = c_void;
type OhRenderer = c_void;
type OhResult = i32;

const AUDIOSTREAM_SUCCESS: OhResult = 0;
const AUDIOSTREAM_TYPE_RENDERER: i32 = 1;
const AUDIOSTREAM_SAMPLE_F32LE: i32 = 4;
const AUDIOSTREAM_ENCODING_RAW: i32 = 0;
const AUDIOSTREAM_LATENCY_NORMAL: i32 = 0;
const AUDIOSTREAM_USAGE_MUSIC: i32 = 1;
const AUDIO_DATA_CALLBACK_RESULT_VALID: i32 = 0;

type WriteCallback = unsafe extern "C" fn(
    renderer: *mut OhRenderer,
    user_data: *mut c_void,
    audio_data: *mut c_void,
    audio_data_size: i32,
) -> i32;

#[link(name = "ohaudio")]
unsafe extern "C" {
    fn OH_AudioStreamBuilder_Create(builder: *mut *mut OhBuilder, stream_type: i32) -> OhResult;
    fn OH_AudioStreamBuilder_Destroy(builder: *mut OhBuilder) -> OhResult;
    fn OH_AudioStreamBuilder_SetSamplingRate(builder: *mut OhBuilder, rate: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetChannelCount(builder: *mut OhBuilder, channels: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetSampleFormat(builder: *mut OhBuilder, format: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetEncodingType(builder: *mut OhBuilder, encoding: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetLatencyMode(builder: *mut OhBuilder, mode: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetRendererInfo(builder: *mut OhBuilder, usage: i32) -> OhResult;
    fn OH_AudioStreamBuilder_SetRendererWriteDataCallback(
        builder: *mut OhBuilder,
        callback: WriteCallback,
        user_data: *mut c_void,
    ) -> OhResult;
    fn OH_AudioStreamBuilder_GenerateRenderer(
        builder: *mut OhBuilder,
        renderer: *mut *mut OhRenderer,
    ) -> OhResult;
    fn OH_AudioRenderer_Start(renderer: *mut OhRenderer) -> OhResult;
    fn OH_AudioRenderer_Pause(renderer: *mut OhRenderer) -> OhResult;
    fn OH_AudioRenderer_Stop(renderer: *mut OhRenderer) -> OhResult;
    fn OH_AudioRenderer_Release(renderer: *mut OhRenderer) -> OhResult;
}

/// 回调侧状态：渲染回调在音频线程上取走这里的闭包。
struct CallbackState {
    data: Box<dyn FnMut(&mut [f32]) + Send>,
}

unsafe extern "C" fn on_write_data(
    _renderer: *mut OhRenderer,
    user_data: *mut c_void,
    audio_data: *mut c_void,
    audio_data_size: i32,
) -> i32 {
    if user_data.is_null() || audio_data.is_null() || audio_data_size <= 0 {
        return AUDIO_DATA_CALLBACK_RESULT_VALID;
    }
    // SAFETY: user_data 由 Stream 持有，生命周期覆盖整个渲染器；
    // audio_data 是后端提供的 f32 缓冲区，长度 = 字节数 / 4。
    let state = unsafe { &mut *(user_data as *mut CallbackState) };
    let samples = audio_data_size as usize / std::mem::size_of::<f32>();
    let out = unsafe { std::slice::from_raw_parts_mut(audio_data as *mut f32, samples) };
    (state.data)(out);
    AUDIO_DATA_CALLBACK_RESULT_VALID
}

pub(crate) struct Device;

pub(crate) struct Stream {
    renderer: *mut OhRenderer,
    state: *mut CallbackState,
}

// OHAudio 的 renderer 句柄本身是线程安全的（Start/Pause/Release 可在任意线程调用），
// 回调状态只被音频线程使用；这里把裸指针包装成可跨线程持有的句柄。
unsafe impl Send for Stream {}
unsafe impl Sync for Stream {}

fn check(what: &str, result: OhResult) -> Result<(), String> {
    if result == AUDIOSTREAM_SUCCESS {
        Ok(())
    } else {
        Err(format!("OHAudio {what} failed: {result}"))
    }
}

impl Device {
    pub(crate) fn default_format(&self) -> Result<OutputFormat, String> {
        // OHAudio 不做能力枚举；48kHz/立体声是所有鸿蒙设备都支持的组合。
        Ok(OutputFormat {
            sample_rate: 48_000,
            channels: 2,
        })
    }

    /// 无枚举能力：返回 `None`，调用方保持请求值。
    pub(crate) fn f32_rate_ranges(&self) -> Option<Vec<(u32, u32)>> {
        None
    }

    /// 无枚举能力：返回 `None`，不做缓冲区钳制。
    pub(crate) fn buffer_range(&self) -> Option<(u32, u32)> {
        None
    }

    pub(crate) fn build_output<D, E>(
        &self,
        format: OutputFormat,
        _buffer: BufferSize,
        data: D,
        _error: E,
    ) -> Result<Stream, String>
    where
        D: FnMut(&mut [f32]) + Send + 'static,
        E: FnMut(StreamError) + Send + 'static,
    {
        let state = Box::into_raw(Box::new(CallbackState {
            data: Box::new(data),
        }));

        let mut builder: *mut OhBuilder = std::ptr::null_mut();
        let mut renderer: *mut OhRenderer = std::ptr::null_mut();

        let mut build = || -> Result<(), String> {
            // SAFETY: 所有调用都成对出现，失败路径在下方统一清理。
            unsafe {
                check(
                    "Create",
                    OH_AudioStreamBuilder_Create(&mut builder, AUDIOSTREAM_TYPE_RENDERER),
                )?;
                check(
                    "SetSamplingRate",
                    OH_AudioStreamBuilder_SetSamplingRate(builder, format.sample_rate as i32),
                )?;
                check(
                    "SetChannelCount",
                    OH_AudioStreamBuilder_SetChannelCount(builder, format.channels as i32),
                )?;
                check(
                    "SetSampleFormat",
                    OH_AudioStreamBuilder_SetSampleFormat(builder, AUDIOSTREAM_SAMPLE_F32LE),
                )?;
                check(
                    "SetEncodingType",
                    OH_AudioStreamBuilder_SetEncodingType(builder, AUDIOSTREAM_ENCODING_RAW),
                )?;
                check(
                    "SetLatencyMode",
                    OH_AudioStreamBuilder_SetLatencyMode(builder, AUDIOSTREAM_LATENCY_NORMAL),
                )?;
                check(
                    "SetRendererInfo",
                    OH_AudioStreamBuilder_SetRendererInfo(builder, AUDIOSTREAM_USAGE_MUSIC),
                )?;
                check(
                    "SetRendererWriteDataCallback",
                    OH_AudioStreamBuilder_SetRendererWriteDataCallback(
                        builder,
                        on_write_data,
                        state as *mut c_void,
                    ),
                )?;
                check(
                    "GenerateRenderer",
                    OH_AudioStreamBuilder_GenerateRenderer(builder, &mut renderer),
                )?;
            }
            Ok(())
        };

        let result = build();
        // builder 生成 renderer 后即可销毁（renderer 独立存活）。
        if !builder.is_null() {
            // SAFETY: builder 由 Create 创建。
            unsafe {
                let _ = OH_AudioStreamBuilder_Destroy(builder);
            }
        }
        if let Err(e) = result {
            if !renderer.is_null() {
                unsafe {
                    let _ = OH_AudioRenderer_Release(renderer);
                }
            }
            // SAFETY: state 由 Box::into_raw 创建，尚未交给任何一方。
            unsafe { drop(Box::from_raw(state)) };
            return Err(e);
        }

        Ok(Stream { renderer, state })
    }
}

impl Stream {
    pub(crate) fn play(&self) -> Result<(), String> {
        // SAFETY: renderer 在 Stream 生命周期内有效。
        unsafe { check("Start", OH_AudioRenderer_Start(self.renderer)) }
    }

    pub(crate) fn pause(&self) -> Result<(), String> {
        // SAFETY: 同上。
        unsafe { check("Pause", OH_AudioRenderer_Pause(self.renderer)) }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        // SAFETY: 先停流（保证音频线程不再进入回调），再释放 renderer 与回调状态。
        unsafe {
            if !self.renderer.is_null() {
                let _ = OH_AudioRenderer_Stop(self.renderer);
                let _ = OH_AudioRenderer_Release(self.renderer);
            }
            if !self.state.is_null() {
                drop(Box::from_raw(self.state));
            }
        }
    }
}

pub(crate) fn default_output_device() -> Option<Device> {
    Some(Device)
}

pub(crate) fn find_output_device(_name: Option<&str>) -> Result<Device, String> {
    // 无法按名字选择：系统决定路由。
    Ok(Device)
}

pub(crate) fn list_devices(output: bool) -> Vec<String> {
    if output {
        vec!["默认输出设备".to_string()]
    } else {
        Vec::new()
    }
}
