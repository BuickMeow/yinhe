use std::collections::HashMap;
use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use eframe::egui;

use super::kind::ToastKind;
use super::model::{HistoryEntry, Toast};

// ── 统一通知中心 ──
pub struct Notifications {
    next_id: u64,
    toasts: Vec<Toast>,
    history: Vec<HistoryEntry>,
    /// 通知列表是否展开（由 mode_bar 铃铛切换）。
    pub center_open: bool,
    max_history: usize,
    center_opened_at: Option<Instant>,
    center_closed_at: Option<Instant>,
    prev_center_open: bool,
}

pub const LOADING_PROGRESS_ID: u64 = 0x4C4F4144; // "LOAD"
pub const SAVE_PROGRESS_ID: u64 = 0x53415645; // "SAVE"
pub const EXPORT_PROGRESS_ID: u64 = 0x45585054; // "EXPT"
pub const RESCALE_PROGRESS_ID: u64 = 0x5253434C; // "RSCL"

impl Default for Notifications {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl Notifications {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            toasts: Vec::new(),
            history: Vec::new(),
            center_open: false,
            max_history: 100,
            center_opened_at: None,
            center_closed_at: None,
            prev_center_open: false,
        }
    }

    // ── 对外推送 API ──

    pub fn info(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.push(ToastKind::Info, title, message, None);
    }

    pub fn success(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.push(ToastKind::Success, title, message, None);
    }

    pub fn warning(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.push(ToastKind::Warning, title, message, None);
    }

    pub fn error(&mut self, title: impl Into<String>, message: impl Into<String>) {
        self.push(ToastKind::Error, title, message, None);
    }

    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
        _ttl: Option<Duration>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let title = title.into();
        let message = message.into();
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
        });
        self.history.push(HistoryEntry {
            id,
            kind,
            title,
            message,
            created: Instant::now(),
            read: false,
            progress: None,
            progress_label: String::new(),
        });
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
        id
    }

    /// 创建或更新进度 toast。同一 key（如 "loading"）复用同一 id。
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_progress(
        &mut self,
        key_id: u64,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
        fraction: f32,
        label: impl Into<String>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> u64 {
        let title = title.into();
        let message = message.into();
        let label = label.into();
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == key_id) {
            t.kind = kind;
            t.title = title.clone();
            t.message = message.clone();
            t.progress = Some(fraction.clamp(0.0, 1.0));
            t.progress_label = label.clone();
            t.cancel = cancel;
            t.leaving_since = None;
            if let Some(h) = self.history.iter_mut().find(|h| h.id == key_id) {
                h.title = title.clone();
                h.message = message.clone();
                h.kind = kind;
                h.progress = Some(fraction.clamp(0.0, 1.0));
                h.progress_label = label.clone();
            } else {
                self.history.push(HistoryEntry {
                    id: key_id,
                    kind,
                    title: title.clone(),
                    message: message.clone(),
                    created: Instant::now(),
                    read: false,
                    progress: Some(fraction.clamp(0.0, 1.0)),
                    progress_label: label.clone(),
                });
                if self.history.len() > self.max_history {
                    let excess = self.history.len() - self.max_history;
                    self.history.drain(0..excess);
                }
            }
            return key_id;
        }
        if key_id >= self.next_id {
            self.next_id = key_id + 1;
        }
        let id = key_id;
        let p = fraction.clamp(0.0, 1.0);
        self.toasts.push(Toast {
            id,
            kind,
            title: title.clone(),
            message: message.clone(),
            created: Instant::now(),
            progress: Some(p),
            progress_label: label.clone(),
            cancel,
            leaving_since: None,
        });
        self.history.push(HistoryEntry {
            id,
            kind,
            title,
            message,
            created: Instant::now(),
            read: false,
            progress: Some(p),
            progress_label: label,
        });
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
        id
    }

    pub fn update_progress(&mut self, id: u64, fraction: f32, label: impl Into<String>) {
        let label = label.into();
        let p = fraction.clamp(0.0, 1.0);
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.progress = Some(p);
            t.progress_label = label.clone();
        }
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.progress = Some(p);
            h.progress_label = label;
        }
    }

    /// 进度完成：保留同一张 toast，原地切换为完成态（进度条满格，避免高度跳变）
    pub fn complete_progress(
        &mut self,
        id: u64,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let title = title.into();
        let message = message.into();
        let mut toast_found = false;
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = kind;
            t.title = title.clone();
            t.message = message.clone();
            t.progress = Some(1.0);
            t.progress_label = "已完成".to_string();
            t.cancel = None;
            t.leaving_since = None;
            toast_found = true;
        }
        let mut hist_found = false;
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = kind;
            h.title = title.clone();
            h.message = message.clone();
            h.progress = Some(1.0);
            h.progress_label = "已完成".to_string();
            hist_found = true;
        }
        if !toast_found && !hist_found {
            self.push(kind, title, message, None);
        } else if toast_found && !hist_found {
            self.history.push(HistoryEntry {
                id,
                kind,
                title: title.clone(),
                message: message.clone(),
                created: Instant::now(),
                read: false,
                progress: Some(1.0),
                progress_label: "已完成".to_string(),
            });
            if self.history.len() > self.max_history {
                let excess = self.history.len() - self.max_history;
                self.history.drain(0..excess);
            }
        } else if !toast_found && hist_found {
            // toast 已被手动关闭但历史仍在，无需额外处理；若需要可重新弹出 toast
            // 保持历史已更新即可
        }
    }

    pub fn fail_progress(&mut self, id: u64, title: impl Into<String>, message: impl Into<String>) {
        let title = title.into();
        let message = message.into();
        let mut toast_found = false;
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = ToastKind::Error;
            t.title = title.clone();
            t.message = message.clone();
            t.progress_label = "失败".to_string();
            t.cancel = None;
            t.leaving_since = None;
            toast_found = true;
        }
        let mut hist_found = false;
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = ToastKind::Error;
            h.title = title.clone();
            h.message = message.clone();
            h.progress_label = "失败".to_string();
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
                read: false,
                progress: None,
                progress_label: "失败".to_string(),
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
            .and_then(|t| t.cancel.clone())
    }

    pub fn remove_progress(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
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

    /// 标记离开动画，300ms 后真正移除
    #[allow(clippy::collapsible_if)]
    pub fn dismiss_toast(&mut self, id: u64) {
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id)
            && t.leaving_since.is_none()
        {
            t.leaving_since = Some(Instant::now());
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.center_open != self.prev_center_open {
            if self.center_open {
                self.center_opened_at = Some(now);
                self.center_closed_at = None;
            } else {
                self.center_opened_at = None;
                self.center_closed_at = Some(now);
            }
            self.prev_center_open = self.center_open;
            ctx.request_repaint();
        }
        let mut needs_repaint = false;
        // 清理已完成离开动画的 toast
        let before = self.toasts.len();
        self.toasts.retain(|t| {
            if let Some(since) = t.leaving_since {
                now.duration_since(since) < Duration::from_millis(320)
            } else {
                true
            }
        });
        if self.toasts.len() != before {
            needs_repaint = true;
        }
        let has_anim = self.toasts.iter().any(|t| {
            t.leaving_since.is_some()
                || now.duration_since(t.created) < Duration::from_millis(400)
                || t.progress.is_some_and(|p| p < 0.999)
        });
        if has_anim || needs_repaint {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if !self.toasts.is_empty() {
            // 常驻 toast 无动画时仍需偶尔重绘以响应 hover
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // history 展开时也需动画（兜底：toast 回退也需）以及重排动画
        if self.center_open && (!self.history.is_empty() || !self.toasts.is_empty()) {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        // 列表关闭退场动画进行中也需重绘
        if let Some(closed) = self.center_closed_at
            && now.duration_since(closed) < Duration::from_millis(400)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    // ── Toast 浮空渲染：右下角 → 右上角堆叠，浮于内容之上 ──
    // 彻底重构：每个 toast 独立 Area，避免父 Area+ScrollArea 宽度异常导致右侧溢出。
    // 打开通知列表时，已存在的 toast 不消失，而是通过非线性 y 插值重排至历史位置；其余历史项飞入。
    pub fn show_toasts(&mut self, ctx: &egui::Context) {
        self.tick(ctx);
        if self.toasts.is_empty() {
            return;
        }
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 32.0;
        const EST_H: f32 = 110.0;

        let viewport = ctx.viewport_rect();
        let max_h = (viewport.height() - BOTTOM_PAD - 24.0).max(120.0);

        // 预计算目标 y：toast 堆叠 vs 历史堆叠
        let mut toast_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let mut cum: f32 = 0.0;
            for toast in self.toasts.iter().rev() {
                toast_y_map.insert(toast.id, BOTTOM_PAD + cum);
                cum += EST_H + GAP;
            }
        }
        let mut history_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let mut cum: f32 = 0.0;
            for entry in self.history.iter().rev() {
                history_y_map.insert(entry.id, BOTTOM_PAD + cum);
                cum += EST_H + GAP;
            }
        }

        let mut to_dismiss: Vec<u64> = Vec::new();
        for toast in self.toasts.iter().rev() {
            let target_y = if self.center_open {
                // 重排至历史中的位置
                history_y_map.get(&toast.id).copied().unwrap_or(BOTTOM_PAD)
            } else {
                toast_y_map.get(&toast.id).copied().unwrap_or(BOTTOM_PAD)
            };
            if target_y > max_h + EST_H {
                continue;
            }
            // 非线性 y 插值：已有通知平滑重排
            let y_off =
                ctx.animate_value_with_time(egui::Id::new(("notif_y", toast.id)), target_y, 0.35);
            let is_leaving = toast.leaving_since.is_some();
            // 打开列表时已存在的 toast 不重新飞入，仅重排；离开时仍飞出
            let x_off = if self.center_open && !is_leaving {
                0.0
            } else {
                super::anim::fly_anim(toast)
            };
            let mut inner_dismiss = false;
            let mut inner_cancel = false;
            let area_id = egui::Id::new(("yinhe_notif", toast.id));
            egui::Area::new(area_id)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-RIGHT_PAD + x_off, -y_off),
                )
                .order(egui::Order::Tooltip)
                .movable(false)
                .interactable(true)
                .show(ctx, |ui| {
                    let (d, c) = super::card::toast_card(ui, toast, CARD_W, 0.0, 1.0);
                    inner_dismiss = d;
                    inner_cancel = c;
                });
            if inner_dismiss {
                to_dismiss.push(toast.id);
            }
            if inner_cancel {
                if let Some(c) = &toast.cancel {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                to_dismiss.push(toast.id);
            }
        }
        if !to_dismiss.is_empty() {
            // 去重，避免同一 id 既 dismiss 又 cancel 重复
            to_dismiss.sort_unstable();
            to_dismiss.dedup();
            for id in to_dismiss {
                self.dismiss_toast(id);
            }
        }
    }

    pub fn show_center(&mut self, ctx: &egui::Context) {
        // 关闭时：历史独有项做退场飞出（入场的严格反向），播完再停
        let closing = !self.center_open;
        if closing {
            let Some(closed_at) = self.center_closed_at else {
                return;
            };
            if Instant::now().duration_since(closed_at) > Duration::from_millis(350) {
                return;
            }
            if self.history.is_empty() {
                return;
            }
        } else if self.history.is_empty() {
            return;
        }
        tracing::debug!(
            "show_center history={} center_open={}",
            self.history.len(),
            self.center_open
        );
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 32.0;
        const EST_H: f32 = 110.0;

        let viewport = ctx.viewport_rect();
        let max_h = (viewport.height() - BOTTOM_PAD - 24.0).max(120.0);

        // 预计算历史目标 y（最新在底部）
        let mut history_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let mut cum: f32 = 0.0;
            for entry in self.history.iter().rev() {
                history_y_map.insert(entry.id, BOTTOM_PAD + cum);
                cum += EST_H + GAP;
            }
        }

        // 仅渲染历史中不在当前 toast 的那些（已在屏幕的由 show_toasts 负责重排）
        for entry in self.history.iter().rev() {
            if self.toasts.iter().any(|t| t.id == entry.id) {
                continue;
            }
            let target_y = history_y_map.get(&entry.id).copied().unwrap_or(BOTTOM_PAD);
            if target_y > max_h + EST_H {
                continue;
            }
            let y_off =
                ctx.animate_value_with_time(egui::Id::new(("notif_y", entry.id)), target_y, 0.35);
            // 打开：无停顿直接从右侧飞入；关闭：入场的严格反向飞出
            let x_off = if closing {
                let closed_at = self.center_closed_at.unwrap_or_else(Instant::now);
                super::anim::exit_x(Instant::now().duration_since(closed_at).as_secs_f32())
            } else if let Some(opened_at) = self.center_opened_at {
                super::anim::enter_x(Instant::now().duration_since(opened_at).as_secs_f32())
            } else {
                0.0
            };
            let area_id = egui::Id::new(("yinhe_notif", entry.id));
            egui::Area::new(area_id)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-RIGHT_PAD + x_off, -y_off),
                )
                .order(egui::Order::Tooltip)
                .movable(false)
                .interactable(true)
                .show(ctx, |ui| {
                    super::card::history_card(ui, entry, CARD_W);
                });
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
