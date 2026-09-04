use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_material_icons::icons::*;

// ── Toast 种类 ──
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

impl ToastKind {
    fn icon(self) -> egui_material_icons::MaterialIcon {
        match self {
            Self::Info => ICON_INFO,
            Self::Success => ICON_CHECK_CIRCLE,
            Self::Warning => ICON_WARNING,
            Self::Error => ICON_ERROR,
        }
    }

    fn color(self) -> egui::Color32 {
        match self {
            Self::Info => crate::theme::text_secondary(),
            Self::Success => crate::theme::accent_active(),
            Self::Warning => crate::theme::warning_gold(),
            Self::Error => crate::theme::danger_text(),
        }
    }
}

// ── 单条 Toast（常驻，需手动关闭）──
struct Toast {
    id: u64,
    kind: ToastKind,
    title: String,
    message: String,
    created: Instant,
    /// 进度：None=普通通知，Some(0.0..1.0)=进度条
    progress: Option<f32>,
    progress_label: String,
    cancel: Option<Arc<AtomicBool>>,
    leaving_since: Option<Instant>,
}

// ── 历史记录（持久）──
#[derive(Clone)]
#[allow(dead_code)]
struct HistoryEntry {
    id: u64,
    kind: ToastKind,
    title: String,
    message: String,
    created: Instant,
    read: bool,
}

// ── 统一通知中心 ──
pub struct Notifications {
    next_id: u64,
    toasts: Vec<Toast>,
    history: Vec<HistoryEntry>,
    /// 通知列表是否展开（由 mode_bar 铃铛切换）。
    pub show_center: bool,
    max_history: usize,
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
            show_center: false,
            max_history: 100,
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
            // 同步历史
            if let Some(h) = self.history.iter_mut().find(|h| h.id == key_id) {
                h.title = title;
                h.message = message;
                h.kind = kind;
            }
            return key_id;
        }
        // 不存在则新建，沿用指定 id
        if key_id >= self.next_id {
            self.next_id = key_id + 1;
        }
        let id = key_id;
        self.toasts.push(Toast {
            id,
            kind,
            title: title.clone(),
            message: message.clone(),
            created: Instant::now(),
            progress: Some(fraction.clamp(0.0, 1.0)),
            progress_label: label,
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
        });
        if self.history.len() > self.max_history {
            let excess = self.history.len() - self.max_history;
            self.history.drain(0..excess);
        }
        id
    }

    pub fn update_progress(&mut self, id: u64, fraction: f32, label: impl Into<String>) {
        let label = label.into();
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.progress = Some(fraction.clamp(0.0, 1.0));
            t.progress_label = label;
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
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = kind;
            t.title = title.clone();
            t.message = message.clone();
            t.progress = Some(1.0);
            t.progress_label = "已完成".to_string();
            t.cancel = None;
            t.leaving_since = None;
        }
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = kind;
            h.title = title.clone();
            h.message = message.clone();
        }
        if self.toasts.iter().find(|t| t.id == id).is_none() {
            // 若进度 toast 已被手动关闭，改为普通 push
            self.push(kind, title, message, None);
        }
    }

    pub fn fail_progress(&mut self, id: u64, title: impl Into<String>, message: impl Into<String>) {
        let title = title.into();
        let message = message.into();
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.kind = ToastKind::Error;
            t.title = title.clone();
            t.message = message.clone();
            t.progress_label = "失败".to_string();
            t.cancel = None;
            t.leaving_since = None;
        }
        if let Some(h) = self.history.iter_mut().find(|h| h.id == id) {
            h.kind = ToastKind::Error;
            h.title = title.clone();
            h.message = message.clone();
        }
        if self.toasts.iter().find(|t| t.id == id).is_none() {
            self.push(ToastKind::Error, title, message, None);
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
        // history 展开时也需动画
        if self.show_center && !self.history.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    // ── Toast 浮空渲染：右下角 → 右上角堆叠，浮于内容之上 ──
    pub fn show_toasts(&mut self, ctx: &egui::Context) {
        self.tick(ctx);
        if self.show_center {
            return;
        }
        if self.toasts.is_empty() {
            return;
        }
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 12.0;

        egui::Area::new(egui::Id::new("yinhe_toasts"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-RIGHT_PAD, -BOTTOM_PAD),
            )
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .show(ctx, |ui| {
                let max_h = (ctx.viewport_rect().height() - BOTTOM_PAD - 24.0).max(120.0);
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([true, true])
                    .stick_to_bottom(true)
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let mut to_dismiss: Vec<u64> = Vec::new();
                            let mut first = true;
                            for (idx, toast) in self.toasts.iter().enumerate() {
                                if !first {
                                    // 缝隙也设为可悬停，避免滚轮穿透到 PR/AR
                                    ui.allocate_response(
                                        egui::vec2(CARD_W, GAP),
                                        egui::Sense::hover(),
                                    );
                                }
                                first = false;
                                let is_leaving = toast.leaving_since.is_some();
                                let stagger =
                                    (self.toasts.len().saturating_sub(1).saturating_sub(idx))
                                        as f32
                                        * 0.04;
                                let (x_off, alpha) = Self::fly_anim(toast, stagger);
                                if is_leaving && alpha < 0.02 {
                                    continue;
                                }
                                let card_alpha = alpha;
                                let resp = Self::toast_card(ui, toast, CARD_W, x_off, card_alpha);
                                if resp.0 {
                                    to_dismiss.push(toast.id);
                                }
                                if resp.1 {
                                    if let Some(c) = &toast.cancel {
                                        c.store(true, std::sync::atomic::Ordering::Relaxed);
                                    }
                                    to_dismiss.push(toast.id);
                                }
                            }
                            if !to_dismiss.is_empty() {
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new("yinhe_toasts_dismiss"),
                                        to_dismiss,
                                    );
                                });
                            }
                        });
                    });
            });
        let pending: Option<Vec<u64>> =
            ctx.data(|d| d.get_temp(egui::Id::new("yinhe_toasts_dismiss")));
        if let Some(ids) = pending {
            ctx.data_mut(|d| d.remove::<Vec<u64>>(egui::Id::new("yinhe_toasts_dismiss")));
            for id in ids {
                self.dismiss_toast(id);
            }
        }
    }

    fn fly_anim(toast: &Toast, stagger: f32) -> (f32, f32) {
        let now = Instant::now();
        if let Some(since) = toast.leaving_since {
            let t = (now.duration_since(since).as_secs_f32() / 0.28).clamp(0.0, 1.0);
            // ease_in_cubic：反向向右飞出，先慢后快
            let e = t * t * t;
            let x = e * 80.0;
            let a = 1.0 - e;
            return (x, a);
        }
        let elapsed = now.duration_since(toast.created).as_secs_f32() - stagger;
        if elapsed < 0.0 {
            return (80.0, 0.0);
        }
        let t = (elapsed / 0.38).clamp(0.0, 1.0);
        // ease_out_cubic：先快后慢，从右向左
        let e = 1.0 - (1.0 - t).powi(3);
        let x = (1.0 - e) * 80.0;
        let a = e;
        (x, a)
    }

    /// 返回 (dismiss, cancel)
    fn toast_card(
        ui: &mut egui::Ui,
        toast: &Toast,
        width: f32,
        x_offset: f32,
        alpha: f32,
    ) -> (bool, bool) {
        let mut dismiss = false;
        let mut cancel = false;
        let bg_base = crate::theme::control_bg();
        let stroke_base = crate::theme::line_fg().gamma_multiply(0.35);
        let bg = egui::Color32::from_rgba_unmultiplied(
            bg_base.r(),
            bg_base.g(),
            bg_base.b(),
            (bg_base.a() as f32 * alpha) as u8,
        );
        let stroke_col = egui::Color32::from_rgba_unmultiplied(
            stroke_base.r(),
            stroke_base.g(),
            stroke_base.b(),
            (stroke_base.a() as f32 * alpha) as u8,
        );
        let frame = egui::Frame {
            fill: bg,
            stroke: egui::Stroke::new(1.0, stroke_col),
            corner_radius: egui::CornerRadius::same(8),
            shadow: egui::Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: egui::Color32::from_black_alpha((60.0 * alpha) as u8),
            },
            inner_margin: egui::Margin::symmetric(10, 10),
            ..Default::default()
        };
        let mut card_alpha = alpha;
        // 右向左飞入：x 80→0，扩大裁剪使屏外部分可见
        ui.scope(|ui| {
            let mut clip = ui.available_rect_before_wrap();
            clip.max.x += 120.0;
            clip.min.x -= 20.0;
            ui.set_clip_rect(clip);
            ui.allocate_ui_with_layout(
                egui::vec2(width, 0.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    if x_offset > 0.5 {
                        ui.add_space(x_offset);
                    }
                    // 禁用时略降透明度
                    if toast.leaving_since.is_some() {
                        card_alpha = alpha;
                    }
                    frame.show(ui, |ui| {
                        ui.set_max_width(width - 20.0);
                        ui.set_min_width(width - 20.0);
                        // 标题行
                        ui.horizontal(|ui| {
                            let icon = toast.kind.icon();
                            let icon_col = mul_alpha(toast.kind.color(), card_alpha);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(icon.codepoint)
                                        .family(icon.font_family())
                                        .size(crate::theme::ICON_FONT)
                                        .color(icon_col),
                                )
                                .selectable(false),
                            );
                            ui.add_space(6.0);
                            ui.vertical(|ui| {
                                ui.set_max_width(width - 90.0);
                                if !toast.title.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&toast.title)
                                                .size(crate::theme::SMALL_FONT)
                                                .strong()
                                                .color(mul_alpha(
                                                    crate::theme::text_primary(),
                                                    card_alpha,
                                                )),
                                        )
                                        .selectable(false)
                                        .wrap(),
                                    );
                                }
                                if !toast.message.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(&toast.message)
                                                .size(crate::theme::SMALL_FONT)
                                                .color(mul_alpha(
                                                    crate::theme::text_secondary(),
                                                    card_alpha,
                                                )),
                                        )
                                        .selectable(false)
                                        .wrap(),
                                    );
                                }
                                if let Some(p) = toast.progress {
                                    if !toast.progress_label.is_empty() {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(&toast.progress_label)
                                                    .size(crate::theme::SMALL_LABEL_FONT)
                                                    .color(mul_alpha(
                                                        crate::theme::text_muted(),
                                                        card_alpha,
                                                    )),
                                            )
                                            .selectable(false)
                                            .wrap(),
                                        );
                                    }
                                    let _ = p;
                                }
                            });
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let close_icon = ICON_CLOSE;
                                    let resp = crate::widgets::hover::hover_button(
                                        ui,
                                        close_icon.codepoint,
                                        egui::FontId::new(
                                            crate::theme::ICON_FONT_SM,
                                            close_icon.font_family(),
                                        ),
                                        mul_alpha(crate::theme::text_muted(), card_alpha),
                                        false,
                                    );
                                    if resp.clicked() {
                                        dismiss = true;
                                    }
                                    // 进度态额外提供取消
                                    if toast.progress.is_some() && toast.cancel.is_some() {
                                        ui.add_space(6.0);
                                        let resp2 = ui.add(
                                            egui::Label::new(
                                                egui::RichText::new("取消")
                                                    .size(crate::theme::SMALL_FONT)
                                                    .color(mul_alpha(
                                                        crate::theme::text_muted(),
                                                        card_alpha,
                                                    )),
                                            )
                                            .sense(egui::Sense::click())
                                            .selectable(false),
                                        );
                                        if resp2.clicked() {
                                            cancel = true;
                                        }
                                        if resp2.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                    }
                                },
                            );
                        });
                        // 进度条：保持占位高度，避免完成态跳变
                        if let Some(p) = toast.progress {
                            ui.add_space(6.0);
                            let bar_w = width - 20.0;
                            let bar_h = 4.0;
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(bar_w, bar_h),
                                egui::Sense::hover(),
                            );
                            let bg =
                                mul_alpha(crate::theme::line_fg().gamma_multiply(0.25), card_alpha);
                            ui.painter().rect_filled(rect, 2.0, bg);
                            let fg_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(rect.width() * p.clamp(0.0, 1.0), rect.height()),
                            );
                            ui.painter().rect_filled(
                                fg_rect,
                                2.0,
                                mul_alpha(toast.kind.color().gamma_multiply(0.85), card_alpha),
                            );
                        } else {
                            ui.add_space(10.0);
                        }
                    });
                },
            );
        });
        (dismiss, cancel)
    }

    // ── 通知展开：点铃铛后从右下→右上排列所有历史 toast，同样浮空，无单独弹窗 ──
    pub fn show_center(&mut self, ctx: &egui::Context) {
        if !self.show_center {
            return;
        }
        if self.history.is_empty() {
            return;
        }
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 12.0;

        let viewport_h = ctx.viewport_rect().height();
        let max_h = (viewport_h - BOTTOM_PAD - 24.0).max(120.0);

        egui::Area::new(egui::Id::new("yinhe_notification_center"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-RIGHT_PAD, -BOTTOM_PAD),
            )
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .scroll_bar_visibility(
                        egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded,
                    )
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let mut first = true;
                            // bottom_up + rev：最新在最底，贴近 mode_bar，单条时也在底部
                            for entry in self.history.iter().rev() {
                                if !first {
                                    ui.allocate_response(
                                        egui::vec2(CARD_W, GAP),
                                        egui::Sense::hover(),
                                    );
                                }
                                first = false;
                                Self::history_card(ui, entry, CARD_W);
                            }
                        });
                    });
            });
        // 展开态持续重绘以支持滚动惯性
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    /// 历史卡片：只读，无删除
    fn history_card(ui: &mut egui::Ui, entry: &HistoryEntry, width: f32) {
        let frame = egui::Frame {
            fill: crate::theme::control_bg(),
            stroke: egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35)),
            corner_radius: egui::CornerRadius::same(8),
            shadow: egui::Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: egui::Color32::from_black_alpha(60),
            },
            inner_margin: egui::Margin::symmetric(10, 10),
            ..Default::default()
        };
        frame.show(ui, |ui| {
            ui.set_max_width(width - 20.0);
            ui.set_min_width(width - 20.0);
            ui.horizontal(|ui| {
                let icon = entry.kind.icon();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(icon.codepoint)
                            .family(icon.font_family())
                            .size(crate::theme::ICON_FONT)
                            .color(entry.kind.color()),
                    )
                    .selectable(false),
                );
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.set_max_width(width - 90.0);
                    if !entry.title.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&entry.title)
                                    .size(crate::theme::SMALL_FONT)
                                    .strong()
                                    .color(crate::theme::text_primary()),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                    if !entry.message.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&entry.message)
                                    .size(crate::theme::SMALL_FONT)
                                    .color(crate::theme::text_secondary()),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                });
            });
        });
    }
}

fn mul_alpha(c: egui::Color32, a: f32) -> egui::Color32 {
    let a = a.clamp(0.0, 1.0);
    egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a) as u8)
}
