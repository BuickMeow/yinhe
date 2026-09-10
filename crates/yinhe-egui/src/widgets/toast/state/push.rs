use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::kind::ToastKind;
use super::super::model::{HistoryEntry, Toast};
use super::Notifications;

#[allow(dead_code)]
impl Notifications {
    // ── 对外推送 API ──

    pub fn info(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Info, title, message, None)
    }

    pub fn success(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Success, title, message, None)
    }

    pub fn warning(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Warning, title, message, None)
    }

    pub fn error(&mut self, title: impl Into<String>, message: impl Into<String>) -> u64 {
        self.push(ToastKind::Error, title, message, None)
    }

    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
        _ttl: Option<Duration>,
    ) -> u64 {
        if !self.enabled {
            return 0;
        }
        let id = self.next_id;
        self.next_id += 1;
        let title = title.into();
        let message = message.into();
        // 普通成功/信息按完成档计时，警告/错误按可操作档计时（需用户留意）
        let dur = match kind {
            ToastKind::Info | ToastKind::Success => self.collapse_secs,
            ToastKind::Warning | ToastKind::Error => self.action_collapse_secs,
        };
        self.toasts.push(Toast {
            id,
            kind,
            title: title.clone(),
            message: message.clone(),
            created: Instant::now(),
            progress: None,
            progress_label: String::new(),
            cancel: None,
            leaving_since: None,
            source: None,
            collapse_at: self.collapse_deadline(dur),
            action: None,
            hovered: false,
            cancelling: false,
        });
        self.history.push(HistoryEntry {
            id,
            kind,
            title,
            message,
            created: Instant::now(),
            read: self.center_open,
            progress: None,
            progress_label: String::new(),
            source: None,
        });
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
        id
    }

    /// 历史条目换数据源（kind/source），无则新建；供 ensure 复用。
    pub(super) fn sync_history_source(
        &mut self,
        id: u64,
        kind: ToastKind,
        source: Arc<dyn super::super::model::ProgressSource>,
    ) {
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = kind;
            h.source = Some(source);
        } else {
            self.push_history(id, kind, source);
        }
    }

    /// 本次运行的历史同步（浮动卡复用固定槽位，历史一任务一条）：
    /// 映射到且条目还在 → 原地更新；否则用 next_id 新建并记录映射。
    /// 固定 id 都是 0x4C4C… 大数，next_id 从 1 递增，不会碰撞。
    pub(super) fn sync_live_history(
        &mut self,
        fixed_id: u64,
        kind: ToastKind,
        source: Arc<dyn super::super::model::ProgressSource>,
    ) {
        if let Some(&hist_id) = self.live_hist.get(&fixed_id)
            && self.history.iter().any(|h| h.id == hist_id)
        {
            self.sync_history_source(hist_id, kind, source);
            return;
        }
        let hist_id = self.next_id;
        self.next_id += 1;
        self.push_history(hist_id, kind, source);
        self.live_hist.insert(fixed_id, hist_id);
    }

    pub(super) fn push_history(
        &mut self,
        id: u64,
        kind: ToastKind,
        source: Arc<dyn super::super::model::ProgressSource>,
    ) {
        self.history.push(HistoryEntry {
            id,
            kind,
            title: source.title(),
            message: String::new(),
            created: Instant::now(),
            read: self.center_open,
            progress: None,
            progress_label: String::new(),
            source: Some(source),
        });
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
    }

    pub fn unread_count(&self) -> usize {
        self.history.iter().filter(|e| !e.read).count()
    }

    pub fn has_unread(&self) -> bool {
        self.history.iter().any(|e| !e.read)
    }

    pub fn mark_all_read(&mut self) {
        for e in &mut self.history {
            e.read = true;
        }
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}
