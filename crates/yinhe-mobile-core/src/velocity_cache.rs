//! 渲染线程预计算的力度摘要发布区。
//!
//! AM 力度条的数据由渲染线程在后台构建（`Renderer::poll_velocity_summary`），
//! Dart 不再自己做同步全量扫描：这里按 `vs_key = revision ^ visible_hash`
//! 发布最新一份摘要，`scene::velocity_bars` 只做切片。
//!
//! 键不匹配表示后台还没算完；Dart 侧稍后重试，而不是退回旧的全量遍历。

use std::sync::{Arc, Mutex, OnceLock};

use yinhe_wgpu::VelocitySummary;

type Published = (u64, Arc<VelocitySummary>);

static CELL: OnceLock<Mutex<Option<Published>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<Published>> {
    CELL.get_or_init(|| Mutex::new(None))
}

/// 发布一份摘要（渲染线程每帧调用；Arc clone 很便宜）。
pub fn publish(key: u64, summary: Arc<VelocitySummary>) {
    if let Ok(mut slot) = cell().lock() {
        *slot = Some((key, summary));
    }
}

/// 文档切换 / 摘要清空时调用，避免旧的可见性结果残留。
pub fn clear() {
    if let Ok(mut slot) = cell().lock() {
        *slot = None;
    }
}

/// 取与 `key` 完全匹配的摘要；不匹配（后台未完成或已过期）返回 `None`。
pub fn get(key: u64) -> Option<Arc<VelocitySummary>> {
    let slot = cell().lock().ok()?;
    slot.as_ref()
        .filter(|(k, _)| *k == key)
        .map(|(_, summary)| Arc::clone(summary))
}
