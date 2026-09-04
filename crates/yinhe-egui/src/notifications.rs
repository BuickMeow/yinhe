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

    /// 自动消失时长：info/success 短，warning 中，error 长（仍自动消失，历史保留）。
    fn ttl(self) -> Duration {
        match self {
            Self::Info => Duration::from_millis(3000),
            Self::Success => Duration::from_millis(3000),
            Self::Warning => Duration::from_millis(5000),
            Self::Error => Duration::from_millis(6000),
        }
    }
}

// ── 单条 Toast（浮空卡片，自动消失）──
struct Toast {
    id: u64,
    kind: ToastKind,
    title: String,
    message: String,
    created: Instant,
    ttl: Duration,
}

// ── 历史记录（持久，通知中心列表）──
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
    /// 通知中心浮层是否打开（由 mode_bar 铃铛切换）。
    pub show_center: bool,
    max_history: usize,
}

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

    /// 自定义 TTL（传 None 则用 kind 默认）。
    pub fn push(
        &mut self,
        kind: ToastKind,
        title: impl Into<String>,
        message: impl Into<String>,
        ttl: Option<Duration>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let title = title.into();
        let message = message.into();
        let ttl = ttl.unwrap_or_else(|| kind.ttl());
        self.toasts.push(Toast {
            id,
            kind,
            title: title.clone(),
            message: message.clone(),
            created: Instant::now(),
            ttl,
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

    pub fn dismiss_toast(&mut self, id: u64) {
        self.toasts.retain(|t| t.id != id);
    }

    // ── 每帧维护：过期自动移除 + 按需重绘 ──
    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        let before = self.toasts.len();
        self.toasts
            .retain(|t| now.duration_since(t.created) < t.ttl);
        if self.toasts.len() != before {
            ctx.request_repaint();
        }
        // 进度条需丝滑：60fps 刷新，直到全部消失
        if !self.toasts.is_empty() {
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
        // 从右下角向上堆叠，浮空在所有 Panel 之上。
        // 用单独的 Area + top_down 布局，卡片宽度固定 360。
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        // mode_bar 高度约 28，shadow 向下 4px，再加 12 边距避免贴边
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
                ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.y = GAP;
                    let mut to_dismiss: Vec<u64> = Vec::new();
                    // top_down：旧在上新在下，整体贴底向上长，新 toast 永远贴着 mode_bar
                    for toast in self.toasts.iter() {
                        let resp = Self::toast_card(ui, toast, CARD_W);
                        if resp {
                            to_dismiss.push(toast.id);
                        }
                    }
                    if !to_dismiss.is_empty() {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new("yinhe_toasts_dismiss"), to_dismiss);
                        });
                    }
                });
            });
        // 取回待关闭列表并移除
        let pending: Option<Vec<u64>> =
            ctx.data(|d| d.get_temp(egui::Id::new("yinhe_toasts_dismiss")));
        if let Some(ids) = pending {
            ctx.data_mut(|d| d.remove::<Vec<u64>>(egui::Id::new("yinhe_toasts_dismiss")));
            for id in ids {
                self.dismiss_toast(id);
            }
        }
    }

    /// 单张卡片：返回 true 表示用户点了关闭。
    fn toast_card(ui: &mut egui::Ui, toast: &Toast, width: f32) -> bool {
        let mut dismiss = false;
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
        let resp = frame.show(ui, |ui| {
            ui.set_max_width(width - 20.0);
            ui.set_min_width(width - 20.0);
            ui.horizontal(|ui| {
                // 左图标
                let icon = toast.kind.icon();
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(icon.codepoint)
                            .family(icon.font_family())
                            .size(crate::theme::ICON_FONT)
                            .color(toast.kind.color()),
                    )
                    .selectable(false),
                );
                ui.add_space(6.0);
                // 标题 + 消息 垂直
                ui.vertical(|ui| {
                    ui.set_max_width(width - 70.0);
                    if !toast.title.is_empty() {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&toast.title)
                                    .size(crate::theme::SMALL_FONT)
                                    .strong()
                                    .color(crate::theme::text_primary()),
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
                                    .color(crate::theme::text_secondary()),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 关闭按钮
                    let close_icon = ICON_CLOSE;
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        close_icon.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT_SM, close_icon.font_family()),
                        crate::theme::text_muted(),
                        false,
                    );
                    if resp.clicked() {
                        dismiss = true;
                    }
                });
            });
            // 进度条（TTL 剩余）：底部细线
            let elapsed = toast.created.elapsed().as_secs_f32();
            let total = toast.ttl.as_secs_f32().max(0.001);
            let frac = 1.0 - (elapsed / total).clamp(0.0, 1.0);
            if frac > 0.0 && frac < 1.0 {
                ui.add_space(6.0);
                let bar_w = width - 20.0;
                let bar_h = 2.0;
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
                let bg = crate::theme::line_fg().gamma_multiply(0.25);
                ui.painter().rect_filled(rect, 1.0, bg);
                let fg_rect = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width() * frac, rect.height()),
                );
                ui.painter()
                    .rect_filled(fg_rect, 1.0, toast.kind.color().gamma_multiply(0.85));
            }
        });
        // 点击卡片本身不关闭，仅 X 按钮
        let _ = resp;
        dismiss
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

        // 可视高度限制：历史很多时用 ScrollArea 承接，避免铺满全屏
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
                    .auto_shrink([true, true])
                    .show(ui, |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = GAP;
                            let mut to_remove: Vec<u64> = Vec::new();
                            // top_down：旧在上新在下，与 transient 一致，新 toast 贴底
                            for entry in self.history.iter() {
                                if Self::history_card(ui, entry, CARD_W) {
                                    to_remove.push(entry.id);
                                }
                            }
                            if !to_remove.is_empty() {
                                ui.ctx().data_mut(|d| {
                                    d.insert_temp(
                                        egui::Id::new("yinhe_history_dismiss"),
                                        to_remove,
                                    );
                                });
                            }
                        });
                    });
            });
        let pending: Option<Vec<u64>> =
            ctx.data(|d| d.get_temp(egui::Id::new("yinhe_history_dismiss")));
        if let Some(ids) = pending {
            ctx.data_mut(|d| d.remove::<Vec<u64>>(egui::Id::new("yinhe_history_dismiss")));
            for id in ids {
                self.history.retain(|e| e.id != id);
                self.toasts.retain(|t| t.id != id);
            }
        }
    }

    /// 历史卡片：与 toast_card 同样式但无 TTL 进度条，复用相同浮空外观
    fn history_card(ui: &mut egui::Ui, entry: &HistoryEntry, width: f32) -> bool {
        let mut dismiss = false;
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
                    ui.set_max_width(width - 70.0);
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let close_icon = ICON_CLOSE;
                    let resp = crate::widgets::hover::hover_button(
                        ui,
                        close_icon.codepoint,
                        egui::FontId::new(crate::theme::ICON_FONT_SM, close_icon.font_family()),
                        crate::theme::text_muted(),
                        false,
                    );
                    if resp.clicked() {
                        dismiss = true;
                    }
                });
            });
        });
        dismiss
    }
}
