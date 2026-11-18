//! 音频设备、命令协议与渲染线程的组装。
//!
//! 子模块：
//! - `command`：UI → renderer 命令、worker 命令/结果、预览参数；
//! - `handle`：UI 侧句柄、CPAL 句柄与电平表读数；
//! - `worker`：模型准备/音色库/chase 的后台线程；
//! - `device`：设备枚举、采样率/缓冲协商；
//! - `stream`：引擎初始化、renderer 线程、CPAL 流。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use crossbeam_channel::{Sender, bounded, unbounded};

use crate::audio_out::{self, BufferSize, Device, OutputFormat, Stream, StreamErrorKind};
use yinhe_core::YinModel;
use yinhe_mixer::{
    InsertProcessor, InstrumentProcessor, MasterParams, MeterReading, MixerParams, SampleReading,
    StripParams,
};
use yinhe_types::{Interpolation, SynthEngine};

use crate::audio_model::{PreparedModel, SortedCC};
use crate::audio_renderer::{RendererSharedState, spawn_renderer};
use crate::audio_ring::AudioRing;
use crate::channel::ChannelState;
use crate::channel_layout::ChannelLayout;

mod command;
mod device;
mod handle;
mod stream;
mod worker;

pub use command::{AudioCommand, InsertTarget, InstrumentPreviewNote, PreviewNoteParams};
pub use device::{discover_sample_rates, list_input_devices, list_output_devices};
pub use handle::{AudioHandle, CpalAudioHandle};
pub use stream::spawn_cpal_audio;

pub(crate) use command::{AmMsMap, ChasePurpose, WorkerCmd, WorkerResult};
#[cfg(test)]
pub(crate) use device::negotiate_sample_rate_in;
pub(crate) use device::{
    negotiate_buffer_size, negotiate_sample_rate, resolve_synth_engine, select_output_device,
};
pub(crate) use handle::MixerMeters;
pub(crate) use worker::spawn_worker;
#[cfg(test)]
pub(crate) use worker::{
    compute_chase_states_for_test, compute_chase_states_with_am_ms_for_test, merge_pending_commands,
};

const STEREO_CHANNELS: usize = 2;
const RING_BUFFER_FRAMES: usize = 16_384;

/// UI → renderer 命令通道容量。
///
/// 16 足够吸收任何合理的用户操作突发（按钮连点、设置切换、文档切换序列），
/// 同时硬性防止 renderer 卡死时命令无限堆积导致内存爆炸 + 鬼畜执行。
///
/// 队列满时 `AudioHandle::send` 走 `try_send` 丢弃新命令并记日志 ——
/// 不阻塞 UI 线程。renderer 已对 `ReloadNotes`/`UpdateNotes` 做同类型合并，
/// worker 对 `PrepareModel`/`PrepareNotes`/`PrepareChase` 也做合并，
/// 因此偶发丢弃只会导致短暂的 UI/音频错位，下一次用户操作即重新同步。
const AUDIO_CMD_CHANNEL_CAPACITY: usize = 16;
/// 编译期保证 `AudioRing` 的容量是 2 的幂（`AudioRing::new` 依赖此不变量做位运算取模）。
/// 任何修改 `RING_BUFFER_FRAMES` / `STEREO_CHANNELS` 或去掉 `.next_power_of_two()` 的改动
/// 都会在编译期触发 assert，而不是等到运行期才 panic。
const RING_CAPACITY: usize = (RING_BUFFER_FRAMES * STEREO_CHANNELS).next_power_of_two();
const _: () = assert!(
    RING_CAPACITY.is_power_of_two(),
    "RING_CAPACITY must be a power of two"
);

/// Analyse a YinModel and return the `ChannelLayout` (active_mask + channel_map).
///
/// A channel is "active" if any note with vel>1 lives on it, OR any
/// non-note control event is present on the owning track.
///
/// 返回的 `ChannelLayout` 在 `AudioEngine` 创建时定型，之后不可变。
/// 若 model 结构变化（增减音轨、改 channel/port），必须 teardown + 重建引擎。
pub fn channels_for_model(model: &YinModel) -> ChannelLayout {
    ChannelLayout::from_model(model)
}

#[cfg(test)]
#[path = "spawn/tests.rs"]
mod tests;
