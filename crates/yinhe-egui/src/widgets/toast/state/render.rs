use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;

use super::super::layout::{is_fully_outside, stack_ys};
use super::super::model::Notification;
use super::super::{anim, card};
use super::Notifications;

const CARD_W: f32 = 360.0;
const GAP: f32 = 8.0;
/// 浮卡底边距中央区域底部的呼吸间隙。
const BOTTOM_GAP: f32 = 8.0;
const RIGHT_PAD: f32 = 32.0;
/// 浮卡堆叠顶部保留（不压顶部工具栏）。
const TOP_PAD: f32 = 24.0;
const EST_H: f32 = 110.0;
const CLOSE_ANIM: Duration = Duration::from_millis(350);

impl Notifications {
    /// 通知中心列表带 `(bottom_gap, max_h)`：完整使用中央内容区，
    /// 顶边贴内容区顶、底边贴内容区底（标题栏/底栏之上）——裁剪线与界面
    /// 边缘重合，列表内部不再出现悬空的裁剪线。
    fn center_band(ctx: &egui::Context, area: egui::Rect) -> (f32, f32) {
        let viewport = ctx.viewport_rect();
        let bottom = area.max.y.min(viewport.max.y);
        let bottom_gap = (viewport.max.y - bottom).max(0.0);
        // max_h 即区域全高：Area 顶 = area.max.y - max_h = area.min.y。
        // （曾再减一次 bottom_gap，导致顶部凭空少掉一个底栏的高度）
        let max_h = area.height().max(120.0);
        (bottom_gap, max_h)
    }

    /// 浮卡带 `(bottom_gap, max_h)`：底边距内容区底 BOTTOM_GAP，顶边留 TOP_PAD。
    fn toast_band(ctx: &egui::Context, area: egui::Rect) -> (f32, f32) {
        let viewport = ctx.viewport_rect();
        let bottom = area.max.y.min(viewport.max.y);
        let bottom_gap = (viewport.max.y - bottom + BOTTOM_GAP).max(BOTTOM_GAP);
        let max_h = (area.height() - TOP_PAD - BOTTOM_GAP).max(120.0);
        (bottom_gap, max_h)
    }

    /// 条目是否由通知中心列表接管渲染：列表展开时全部接管；
    /// 关闭退场窗口内，关闭前已存在的条目随列表整列滑出（之后新 push 的走浮卡）。
    fn center_takes(
        n: &Notification,
        center_open: bool,
        closed_at: Option<Instant>,
        now: Instant,
    ) -> bool {
        if center_open {
            return true;
        }
        closed_at.is_some_and(|t| now.duration_since(t) <= CLOSE_ANIM && n.created <= t)
    }

    // ── 浮卡渲染：右下角 → 右上角堆叠，浮于内容之上 ──
    // 每个 toast 独立 Area，避免父 Area+ScrollArea 宽度异常导致右侧溢出。
    pub fn show_toasts(&mut self, ctx: &egui::Context, area: egui::Rect) {
        self.tick(ctx);
        let now = Instant::now();
        let needs_float = self.items.iter().any(|n| {
            n.on_screen && !Self::center_takes(n, self.center_open, self.center_closed_at, now)
        });
        if !needs_float {
            return;
        }
        let (bottom_gap, max_h) = Self::toast_band(ctx, area);
        // 最大 y 偏移（相对底边）：底留白 + 可见高
        let max_y = bottom_gap + max_h;

        // 浮动堆叠目标 y：最新在底，按每张卡实测高度累加
        let ids: Vec<u64> = self
            .items
            .iter()
            .rev()
            .filter(|n| {
                n.on_screen && !Self::center_takes(n, self.center_open, self.center_closed_at, now)
            })
            .map(|n| n.id)
            .collect();
        let ys = stack_ys(&self.card_h, &ids, bottom_gap, GAP, EST_H);
        let mut toast_y_map: HashMap<u64, f32> = HashMap::new();
        for (id, y) in ids.iter().zip(ys) {
            toast_y_map.insert(*id, y);
        }

        let mut to_dismiss: Vec<u64> = Vec::new();
        for idx in (0..self.items.len()).rev() {
            if !self.items[idx].on_screen {
                continue;
            }
            if Self::center_takes(
                &self.items[idx],
                self.center_open,
                self.center_closed_at,
                now,
            ) {
                continue;
            }
            let tid = self.items[idx].id;
            let target_y = toast_y_map.get(&tid).copied().unwrap_or(bottom_gap);
            let card_h = self.measured_h(tid, EST_H);
            // 自有 ease-out y 插值（与 x 飞行动画同族曲线），堆叠重排不线性
            let y_off = self.y_for(tid, target_y, now);
            // 用“显示位置”判可见性：快速重排时目标先出带、动画仍在滑出，
            // 若按目标裁会未滑完就消失（提前消失）；显示位置完全出带才跳过
            if is_fully_outside(y_off, card_h, bottom_gap, max_y) {
                self.items[idx].hovered = false;
                continue;
            }
            let x_off = anim::fly_anim(&self.items[idx]);
            let cancel_flag = self.items[idx].cancel_flag();
            let action_opt = self.items[idx].action.clone();
            let mut outcome = super::super::model::CardOutcome::default();
            let area_id = egui::Id::new(("yinhe_notif", tid));
            let area_resp = egui::Area::new(area_id)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-RIGHT_PAD + x_off, -y_off),
                )
                .order(egui::Order::Tooltip)
                .movable(false)
                .interactable(true)
                // 允许飞出视口：否则 egui 默认 constrain 会把屏外的起点拉回窗内，
                // 卡片看起来就是“右侧贴窗”而不是“从窗外滑入”
                .constrain(false)
                // 关 egui Area 自带 fade-in（默认 true，0.2s 透明度动画）：位移只走自有飞入
                .fade_in(false)
                .show(ctx, |ui| {
                    outcome = card::toast_card(ui, &self.items[idx], CARD_W, true);
                    ui.min_rect().height()
                });
            self.card_h.insert(tid, area_resp.inner);
            self.items[idx].hovered = outcome.hovered;
            if outcome.cancel {
                // stop：只置 flag + 中止态，卡片留着等 abort 确认，不进退场
                if let Some(c) = cancel_flag {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.items[idx].cancelling = true;
            } else if outcome.dismiss {
                to_dismiss.push(tid);
            }
            // 操作按钮只执行不收卡（收起交给自动计时）
            if outcome.action
                && let Some(a) = action_opt
            {
                Self::perform_action(&a);
            }
        }
        if !to_dismiss.is_empty() {
            // 去重，避免同一 id 既 dismiss 又 cancel 重复
            to_dismiss.sort_unstable();
            to_dismiss.dedup();
            for id in to_dismiss {
                self.dismiss(id);
            }
        }
    }

    // ── 通知中心列表：ScrollArea 从底部向上堆积，打开/关闭整列滑入滑出 ──
    pub fn show_center(&mut self, ctx: &egui::Context, area: egui::Rect) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        let closing = !self.center_open;
        if closing {
            let Some(closed_at) = self.center_closed_at else {
                return;
            };
            if now.duration_since(closed_at) > CLOSE_ANIM {
                return;
            }
        }
        let taken: Vec<usize> = (0..self.items.len())
            .filter(|&i| {
                Self::center_takes(&self.items[i], self.center_open, self.center_closed_at, now)
            })
            .collect();
        if taken.is_empty() {
            return;
        }

        let (bottom_gap, max_h) = Self::center_band(ctx, area);
        // 打开：无停顿整列从右侧滑入；关闭：入场的严格反向滑出
        let x_off = if closing {
            let closed_at = self.center_closed_at.unwrap_or(now);
            anim::exit_x(now.duration_since(closed_at).as_secs_f32())
        } else if let Some(opened_at) = self.center_opened_at {
            anim::enter_x(now.duration_since(opened_at).as_secs_f32())
        } else {
            0.0
        };
        // 内容不足一屏时顶部补白，让最新消息贴底（stick_to_bottom 只管滚动位置）
        let mut total: f32 = 0.0;
        for &i in &taken {
            total += self.card_h.get(&self.items[i].id).copied().unwrap_or(EST_H) + GAP;
        }
        total -= GAP;
        let pad_top = (max_h - total).max(0.0);

        // 卡片交互结果（闭包内不能 &mut self，收集后回写）
        let mut outcomes: Vec<(u64, super::super::model::CardOutcome)> = Vec::new();
        // 打开边沿：首帧把滚动定位到最新（底部），不依赖 stick 状态
        let scroll_bottom = self.center_bottom_pending;
        self.center_bottom_pending = false;
        egui::Area::new(egui::Id::new("yinhe_notif_center"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                egui::vec2(-RIGHT_PAD + x_off, -bottom_gap),
            )
            // 显式给出列宽与可见高：Area 首帧默认尺寸极小，ScrollArea 会被压扁成
            // 1px 宽只剩滚动条，default_size 是首帧 Ui max_rect 的依据
            .default_size(egui::vec2(CARD_W + 8.0, max_h))
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .constrain(false)
            .fade_in(false)
            .show(ctx, |ui| {
                ui.set_max_width(CARD_W + 8.0);
                ui.style_mut().spacing.scroll = egui::style::ScrollStyle::thin();
                egui::ScrollArea::vertical()
                    .id_salt("yinhe_notif_center_scroll")
                    .max_width(CARD_W + 8.0)
                    .max_height(max_h)
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    // 打开列表的“定位到最新”即时生效，不做滚动动画
                    .animated(false)
                    .show(ui, |ui| {
                        ui.set_width(CARD_W);
                        if pad_top > 0.0 {
                            ui.add_space(pad_top);
                        }
                        for &i in &taken {
                            let n = &self.items[i];
                            // 浮动层的卡在列表里仍可 stop/pause/操作（无 X）；已收起卡只读
                            if n.on_screen {
                                let outcome = card::toast_card(ui, n, CARD_W, false);
                                outcomes.push((n.id, outcome));
                            } else {
                                card::history_card(ui, n, CARD_W);
                            }
                            ui.add_space(GAP);
                        }
                        if scroll_bottom {
                            ui.scroll_to_cursor(Some(egui::Align::BOTTOM));
                        }
                    });
            });
        // 交互结果在闭包外回写：stop → 中止态；操作按钮 → 执行
        for (id, outcome) in outcomes {
            if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
                n.hovered = outcome.hovered;
                if outcome.cancel {
                    n.cancelling = true;
                }
            }
            if outcome.action {
                let action = self
                    .items
                    .iter()
                    .find(|n| n.id == id)
                    .and_then(|n| n.action.clone());
                if let Some(a) = action {
                    Self::perform_action(&a);
                }
            }
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
