use std::collections::VecDeque;
use std::sync::Arc;

use super::kind::ToastKind;
use super::model::{ErrorReport, Notification, ProgressOutcome, ProgressSource};

mod render;

#[cfg(test)]
mod tests;

pub const LOADING_PROGRESS_ID: u64 = 0x4C4F4144; // "LOAD"
pub const SAVE_PROGRESS_ID: u64 = 0x53415645; // "SAVE"
pub const EXPORT_PROGRESS_ID: u64 = 0x45585054; // "EXPT"
pub const MIDI_EXPORT_PROGRESS_ID: u64 = 0x4D455849; // "MEXI"
pub const RESCALE_PROGRESS_ID: u64 = 0x5253434C; // "RSCL"
pub const ENGINE_PROGRESS_ID: u64 = 0x454E474E; // "ENGN"
pub const ARCHIVE_PROGRESS_ID: u64 = 0x41524348; // "ARCH"

/// 错误队列上限：阻塞窗口逐条确认，异常情况下也不能无限堆积。
const MAX_ERRORS: usize = 32;

/// 标题栏通知：最多 6 个进行中的任务 + 1 条结果通知，不保存历史。
/// 错误不在这里展示，单独进阻塞队列（`front_error` / `pop_error`）。
pub struct Notifications {
    next_id: u64,
    /// 进行中的任务 + 至多一条结果通知。
    items: Vec<Notification>,
    /// 主槽显示的条目；失效时自动回退到最后一个活动任务/结果通知。
    primary: Option<u64>,
    /// 多任务弹窗是否展开（点击 `+N`）。
    tasks_open: bool,
    /// 阻塞式错误队列，一次显示一条。
    errors: VecDeque<ErrorReport>,
}

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

impl Notifications {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            items: Vec::new(),
            primary: None,
            tasks_open: false,
            errors: VecDeque::new(),
        }
    }

    pub(super) fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(super) fn get(&self, id: u64) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    // ── 普通通知（结果反馈）──

    pub fn success(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Success, title, message)
    }

    /// 错误统一进阻塞队列，不再作为一条会自动消失的通知。
    pub fn error(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.report_error(title, message);
        0
    }

    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        if kind == ToastKind::Error {
            self.report_error(title, message);
            return 0;
        }
        // 结果通知只保留最新一条；进行中的任务不受影响。
        self.items.retain(|n| n.source.is_some());
        let id = self.alloc_id();
        self.items.push(Notification {
            id,
            kind,
            title: title.into(),
            message: message.into(),
            progress: None,
            cancel: None,
            source: None,
            action: None,
            cancelling: false,
            dismissed: false,
        });
        self.primary = Some(id);
        id
    }

    // ── 进度任务 ──

    /// 确保进度任务存在：同一 key（如 LOADING_PROGRESS_ID）复用同一 id。
    /// 只换数据源（Arc 交换，无文案拷贝），进度文案渲染时 pull。
    /// 用户取消后（dismissed）只更新数据、不再显示。
    pub fn ensure_progress(&mut self, id: u64, kind: ToastKind, source: Arc<dyn ProgressSource>) {
        let title = source.title();
        let mut promote = false;
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            let was_done = n.source.is_none();
            n.kind = kind;
            n.source = Some(source);
            n.cancel = n.source.as_ref().and_then(|s| s.cancel());
            n.cancelling = false;
            if was_done {
                // 上一任务已结束，同一 key 被重新使用：重置为进行中。
                n.title = title;
                n.message.clear();
                n.progress = None;
                n.dismissed = false;
                promote = true;
            }
        } else {
            self.items.push(Notification {
                id,
                kind,
                title,
                message: String::new(),
                progress: None,
                cancel: source.cancel(),
                source: Some(source),
                action: None,
                cancelling: false,
                dismissed: false,
            });
            promote = true;
        }
        if promote {
            self.primary = Some(id);
        }
    }

    /// 进度任务收尾：
    /// - 完成/中止 → 原地转成一条结果通知（5s 后消失），返回新 id 供 set_action 用。
    /// - 失败 → 移除任务并把错误推入阻塞队列，返回 0。
    pub fn finish_progress(
        &mut self,
        id: u64,
        outcome: ProgressOutcome,
        title: impl Into<String>,
        message: impl Into<String>,
        _detail: Option<String>,
    ) -> u64 {
        let Some(idx) = self.items.iter().position(|n| n.id == id) else {
            // 条目不存在（被取消清理等）：回退普通通知/错误。
            return match outcome {
                ProgressOutcome::Completed => self.push(ToastKind::Success, title, message),
                ProgressOutcome::Aborted => self.push(ToastKind::Warning, title, message),
                ProgressOutcome::Failed => {
                    self.report_error(title, message);
                    0
                }
            };
        };
        if outcome == ProgressOutcome::Failed {
            self.items.remove(idx);
            if self.primary == Some(id) {
                self.primary = None;
            }
            self.report_error(title, message);
            return 0;
        }

        let promoted = self.alloc_id();
        {
            let n = &mut self.items[idx];
            n.id = promoted;
            n.kind = match outcome {
                ProgressOutcome::Completed => ToastKind::Success,
                ProgressOutcome::Aborted => ToastKind::Warning,
                ProgressOutcome::Failed => unreachable!(),
            };
            n.title = title.into();
            n.message = message.into();
            // 完成满格；中止保留中断时刻快照。
            n.progress = match outcome {
                ProgressOutcome::Completed => Some(1.0),
                ProgressOutcome::Aborted => n.snapshot_fraction(),
                ProgressOutcome::Failed => None,
            };
            n.cancel = None;
            n.source = None;
            n.cancelling = false;
            n.dismissed = false;
        }
        self.primary = Some(promoted);
        promoted
    }

    /// 给通知挂操作按钮（如“打开文件夹”）。
    pub fn set_action_with_icon(
        &mut self,
        id: u64,
        label: impl Into<String>,
        kind: super::model::ToastActionKind,
        icon: Option<egui_material_icons::MaterialIcon>,
    ) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.action = Some(super::model::ToastAction {
                label: label.into(),
                kind,
                icon,
            });
        }
    }

    /// 用户取消/收起：不再渲染，但保留源数据直到任务真正结束。
    /// 进行中的暂停任务需要 resume，避免卡片消失后永远暂停。
    pub fn dismiss(&mut self, id: u64) {
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            if let Some(p) = n.pause_flag() {
                p.store(false, std::sync::atomic::Ordering::Relaxed);
            }
            n.dismissed = true;
        }
        if self.primary == Some(id) {
            self.primary = None;
        }
    }

    pub fn is_dismissed(&self, id: u64) -> bool {
        self.get(id).is_some_and(|n| n.dismissed)
    }

    pub fn get_cancel_flag(&self, id: u64) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.get(id).and_then(|n| n.cancel_flag())
    }

    // ── 错误队列（阻塞窗口）──

    pub fn report_error(&mut self, title: impl Into<String>, message: impl Into<String>) {
        let title = title.into();
        let message = message.into();
        // 同一错误连续上报（例如后台每帧重试）只保留一条。
        if self
            .errors
            .back()
            .is_some_and(|e| e.title == title && e.message == message)
        {
            return;
        }
        if self.errors.len() >= MAX_ERRORS {
            self.errors.pop_front();
        }
        self.errors.push_back(ErrorReport { title, message });
    }

    pub fn front_error(&self) -> Option<&ErrorReport> {
        self.errors.front()
    }

    pub fn pop_error(&mut self) {
        self.errors.pop_front();
    }

    // ── 帧内维护 ──

    /// 修正主槽指向。每帧由标题栏渲染调用一次。
    pub(super) fn tick(&mut self) {
        if self
            .primary
            .is_some_and(|id| !self.items.iter().any(|n| n.id == id && !n.dismissed))
        {
            self.primary = None;
        }
        if self.primary.is_none() {
            self.primary = self
                .items
                .iter()
                .rev()
                .find(|n| n.source.is_some() && !n.dismissed)
                .or_else(|| self.items.iter().rev().find(|n| !n.dismissed))
                .map(|n| n.id);
        }
    }
}
