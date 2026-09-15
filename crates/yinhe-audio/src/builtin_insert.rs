//! 内置效果器（yinhe-dsp）→ 混音台 insert 的适配器。
//!
//! 与 [`crate::clap_insert::ClapInsert`] 同模型：
//! - 旁通标志是 `Arc<AtomicBool>`（UI 写、渲染线程读，无命令往返）；
//! - `owner` 是机架槽位 id，处理器退回时用于匹配回原槽位；
//! - 内置处理器无 deactivate 需求，回收时直接释放。
//!
//! trait 转发（含 `handled_ccs`/`apply_cc`）保证混音图的 CC 接管掩码
//! 与 CC 广播对内置模块透明。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use yinhe_mixer::InsertProcessor;

/// 内置效果器的 insert 包装（旁通 + 回收 owner）。
pub struct BuiltinInsert {
    inner: Box<dyn InsertProcessor>,
    /// 旁通标志：UI 写、渲染线程读（无锁）。
    bypass: Arc<AtomicBool>,
    /// 机架槽位 id（回收匹配用）。
    owner: u64,
}

impl BuiltinInsert {
    pub fn new(inner: Box<dyn InsertProcessor>, bypass: Arc<AtomicBool>, owner: u64) -> Self {
        Self {
            inner,
            bypass,
            owner,
        }
    }

    /// 拆回部件（回收路径）。
    pub fn into_parts(self) -> (Box<dyn InsertProcessor>, Arc<AtomicBool>, u64) {
        (self.inner, self.bypass, self.owner)
    }
}

impl InsertProcessor for BuiltinInsert {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.bypass.load(Ordering::Relaxed) {
            return;
        }
        self.inner.process(left, right);
    }

    fn reset(&mut self) {
        self.inner.reset();
    }

    fn flush_pending_params(&mut self, position_samples: u64) {
        self.inner.flush_pending_params(position_samples);
    }

    fn latency_samples(&self) -> u32 {
        self.inner.latency_samples()
    }

    fn handled_ccs(&self) -> &'static [u8] {
        self.inner.handled_ccs()
    }

    fn apply_cc(&mut self, cc: u8, value: u8) {
        self.inner.apply_cc(cc, value);
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}
