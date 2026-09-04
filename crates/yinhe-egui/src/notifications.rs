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
        // 若还有存活 toast，每 100ms 唤醒一次以检测过期
        if !self.toasts.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    // ── Toast 浮空渲染：右下角 → 右上角堆叠，浮于内容之上 ──
    pub fn show_toasts(&mut self, ctx: &egui::Context) {
        self.tick(ctx);
        if self.toasts.is_empty() {
            return;
        }
        // 从右下角向上堆叠，浮空在所有 Panel 之上。
        // 用单独的 Area + bottom_up 布局，卡片宽度固定 360。
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        // mode_bar 高度约 28，加 12 边距避免贴边
        const BOTTOM_PAD: f32 = 40.0;
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
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                    ui.spacing_mut().item_spacing.y = GAP;
                    // newest at bottom → 逆序遍历，让最新的贴底
                    let mut to_dismiss: Vec<u64> = Vec::new();
                    // 为了 bottom_up，新est 先画（在最底）
                    for toast in self.toasts.iter().rev() {
                        let resp = Self::toast_card(ui, toast, CARD_W);
                        if resp {
                            to_dismiss.push(toast.id);
                        }
                    }
                    // 收集待关闭
                    if !to_dismiss.is_empty() {
                        // 延迟移除，避免借用冲突（外层 &mut self 已在闭包内）
                        // 这里用 ctx data 传递？改为直接在外层处理：先收集，闭包结束后移除。
                        // 但闭包内无法 &mut self.toasts，所以改为在闭包外处理：
                        // 技巧：把 to_dismiss 写进 egui temp
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

    // ── 通知中心浮层：点击铃铛在右下角弹出，可滚动历史 ──
    #[allow(clippy::collapsible_if)]
    pub fn show_center(&mut self, ctx: &egui::Context) {
        if !self.show_center {
            return;
        }
        const W: f32 = 380.0;
        const H: f32 = 420.0;
        const BOTTOM_PAD: f32 = 40.0;
        const RIGHT_PAD: f32 = 12.0;

        // 点击外部关闭：检测是否点在浮层外且非铃铛
        let mut should_close = false;
        if ctx.input(|i| i.pointer.any_click()) {
            if let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) {
                let screen = ctx.viewport_rect();
                let panel_rect = egui::Rect::from_min_size(
                    egui::pos2(screen.max.x - RIGHT_PAD - W, screen.max.y - BOTTOM_PAD - H),
                    egui::vec2(W, H),
                );
                // 底部 mode_bar 区域点击也视为外部
                let bar_h = 28.0;
                let bar_rect = egui::Rect::from_min_max(
                    egui::pos2(screen.min.x, screen.max.y - bar_h),
                    screen.max,
                );
                // 若点击在面板内或铃铛大致区域（右下角 bar 内靠右 40px）则不关闭
                let bell_rect = egui::Rect::from_min_max(
                    egui::pos2(screen.max.x - 40.0, screen.max.y - bar_h),
                    screen.max,
                );
                if !panel_rect.contains(pos) && !bell_rect.contains(pos) && !bar_rect.contains(pos)
                {
                    // 严格：只有点空白内容区才关，bar 上其他按钮不触发
                    // 简化：点面板外就关，但排除铃铛
                    if !bell_rect.contains(pos) {
                        // 额外判断：若点在 toast 区域，透传不关？
                        // 暂不处理，直接关
                        should_close = true;
                    }
                } else if panel_rect.contains(pos) {
                    // 点面板内不关
                } else if bell_rect.contains(pos) {
                    // 点铃铛由 mode_bar 自身切换，这里不重复处理
                } else if bar_rect.contains(pos) && !bell_rect.contains(pos) {
                    // 点 mode_bar 其他区域 → 关闭
                    should_close = true;
                }
            }
        }
        if should_close {
            self.show_center = false;
            return;
        }

        egui::Area::new(egui::Id::new("yinhe_notification_center"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-RIGHT_PAD, -BOTTOM_PAD),
            )
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .show(ctx, |ui| {
                let frame = egui::Frame {
                    fill: crate::theme::app_bg(),
                    stroke: egui::Stroke::new(1.0, crate::theme::line_fg().gamma_multiply(0.35)),
                    corner_radius: egui::CornerRadius::same(8),
                    shadow: egui::Shadow {
                        offset: [0, 6],
                        blur: 20,
                        spread: 0,
                        color: egui::Color32::from_black_alpha(70),
                    },
                    inner_margin: egui::Margin::symmetric(10, 10),
                    ..Default::default()
                };
                frame.show(ui, |ui| {
                    ui.set_max_width(W - 20.0);
                    ui.set_min_width(W - 20.0);
                    ui.set_max_height(H - 20.0);
                    // 标题栏
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("通知")
                                    .size(crate::theme::BODY_FONT)
                                    .strong()
                                    .color(crate::theme::text_primary()),
                            )
                            .selectable(false),
                        );
                        let unread = self.history.iter().filter(|e| !e.read).count();
                        if unread > 0 {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(format!("{} 未读", unread))
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::accent_active()),
                                )
                                .selectable(false),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            // 关闭
                            let close_icon = ICON_CLOSE;
                            let resp = crate::widgets::hover::hover_button(
                                ui,
                                close_icon.codepoint,
                                egui::FontId::new(
                                    crate::theme::ICON_FONT_SM,
                                    close_icon.font_family(),
                                ),
                                crate::theme::text_muted(),
                                false,
                            );
                            if resp.clicked() {
                                self.show_center = false;
                            }
                            ui.add_space(8.0);
                            // 清空
                            if !self.history.is_empty() {
                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("清空")
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_muted()),
                                    )
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                                );
                                if resp.clicked() {
                                    self.history.clear();
                                }
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                            ui.add_space(8.0);
                            // 全部已读
                            if unread > 0 {
                                let resp = ui.add(
                                    egui::Label::new(
                                        egui::RichText::new("全部已读")
                                            .size(crate::theme::SMALL_FONT)
                                            .color(crate::theme::text_muted()),
                                    )
                                    .sense(egui::Sense::click())
                                    .selectable(false),
                                );
                                if resp.clicked() {
                                    for e in &mut self.history {
                                        e.read = true;
                                    }
                                }
                                if resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(4.0);

                    if self.history.is_empty() {
                        ui.vertical_centered(|ui| {
                            ui.add_space(40.0);
                            let icon = ICON_NOTIFICATIONS;
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(icon.codepoint)
                                        .family(icon.font_family())
                                        .size(crate::theme::ICON_FONT_XL)
                                        .color(crate::theme::text_muted()),
                                )
                                .selectable(false),
                            );
                            ui.add_space(8.0);
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new("暂无通知")
                                        .size(crate::theme::SMALL_FONT)
                                        .color(crate::theme::text_muted()),
                                )
                                .selectable(false),
                            );
                        });
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(H - 70.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 6.0;
                                // 新的在上
                                let mut to_remove: Option<u64> = None;
                                for entry in self.history.iter().rev() {
                                    let is_unread = !entry.read;
                                    let bg = if is_unread {
                                        crate::theme::selected_bg().gamma_multiply(0.55)
                                    } else {
                                        egui::Color32::TRANSPARENT
                                    };
                                    let frame = egui::Frame {
                                        fill: bg,
                                        corner_radius: egui::CornerRadius::same(6),
                                        inner_margin: egui::Margin::symmetric(8, 6),
                                        ..Default::default()
                                    };
                                    let resp = frame.show(ui, |ui| {
                                        ui.set_max_width(W - 40.0);
                                        ui.horizontal(|ui| {
                                            let icon = entry.kind.icon();
                                            ui.add(
                                                egui::Label::new(
                                                    egui::RichText::new(icon.codepoint)
                                                        .family(icon.font_family())
                                                        .size(crate::theme::ICON_FONT_SM)
                                                        .color(entry.kind.color()),
                                                )
                                                .selectable(false),
                                            );
                                            ui.add_space(6.0);
                                            ui.vertical(|ui| {
                                                ui.set_max_width(W - 90.0);
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
                                                if !entry.message.is_empty() {
                                                    ui.add(
                                                        egui::Label::new(
                                                            egui::RichText::new(&entry.message)
                                                                .size(crate::theme::SMALL_FONT)
                                                                .color(
                                                                    crate::theme::text_secondary(),
                                                                ),
                                                        )
                                                        .selectable(false)
                                                        .wrap(),
                                                    );
                                                }
                                                let ago = format_ago(entry.created);
                                                ui.add(
                                                    egui::Label::new(
                                                        egui::RichText::new(ago)
                                                            .size(crate::theme::MODE_LABEL_FONT)
                                                            .color(crate::theme::text_muted()),
                                                    )
                                                    .selectable(false),
                                                );
                                            });
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let close_icon = ICON_CLOSE;
                                                    let r = crate::widgets::hover::hover_button(
                                                        ui,
                                                        close_icon.codepoint,
                                                        egui::FontId::new(
                                                            crate::theme::ICON_FONT_SM,
                                                            close_icon.font_family(),
                                                        ),
                                                        crate::theme::text_muted(),
                                                        false,
                                                    );
                                                    if r.clicked() {
                                                        to_remove = Some(entry.id);
                                                    }
                                                },
                                            );
                                        });
                                    });
                                    let _ = resp;
                                }
                                if let Some(id) = to_remove {
                                    self.history.retain(|e| e.id != id);
                                    self.toasts.retain(|t| t.id != id);
                                }
                            });
                    }
                });
            });
        // 打开即标记已读（下次打开铃铛变回普通）
        if self.show_center {
            // 不立即全已读，保留未读直到用户点"全部已读"或关闭后标记？
            // 按常见设计：打开即已读
            // 这里延迟到下一帧？先不自动，靠按钮
        }
    }

    /// 当通知中心关闭时由外部调用，标记已读（可选）。
    /// 目前策略：关闭时不自动已读，需用户手动；如需自动，取消注释。
    pub fn on_center_closed_mark_read(&mut self) {
        // self.mark_all_read();
    }
}

fn format_ago(t: Instant) -> String {
    let secs = t.elapsed().as_secs();
    if secs < 60 {
        "刚刚".to_string()
    } else if secs < 3600 {
        format!("{} 分钟前", secs / 60)
    } else if secs < 86400 {
        format!("{} 小时前", secs / 3600)
    } else {
        format!("{} 天前", secs / 86400)
    }
}
