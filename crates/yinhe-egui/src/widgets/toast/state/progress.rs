use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::super::kind::ToastKind;
use super::super::model::{HistoryEntry, Toast};
use super::{EXPORT_PROGRESS_ID, Notifications};

#[allow(dead_code)]
impl Notifications {
    /// 确保进度卡存在：同一 key（如 "loading"）复用同一 id。
    /// 只建卡/换数据源（Arc 交换，无文案拷贝），进度文案渲染时 pull。
    /// 调用方每帧调也无妨，但文案不再每帧拷贝；用户点了 X（leaving）时不复活。
    pub fn ensure_progress(
        &mut self,
        key_id: u64,
        kind: ToastKind,
        source: Arc<dyn super::super::model::ProgressSource>,
    ) {
        if !self.enabled {
            return;
        }
        if self.is_leaving(key_id) {
            return;
        }
        // 用户收起中的任务：只更新历史条目，不重建浮动卡
        if self.collapsed.contains(&key_id) {
            self.sync_live_history(key_id, kind, source);
            return;
        }
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == key_id) {
            // 已完成态（source 已清空）的新任务：重新入场
            if t.source.is_none() {
                t.created = Instant::now();
                t.title = source.title();
            }
            t.kind = kind;
            t.source = Some(source.clone());
            // 进行中不计时，完成时才起算
            t.collapse_at = None;
            t.leaving_since = None;
            t.cancelling = false;
            self.sync_live_history(key_id, kind, source);
            return;
        }
        let now = Instant::now();
        self.toasts.push(Toast {
            id: key_id,
            kind,
            title: source.title(),
            message: String::new(),
            created: now,
            progress: None,
            progress_label: String::new(),
            cancel: source.cancel(),
            leaving_since: None,
            source: Some(source.clone()),
            collapse_at: None,
            action: None,
            hovered: false,
            cancelling: false,
        });
        self.sync_live_history(key_id, kind, source);
    }

    /// 清掉本次运行的 live 历史条目并删映射（封存的不碰）；未映射回退按 id 清，做兼容。
    pub fn prune_history(&mut self, id: u64) {
        if let Some(hist_id) = self.live_hist.remove(&id) {
            self.history.retain(|h| h.id != hist_id);
        } else {
            self.history.retain(|h| h.id != id);
        }
        self.collapsed.remove(&id);
    }

    /// 显式覆盖进度（完成 label 等）：快照优先，清空 source 接管。
    /// 调用者均为固定 id（poll.rs 加载完成后的耗时覆盖，LOADING_PROGRESS_ID）。
    /// 历史走 live 映射，未命中回退按 id；complete 后映射已封存时按标题+正文找最近封存，保证一致。
    pub fn update_progress(&mut self, id: u64, fraction: f32, label: impl Into<String>) {
        if !self.enabled {
            return;
        }
        let label = label.into();
        let p = fraction.clamp(0.0, 1.0);
        let mut sealed_key: Option<(String, String)> = None;
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.progress = Some(p);
            t.progress_label = label.clone();
            t.source = None;
            sealed_key = Some((t.title.clone(), t.message.clone()));
        }
        if let Some(&hist_id) = self.live_hist.get(&id)
            && let Some(h) = self.history.iter_mut().find(|h| h.id == hist_id)
        {
            h.progress = Some(p);
            h.progress_label = label;
            h.source = None;
            return;
        }
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.progress = Some(p);
            h.progress_label = label;
            h.source = None;
            return;
        }
        // complete 刚封存（映射已移除）：找同标题+正文的最近一条封存同步 label
        if let Some((tt, tm)) = sealed_key
            && let Some(h) = self
                .history
                .iter_mut()
                .rev()
                .find(|h| h.title == tt && h.message == tm)
        {
            h.progress = Some(p);
            h.progress_label = label;
            h.source = None;
        }
    }

    /// 进度完成：保留同一张 toast，原地切换为完成态（进度条满格，避免高度跳变）。
    /// 完成即按档位起算自动收起（导出走可操作档，其余走完成档）。
    /// 返回实际作用到的 toast id（回退 push 时为新 id，供 set_action 用）。
    pub fn complete_progress(
        &mut self,
        id: u64,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        if !self.enabled {
            return id;
        }
        let title = title.into();
        let message = message.into();
        // 导出完成卡带操作按钮，留足操作时间
        let dur = if id == EXPORT_PROGRESS_ID {
            self.action_collapse_secs
        } else {
            self.collapse_secs
        };
        let deadline = self.collapse_deadline(dur);
        self.collapsed.remove(&id);
        let mut toast_found = false;
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = kind;
            t.title = title.clone();
            t.message = message.clone();
            t.progress = Some(1.0);
            t.progress_label = "已完成".to_string();
            t.cancel = None;
            t.leaving_since = None;
            t.source = None;
            t.collapse_at = deadline;
            t.cancelling = false;
            toast_found = true;
        }
        let mut hist_found = false;
        // 历史走 live 映射：命中则更新该条目并移除映射=封存；未命中回退按 id 找
        if let Some(hist_id) = self.live_hist.remove(&id) {
            if let Some(h) = self.history.iter_mut().find(|h| h.id == hist_id) {
                h.kind = kind;
                h.title = title.clone();
                h.message = message.clone();
                h.progress = Some(1.0);
                h.progress_label = "已完成".to_string();
                h.source = None;
                hist_found = true;
            } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
                h.kind = kind;
                h.title = title.clone();
                h.message = message.clone();
                h.progress = Some(1.0);
                h.progress_label = "已完成".to_string();
                h.source = None;
                hist_found = true;
            }
        } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = kind;
            h.title = title.clone();
            h.message = message.clone();
            h.progress = Some(1.0);
            h.progress_label = "已完成".to_string();
            h.source = None;
            hist_found = true;
        }
        if !toast_found && !hist_found {
            // 回退为普通 push；push 内部已按 kind 定档计时
            return self.push(kind, title, message, None);
        } else if toast_found && !hist_found {
            self.history.push(HistoryEntry {
                id,
                kind,
                title: title.clone(),
                message: message.clone(),
                created: Instant::now(),
                read: self.center_open,
                progress: Some(1.0),
                progress_label: "已完成".to_string(),
                source: None,
            });
            if self.history.len() > self.max_history {
                let excess = self.history.len() - self.max_history;
                self.history.drain(0..excess);
            }
        } else if !toast_found && hist_found {
            // toast 已被手动关闭但历史仍在，无需额外处理；若需要可重新弹出 toast
            // 保持历史已更新即可
        }
        id
    }

    pub fn fail_progress(&mut self, id: u64, title: impl Into<String>, message: impl Into<String>) {
        if !self.enabled {
            return;
        }
        let title = title.into();
        let message = message.into();
        // 失败需用户留意，按可操作档计时
        let deadline = self.collapse_deadline(self.action_collapse_secs);
        self.collapsed.remove(&id);
        let mut toast_found = false;
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = ToastKind::Error;
            t.title = title.clone();
            t.message = message.clone();
            t.progress_label = "失败".to_string();
            t.cancel = None;
            t.leaving_since = None;
            t.source = None;
            t.collapse_at = deadline;
            t.cancelling = false;
            toast_found = true;
        }
        let mut hist_found = false;
        // 历史走 live 映射：命中则封存，未命中回退按 id
        if let Some(hist_id) = self.live_hist.remove(&id) {
            if let Some(h) = self.history.iter_mut().find(|h| h.id == hist_id) {
                h.kind = ToastKind::Error;
                h.title = title.clone();
                h.message = message.clone();
                h.progress_label = "失败".to_string();
                h.source = None;
                hist_found = true;
            } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
                h.kind = ToastKind::Error;
                h.title = title.clone();
                h.message = message.clone();
                h.progress_label = "失败".to_string();
                h.source = None;
                hist_found = true;
            }
        } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = ToastKind::Error;
            h.title = title.clone();
            h.message = message.clone();
            h.progress_label = "失败".to_string();
            h.source = None;
            hist_found = true;
        }
        if !toast_found && !hist_found {
            self.push(ToastKind::Error, title, message, None);
        } else if toast_found && !hist_found {
            self.history.push(HistoryEntry {
                id,
                kind: ToastKind::Error,
                title: title.clone(),
                message: message.clone(),
                created: Instant::now(),
                read: self.center_open,
                progress: None,
                progress_label: "失败".to_string(),
                source: None,
            });
            if self.history.len() > self.max_history {
                let excess = self.history.len() - self.max_history;
                self.history.drain(0..excess);
            }
        }
    }

    pub fn has_progress(&self, id: u64) -> bool {
        self.toasts.iter().any(|t| t.id == id)
    }

    pub fn is_leaving(&self, id: u64) -> bool {
        self.toasts
            .iter()
            .find(|t| t.id == id)
            .is_some_and(|t| t.leaving_since.is_some())
    }

    pub fn get_cancel_flag(&self, id: u64) -> Option<Arc<AtomicBool>> {
        self.toasts
            .iter()
            .find(|t| t.id == id)
            .and_then(super::super::model::resolve_cancel_toast)
    }

    /// 给浮动卡挂操作按钮（如“打开文件夹”）。
    /// 有按钮即可操作，计时自动升为可操作档。
    pub fn set_action(
        &mut self,
        id: u64,
        label: impl Into<String>,
        kind: super::super::model::ToastActionKind,
    ) {
        self.set_action_with_icon(id, label, kind, None);
    }

    /// 带图标的操作按钮（已中止卡用文件夹图标，无文字，hover 显示 label）。
    pub fn set_action_with_icon(
        &mut self,
        id: u64,
        label: impl Into<String>,
        kind: super::super::model::ToastActionKind,
        icon: Option<egui_material_icons::MaterialIcon>,
    ) {
        if !self.enabled {
            return;
        }
        let deadline = self.collapse_deadline(self.action_collapse_secs);
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.action = Some(super::super::model::ToastAction {
                label: label.into(),
                kind,
                icon,
            });
            t.collapse_at = deadline;
        }
    }

    /// 任务中止：Warning 黄卡，进度为中断时刻快照，label“已中止”，可操作档计时。
    /// 三态：toast 在则原地更新+历史同步；toast 不在但历史在则历史更新+重建卡；
    /// 都不在则回退 push 普通卡。返回作用到的 id 供 set_action 用。
    pub fn abort_progress(
        &mut self,
        id: u64,
        title: impl Into<String>,
        message: impl Into<String>,
    ) -> u64 {
        if !self.enabled {
            return id;
        }
        let title = title.into();
        let message = message.into();
        let deadline = self.collapse_deadline(self.action_collapse_secs);
        self.collapsed.remove(&id);
        // 中断时刻进度快照：有 live source 读 fraction，无则保持原 progress
        let snapshot_fraction = |t: &Toast| -> Option<f32> {
            if let Some(s) = &t.source {
                Some(s.fraction().clamp(0.0, 1.0))
            } else {
                t.progress
            }
        };
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            let frac = snapshot_fraction(t);
            t.kind = ToastKind::Warning;
            t.title = title.clone();
            t.message = message.clone();
            t.progress = frac;
            t.progress_label = "已中止".to_string();
            t.cancel = None;
            t.cancelling = false;
            t.leaving_since = None;
            t.source = None;
            t.collapse_at = deadline;
            t.action = None;
            // 历史走 live 映射：命中则封存，未命中回退按 id
            let mut hist_done = false;
            if let Some(hist_id) = self.live_hist.remove(&id) {
                if let Some(h) = self.history.iter_mut().find(|h| h.id == hist_id) {
                    h.kind = ToastKind::Warning;
                    h.title = title.clone();
                    h.message = message.clone();
                    h.progress = frac;
                    h.progress_label = "已中止".to_string();
                    h.source = None;
                    hist_done = true;
                } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
                    h.kind = ToastKind::Warning;
                    h.title = title.clone();
                    h.message = message.clone();
                    h.progress = frac;
                    h.progress_label = "已中止".to_string();
                    h.source = None;
                    hist_done = true;
                }
            } else if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
                h.kind = ToastKind::Warning;
                h.title = title.clone();
                h.message = message.clone();
                h.progress = frac;
                h.progress_label = "已中止".to_string();
                h.source = None;
                hist_done = true;
            }
            if !hist_done {
                self.history.push(HistoryEntry {
                    id,
                    kind: ToastKind::Warning,
                    title: title.clone(),
                    message: message.clone(),
                    created: Instant::now(),
                    read: self.center_open,
                    progress: frac,
                    progress_label: "已中止".to_string(),
                    source: None,
                });
                if self.history.len() > self.max_history {
                    let excess = self.history.len() - self.max_history;
                    self.history.drain(0..excess);
                }
            }
            return id;
        }
        // toast 不在：历史走 live 映射，命中则封存并重建同 id 浮动卡
        if let Some(hist_id) = self.live_hist.remove(&id)
            && let Some(h) = self.history.iter_mut().find(|h| h.id == hist_id)
        {
            let frac = h
                .source
                .as_ref()
                .map(|s| s.fraction().clamp(0.0, 1.0))
                .or(h.progress);
            h.kind = ToastKind::Warning;
            h.title = title.clone();
            h.message = message.clone();
            h.progress = frac;
            h.progress_label = "已中止".to_string();
            h.source = None;
            // 重建同 id 浮动卡（abort 态，deadline 重算）
            self.toasts.push(Toast {
                id,
                kind: ToastKind::Warning,
                title,
                message,
                created: Instant::now(),
                progress: frac,
                progress_label: "已中止".to_string(),
                cancel: None,
                leaving_since: None,
                source: None,
                collapse_at: deadline,
                action: None,
                hovered: false,
                cancelling: false,
            });
            return id;
        }
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            let frac = h
                .source
                .as_ref()
                .map(|s| s.fraction().clamp(0.0, 1.0))
                .or(h.progress);
            h.kind = ToastKind::Warning;
            h.title = title.clone();
            h.message = message.clone();
            h.progress = frac;
            h.progress_label = "已中止".to_string();
            h.source = None;
            // 重建同 id 浮动卡（abort 态，deadline 重算）
            self.toasts.push(Toast {
                id,
                kind: ToastKind::Warning,
                title,
                message,
                created: Instant::now(),
                progress: frac,
                progress_label: "已中止".to_string(),
                cancel: None,
                leaving_since: None,
                source: None,
                collapse_at: deadline,
                action: None,
                hovered: false,
                cancelling: false,
            });
            return id;
        }
        self.push(ToastKind::Warning, title, message, None)
    }
}
