use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::kind::ToastKind;

/// 浮动 toast 上的操作按钮（历史列表保持只读，不带按钮）。
#[derive(Clone, Debug)]
pub(crate) enum ToastActionKind {
    /// 在文件管理器中定位文件（打开所在目录）
    RevealInFolder(PathBuf),
}

/// 浮动 toast 上的操作按钮（历史列表保持只读，不带按钮）。
#[derive(Clone, Debug)]
pub(crate) struct ToastAction {
    pub label: String,
    pub kind: ToastActionKind,
    /// 有图标画图标按钮（无文字，hover tooltip 显示 label），无则走文字分支。
    pub icon: Option<egui_material_icons::MaterialIcon>,
}

/// 卡片一帧的交互结果（bool 四元组太 cryptic，收拢成结构）。
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct CardOutcome {
    pub dismiss: bool,
    pub cancel: bool,
    pub action: bool,
    pub hovered: bool,
}

// ── 进度数据源（pull 式）：卡片渲染时实时读取，后台任务只管写自己的共享状态，
// 不再每帧往通知层拷贝文案。完成/失败/中止时快照进 title/message，source 清空转静态。
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
    /// 暂停句柄（仅导出任务给 Some，其余默认 None → 暂停按钮自动只出现在导出卡）。
    fn pause(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

/// 进度任务的收尾方式：决定 kind、默认详情文案与自动收起档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressOutcome {
    Completed,
    Failed,
    Aborted,
}

/// 统一通知条目：浮动卡与通知中心列表共用同一份数据。
/// `on_screen` 是浮动卡可见性标记：收起（点 X/到期/关列表）只翻旗标，不搬数据；
/// 通知中心是同一列表的另一个视图，不再维护第二份历史。
pub(crate) struct Notification {
    pub(crate) id: u64,
    pub(crate) kind: ToastKind,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) created: Instant,
    /// 进度：None=普通通知，Some(0.0..1.0)=进度条
    pub(crate) progress: Option<f32>,
    pub(crate) progress_label: String,
    /// 取消句柄快照（进行中由 source 提供 live 句柄）。
    pub(crate) cancel: Option<Arc<AtomicBool>>,
    /// 进度任务进行中为 Some，渲染时 pull；结束后快照并清空。
    pub(crate) source: Option<Arc<dyn ProgressSource>>,
    /// 自动收起时刻（None=不自动收起；进行中的进度任务不计时，完成才起算）。
    pub(crate) collapse_at: Option<Instant>,
    /// 操作按钮（仅浮动卡显示，如“打开文件夹”）。
    pub(crate) action: Option<ToastAction>,
    /// 上帧渲染时指针是否悬停（悬停暂停自动收起计时）。
    pub(crate) hovered: bool,
    /// stop 已点、中止确认中（按钮置灰，detail 显示“正在中止…”）。
    pub(crate) cancelling: bool,
    /// 是否在浮动卡层（false=已收进通知中心，仅列表可见）。
    pub(crate) on_screen: bool,
    /// 浮动卡正在退场（320ms 后翻 off-screen）。
    pub(crate) leaving_since: Option<Instant>,
    /// 是否已读（通知中心未读点）。
    pub(crate) read: bool,
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
                    "正在中止…".to_string(),
                )
            } else {
                (
                    self.title.clone(),
                    self.message.clone(),
                    self.progress,
                    "正在中止…".to_string(),
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
                    "已暂停".to_string(),
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

    /// 暂停句柄解析：有 source 读 live；无 source 则无暂停（暂停只存在于进行中导出卡）。
    pub(crate) fn pause_flag(&self) -> Option<Arc<AtomicBool>> {
        self.source.as_ref().and_then(|s| s.pause())
    }
}
