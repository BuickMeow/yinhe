use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::kind::ToastKind;

/// 通知上的操作按钮（如“打开文件夹”）。
#[derive(Clone, Debug)]
pub(crate) enum ToastActionKind {
    /// 在文件管理器中定位文件（打开所在目录）
    RevealInFolder(PathBuf),
}

/// 通知上的操作按钮。
#[derive(Clone, Debug)]
pub(crate) struct ToastAction {
    pub label: String,
    pub kind: ToastActionKind,
    /// 有图标画图标按钮（无文字，hover tooltip 显示 label），无则走文字分支。
    pub icon: Option<egui_material_icons::MaterialIcon>,
}

// ── 进度数据源（pull 式）：标题栏渲染时实时读取，后台任务只管写自己的共享状态，
// 不再每帧往通知层拷贝文案。完成/失败/中止时快照进 title/message，source 清空转静态。
pub(crate) trait ProgressSource: Send + Sync {
    /// 通知标题（各任务固定，如“正在加载”）。
    fn title(&self) -> String;
    /// 第二行：当前阶段文案。
    fn message(&self) -> String;
    /// 进度 0.0~1.0。
    fn fraction(&self) -> f32;
    /// 第三行：详情/百分比。
    fn detail(&self) -> String;
    /// 取消句柄（保存任务不支持取消，给 None）。
    fn cancel(&self) -> Option<Arc<AtomicBool>>;
    /// 暂停句柄（仅导出任务给 Some，其余默认 None → 暂停按钮自动只出现在导出任务）。
    fn pause(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

/// 进度任务的收尾方式：决定 kind、默认详情文案与结果通知的配色。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressOutcome {
    Completed,
    Failed,
    Aborted,
}

/// 阻塞式错误窗口的一条错误。
#[derive(Clone, Debug)]
pub(crate) struct ErrorReport {
    pub title: String,
    pub message: String,
}

/// 通知条目：进行中的任务（`source = Some`）或一条结果通知（`source = None`）。
/// 结果通知只保留一条，新结果覆盖旧结果；不维护历史。
pub(crate) struct Notification {
    pub(crate) id: u64,
    pub(crate) kind: ToastKind,
    pub(crate) title: String,
    pub(crate) message: String,
    /// 进度：None=普通通知，Some(0.0..1.0)=进度条
    pub(crate) progress: Option<f32>,
    pub(crate) progress_label: String,
    /// 取消句柄快照（进行中由 source 提供 live 句柄）。
    pub(crate) cancel: Option<Arc<AtomicBool>>,
    /// 进度任务进行中为 Some，渲染时 pull；结束后快照并清空。
    pub(crate) source: Option<Arc<dyn ProgressSource>>,
    /// 操作按钮（如“打开文件夹”）。
    pub(crate) action: Option<ToastAction>,
    /// stop 已点、中止确认中（按钮置灰，detail 显示“正在中止…”）。
    pub(crate) cancelling: bool,
    /// 用户已取消/收起：不再渲染，但保留到任务真正结束。
    pub(crate) dismissed: bool,
    /// 结果通知的过期时刻（None=进行中的任务，常驻）。
    pub(crate) expires_at: Option<Instant>,
}

impl Notification {
    /// 当前进度快照：进行中读 source，否则读冻结值。
    pub(crate) fn snapshot_fraction(&self) -> Option<f32> {
        self.source
            .as_ref()
            .map(|s| s.fraction().clamp(0.0, 1.0))
            .or(self.progress)
    }

    /// 渲染用解析值：有 source 读 live，无则读快照。
    /// 中止中 detail 覆盖为“正在中止…”（仍 1 行，高度不变）。
    /// 已暂停 detail 覆盖为“已暂停”（优先级低于“正在中止…”；仍 1 行，高度不变）。
    /// 返回 (标题, 正文, 进度, 详情)。
    pub(crate) fn resolve(&self) -> (String, String, Option<f32>, String) {
        if self.cancelling {
            if let Some(s) = &self.source {
                (
                    s.title(),
                    s.message(),
                    Some(s.fraction().clamp(0.0, 1.0)),
                    rust_i18n::t!("toast.cancelling").to_string(),
                )
            } else {
                (
                    self.title.clone(),
                    self.message.clone(),
                    self.progress,
                    rust_i18n::t!("toast.cancelling").to_string(),
                )
            }
        } else if let Some(s) = &self.source {
            // 已暂停覆盖 detail（进度条本身冻结，无需处理；仍 1 行高度不变）
            if s.pause()
                .is_some_and(|p| p.load(std::sync::atomic::Ordering::Relaxed))
            {
                (
                    s.title(),
                    s.message(),
                    Some(s.fraction().clamp(0.0, 1.0)),
                    rust_i18n::t!("toast.paused").to_string(),
                )
            } else {
                (
                    s.title(),
                    s.message(),
                    Some(s.fraction().clamp(0.0, 1.0)),
                    s.detail(),
                )
            }
        } else {
            (
                self.title.clone(),
                self.message.clone(),
                self.progress,
                self.progress_label.clone(),
            )
        }
    }

    /// 取消句柄解析：有 source 读 live（换任务时句柄会换），无则读快照。
    pub(crate) fn cancel_flag(&self) -> Option<Arc<AtomicBool>> {
        self.source
            .as_ref()
            .and_then(|s| s.cancel())
            .or_else(|| self.cancel.clone())
    }

    /// 暂停句柄解析：有 source 读 live；无 source 则无暂停（暂停只存在于进行中导出）。
    pub(crate) fn pause_flag(&self) -> Option<Arc<AtomicBool>> {
        self.source.as_ref().and_then(|s| s.pause())
    }
}
