//! CLAP 处理器 → 混音台 insert 的适配器。
//!
//! 生命周期约定（线程模型见 yinhe-clap 模块文档）：
//! - UI/管理线程：`ClapPluginInstance` 加载、`activate()` 产出 `ClapProcessor`；
//! - 渲染线程：本适配器持有处理器跑 `process`（旁通为 Arc 原子标志，无需命令往返）；
//! - 回收：渲染线程经 return 通道退回 → UI 线程 downcast 拿回 →
//!   `ClapPluginInstance::deactivate()`。`owner` 是混音器机架分配的槽位 id，
//!   用于把退回的处理器匹配回原实例。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use yinhe_clap::ClapProcessor;
use yinhe_mixer::InsertProcessor;

pub struct ClapInsert {
    processor: ClapProcessor,
    /// 旁通标志：UI 写、渲染线程读（无锁）。
    bypass: Arc<AtomicBool>,
    /// 机架槽位 id（回收匹配用）。
    owner: u64,
}

impl ClapInsert {
    pub fn new(processor: ClapProcessor, bypass: Arc<AtomicBool>, owner: u64) -> Self {
        Self {
            processor,
            bypass,
            owner,
        }
    }

    /// 拆回部件（回收路径：processor 交还实例 deactivate）。
    pub fn into_parts(self) -> (ClapProcessor, Arc<AtomicBool>, u64) {
        (self.processor, self.bypass, self.owner)
    }
}

impl InsertProcessor for ClapInsert {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.bypass.load(Ordering::Relaxed) {
            return;
        }
        // 效果器处理失败（插件回调返回错误）：静默旁通这一块，
        // 比 panic 杀渲染线程（丢未保存工程）好——记日志留给诊断。
        if let Err(e) = self.processor.process_effect(left, right, &[], None) {
            tracing::warn!(target: "clap-insert", "insert 处理失败，本块旁通: {e}");
        }
    }

    fn reset(&mut self) {
        self.processor.reset();
    }

    fn into_any(self: Box<Self>) -> Box<dyn std::any::Any> {
        self
    }
}
