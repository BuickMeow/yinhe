use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::kind::ToastKind;

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
}
