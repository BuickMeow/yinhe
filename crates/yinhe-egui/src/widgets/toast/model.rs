use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::kind::ToastKind;

// ── 进度数据源（pull 式）：卡片渲染时实时读取，后台任务只管写自己的共享状态，
// 不再每帧往通知层拷贝文案。完成/失败时快照进 title/message，source 清空转静态。
pub(crate) trait ProgressSource: Send + Sync {
    /// 卡片标题（各任务固定，如“正在加载”）。
    fn title(&self) -> String;
    /// 第二行：当前阶段文案。
    fn message(&self) -> String;
    /// 进度 0.0~1.0。
    fn fraction(&self) -> f32;
    /// 第三行：详情/百分比。
    fn detail(&self) -> String;
    /// 取消句柄（保存任务不支持取消，给 None）。
    fn cancel(&self) -> Option<Arc<AtomicBool>>;
}

// ── 单条 Toast（常驻，需手动关闭）──
pub(crate) struct Toast {
    pub(crate) id: u64,
    pub(crate) kind: ToastKind,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) created: Instant,
    /// 进度：None=普通通知，Some(0.0..1.0)=进度条
    pub(crate) progress: Option<f32>,
    pub(crate) progress_label: String,
    pub(crate) cancel: Option<Arc<AtomicBool>>,
    pub(crate) leaving_since: Option<Instant>,
    /// 进度任务进行中为 Some，渲染时 pull；完成后快照并清空。
    pub(crate) source: Option<std::sync::Arc<dyn ProgressSource>>,
}

// ── 历史记录（持久，与 Toast 同尺寸以便复用）──
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct HistoryEntry {
    pub(crate) id: u64,
    pub(crate) kind: ToastKind,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) created: Instant,
    pub(crate) read: bool,
    pub(crate) progress: Option<f32>,
    pub(crate) progress_label: String,
    pub(crate) source: Option<std::sync::Arc<dyn ProgressSource>>,
}

/// 渲染用解析值：有 source 读 live，无则读快照。
/// 返回 (标题, 正文, 进度, 详情)。
pub(crate) fn resolve_toast(t: &Toast) -> (String, String, Option<f32>, String) {
    if let Some(s) = &t.source {
        (
            s.title(),
            s.message(),
            Some(s.fraction().clamp(0.0, 1.0)),
            s.detail(),
        )
    } else {
        (
            t.title.clone(),
            t.message.clone(),
            t.progress,
            t.progress_label.clone(),
        )
    }
}

pub(crate) fn resolve_history(h: &HistoryEntry) -> (String, String, Option<f32>, String) {
    if let Some(s) = &h.source {
        (
            s.title(),
            s.message(),
            Some(s.fraction().clamp(0.0, 1.0)),
            s.detail(),
        )
    } else {
        (
            h.title.clone(),
            h.message.clone(),
            h.progress,
            h.progress_label.clone(),
        )
    }
}

/// 取消句柄解析：有 source 读 live（换任务时句柄会换），无则读快照。
pub(crate) fn resolve_cancel_toast(t: &Toast) -> Option<Arc<AtomicBool>> {
    t.source
        .as_ref()
        .and_then(|s| s.cancel())
        .or_else(|| t.cancel.clone())
}
