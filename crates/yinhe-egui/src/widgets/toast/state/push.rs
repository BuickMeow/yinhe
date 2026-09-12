use std::sync::Arc;
use std::time::Instant;

use super::super::kind::ToastKind;
use super::super::model::{Notification, ProgressSource};
use super::Notifications;

impl Notifications {
    // ── 对外推送 API ──

    pub fn success(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Success, title, message)
    }

    pub fn error(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Error, title, message)
    }

    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        if !self.enabled {
            return 0;
        }
        // 普通成功/信息按完成档计时，警告/错误按可操作档计时（需用户留意）
        let dur = match kind {
            ToastKind::Info | ToastKind::Success => self.collapse_secs,
            ToastKind::Warning | ToastKind::Error => self.action_collapse_secs,
        };
        let deadline = self.collapse_deadline(dur);
        let id = self.alloc_id();
        self.insert(id, kind, title.into(), message.into(), None, deadline)
    }

    /// 统一构造一条通知（浮动卡入场，自动进列表）。返回 id。
    pub(super) fn insert(
        &mut self,
        id: u64,
        kind: ToastKind,
        title: String,
        message: String,
        source: Option<Arc<dyn ProgressSource>>,
        collapse_at: Option<Instant>,
    ) -> u64 {
        self.items.push(Notification {
            id,
            kind,
            title,
            message,
            created: Instant::now(),
            progress: None,
            progress_label: String::new(),
            cancel: source.as_ref().and_then(|s| s.cancel()),
            source,
            collapse_at,
            action: None,
            hovered: false,
            cancelling: false,
            on_screen: true,
            leaving_since: None,
            read: self.center_open,
        });
        self.prune_overflow();
        id
    }

    pub fn has_unread(&self) -> bool {
        self.items.iter().any(|n| !n.read)
    }

    pub fn mark_all_read(&mut self) {
        for n in &mut self.items {
            n.read = true;
        }
    }
}
