//! yinhe 混音台核心。
//!
//! 分层：
//! - [`params`]：serde 持久化结构，存进工程文件（按源 MIDI 通道 A01..P16 索引）。
//! - [`strip`]：单个通道条的运行时状态（增益/声像斜坡，抗 zipper noise）。
//! - [`meter`]：峰值电平表，渲染线程写、UI 线程读（Arc<AtomicU32> 模式，
//!   与 yinhe-audio 的 sample_position 相同）。
//! - [`graph`]：渲染线程持有的处理图，处理期间零分配、零锁。
//!
//! 依赖方向：本 crate 不依赖任何插件/合成器 crate。
//! insert 效果通过 [`graph::InsertProcessor`] trait 由上层（yinhe-audio）适配接入。

mod graph;
mod meter;
mod params;
mod strip;

pub use graph::{ChannelBuffers, InsertProcessor, MixerGraph};
pub use meter::{MeterReading, MeterTap};
pub use params::{CHANNEL_COUNT, InsertRef, MasterParams, MixerParams, StripParams};
