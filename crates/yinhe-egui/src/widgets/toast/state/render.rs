use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;

use super::super::layout::{
    follow_scroll, is_fully_outside, notif_band, scroll_max, stack_ys, visible_h,
};
use super::super::{anim, card};
use super::Notifications;

impl Notifications {
    /// 每帧更新滚动上限：贴底跟随时吸到新 max，否则 clamp 旧值。
    pub(super) fn update_center_scroll(
        &mut self,
        viewport_h: f32,
        bottom_pad: f32,
        gap: f32,
        fallback: f32,
    ) {
        let total = self.center_total_h(fallback, gap);
        let visible = visible_h(viewport_h, bottom_pad);
        let new_max = scroll_max(total, visible);
        let followed = follow_scroll(self.center_scroll, self.center_scroll_max, new_max);
        self.center_scroll_max = new_max;
        self.center_scroll = followed.clamp(0.0, new_max);
    }

    /// 滚动条右缘距屏边（卡片在其左侧，两者留空隙）。
    const SCROLL_RIGHT: f32 = 8.0;
    /// 列表背景捕获（与卡片同 Order::Tooltip 但先画所以在下）：
    /// 覆盖本列实际范围，吞缝隙点击 + 滚轮转滚动。卡片之后再画滚动条（在上）。
    #[allow(clippy::too_many_arguments)]
    fn show_center_bg(
        &mut self,
        ctx: &egui::Context,
        viewport: egui::Rect,
        card_w: f32,
        gap: f32,
        bottom_pad: f32,
        right_pad: f32,
        fallback: f32,
    ) {
        self.update_center_scroll(viewport.height(), bottom_pad, gap, fallback);
        let total = self.center_total_h(fallback, gap);
        if total <= 0.0 {
            return;
        }
        let scroll = self.center_scroll;
        let left = viewport.max.x - right_pad - card_w;
        // 背景盖到滚动条右缘（含条带），滚轮/吞点击对条带也生效
        let right = viewport.max.x - Self::SCROLL_RIGHT;
        let bg_bottom = viewport.max.y - (bottom_pad - gap);
        let top_y_off = bottom_pad + total - scroll + gap;
        let bg_top = (viewport.max.y - top_y_off).max(viewport.min.y);
        if bg_top >= bg_bottom {
            return;
        }
        let bg_rect =
            egui::Rect::from_min_max(egui::pos2(left, bg_top), egui::pos2(right, bg_bottom));
        egui::Area::new(egui::Id::new("yinhe_notif_center_bg"))
            .fixed_pos(bg_rect.min)
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .constrain(false)
            .show(ctx, |ui| {
                let resp = ui.allocate_response(bg_rect.size(), egui::Sense::click());
                // 吞掉缝隙点击，不做任何事
                let _ = resp.clicked();
            });
        // 滚轮：仅指针在本列时生效
        let over_column = ctx.input(|i| i.pointer.hover_pos().is_some_and(|p| bg_rect.contains(p)));
        if over_column {
            let dy = ctx.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                // y_off 从底边向上量：上滑（dy>0）内容下移看旧消息 → scroll 增大
                let max = self.center_scroll_max;
                self.center_scroll = (self.center_scroll + dy).clamp(0.0, max);
            }
        }
    }

    /// 细滚动条（列右缘 track+thumb，thumb 可拖），在卡片之后画所以在上。
    #[allow(clippy::too_many_arguments)]
    fn show_center_scrollbar(
        &mut self,
        ctx: &egui::Context,
        viewport: egui::Rect,
        _card_w: f32,
        gap: f32,
        bottom_pad: f32,
        _right_pad: f32,
        fallback: f32,
    ) {
        let max = self.center_scroll_max;
        if max <= 0.0 {
            return;
        }
        let total = self.center_total_h(fallback, gap);
        if total <= 0.0 {
            return;
        }
        let scroll = self.center_scroll;
        let visible = visible_h(viewport.height(), bottom_pad);
        // 滚动条贴屏边独立条带，与卡片留 18px 空隙
        let right = viewport.max.x - Self::SCROLL_RIGHT;
        let bg_bottom = viewport.max.y - (bottom_pad - gap);
        let top_y_off = bottom_pad + total - scroll + gap;
        let bg_top = (viewport.max.y - top_y_off).max(viewport.min.y);
        if bg_top >= bg_bottom {
            return;
        }
        const TRACK_W: f32 = 6.0;
        let track_rect = egui::Rect::from_min_max(
            egui::pos2(right - TRACK_W, bg_top),
            egui::pos2(right, bg_bottom),
        );
        let track_h = track_rect.height();
        if track_h <= 0.0 {
            return;
        }
        let thumb_h = ((visible / total.max(1.0)) * track_h).clamp(24.0, track_h);
        let travel = (track_h - thumb_h).max(1.0);
        let ratio = if max > 0.0 {
            (scroll / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // scroll=0 看的是底部最新消息，thumb 应在底部
        let thumb_y = track_rect.min.y + (1.0 - ratio) * travel;
        let thumb_rect = egui::Rect::from_min_size(
            egui::pos2(track_rect.min.x, thumb_y),
            egui::vec2(TRACK_W, thumb_h),
        );
        // 先画 track+thumb，再用透明 Area 承接拖动（同层后画在上）
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("yinhe_notif_scroll_paint"),
        ));
        painter.rect_filled(track_rect, 3.0, egui::Color32::from_black_alpha(50));
        painter.rect_filled(thumb_rect, 3.0, egui::Color32::from_black_alpha(130));
        let drag_dy = egui::Area::new(egui::Id::new("yinhe_notif_scrollbar"))
            .fixed_pos(track_rect.min)
            .order(egui::Order::Tooltip)
            .movable(false)
            .interactable(true)
            .constrain(false)
            .show(ctx, |ui| {
                let _ = ui.allocate_response(egui::vec2(TRACK_W, track_h), egui::Sense::click());
                let thumb_resp = ui.interact(
                    thumb_rect,
                    egui::Id::new("yinhe_notif_thumb"),
                    egui::Sense::drag(),
                );
                if thumb_resp.dragged() {
                    thumb_resp.drag_delta().y
                } else {
                    0.0
                }
            })
            .inner;
        if drag_dy != 0.0 {
            // 下拽 thumb（drag_dy>0）内容上移看新消息 → scroll 减小
            self.center_scroll = (self.center_scroll - drag_dy * max / travel).clamp(0.0, max);
        }
    }

    /// 历史堆叠目标 y：全部条目按创建顺序（最新在底）累加实测高度。
    fn history_ys(
        items: &[super::super::model::Notification],
        card_h: &HashMap<u64, f32>,
        bottom_pad: f32,
        gap: f32,
        fallback: f32,
    ) -> HashMap<u64, f32> {
        let ids: Vec<u64> = items.iter().rev().map(|n| n.id).collect();
        let mut out = HashMap::new();
        for (id, y) in ids
            .iter()
            .zip(stack_ys(card_h, &ids, bottom_pad, gap, fallback))
        {
            out.insert(*id, y);
        }
        out
    }

    // ── Toast 浮空渲染：右下角 → 右上角堆叠，浮于内容之上 ──
    // 每个 toast 独立 Area，避免父 Area+ScrollArea 宽度异常导致右侧溢出。
    // 打开通知列表时，已存在的 toast 不消失，而是通过非线性 y 插值重排至历史位置；其余历史项飞入。
    pub fn show_toasts(&mut self, ctx: &egui::Context) {
        self.tick(ctx);
        if !self.items.iter().any(|n| n.on_screen) {
            return;
        }
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 32.0;
        const EST_H: f32 = 110.0;

        let viewport = ctx.viewport_rect();
        // 最大 y 偏移（相对底边）：底留白 + 可见高。与滚动上限同源，否则滚到顶会被多切 48px。
        let max_y = BOTTOM_PAD + visible_h(viewport.height(), BOTTOM_PAD);
        // 列表开着时先刷新滚动上限，保证两处用同一 scroll（show_center 随后还会刷一次，同输入幂等）。
        if self.center_open {
            self.update_center_scroll(viewport.height(), BOTTOM_PAD, GAP, EST_H);
        }

        // 预计算目标 y：浮动卡堆叠 vs 历史堆叠（按每张卡实测高度累加）
        let on_screen_ids: Vec<u64> = self
            .items
            .iter()
            .filter(|n| n.on_screen)
            .map(|n| n.id)
            .collect();
        let mut toast_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let ids: Vec<u64> = on_screen_ids.iter().rev().copied().collect();
            for (id, y) in ids
                .iter()
                .zip(stack_ys(&self.card_h, &ids, BOTTOM_PAD, GAP, EST_H))
            {
                toast_y_map.insert(*id, y);
            }
        }
        let history_y_map = Self::history_ys(&self.items, &self.card_h, BOTTOM_PAD, GAP, EST_H);

        let mut to_dismiss: Vec<u64> = Vec::new();
        for idx in (0..self.items.len()).rev() {
            if !self.items[idx].on_screen {
                continue;
            }
            let tid = self.items[idx].id;
            let raw_y = if self.center_open {
                // 重排至历史中的位置
                history_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD)
            } else {
                toast_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD)
            };
            // 列表开着时两处统一减滚动
            let target_y = if self.center_open {
                raw_y - self.center_scroll
            } else {
                raw_y
            };
            let card_h = self.measured_h(tid, EST_H);
            // 自有 ease-out y 插值（与 x 飞行动画同族曲线），重排不线性
            let y_off = self.y_for(tid, target_y, Instant::now());
            // 用“显示位置”判可见性：快速滚动时目标先出带、动画仍在滑出，
            // 若按目标裁会未滑完就消失（提前消失）；显示位置完全出带才跳过
            if is_fully_outside(y_off, card_h, BOTTOM_PAD, max_y) {
                self.items[idx].hovered = false;
                continue;
            }
            let is_leaving = self.items[idx].leaving_since.is_some();
            // 打开列表时已存在的 toast 不重新飞入，仅重排；离开时仍飞出
            let x_off = if self.center_open && !is_leaving {
                0.0
            } else {
                anim::fly_anim(&self.items[idx])
            };
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
                    // 边缘裁剪滑出：收窄 clip 到可见带，半张卡被裁掉而非突然消失
                    let clip = ui.clip_rect();
                    ui.set_clip_rect(clip.intersect(notif_band(clip, viewport, BOTTOM_PAD, max_y)));
                    outcome =
                        card::toast_card(ui, &self.items[idx], CARD_W, 0.0, !self.center_open);
                    ui.min_rect().height()
                });
            self.card_h.insert(tid, area_resp.inner);
            self.items[idx].hovered = outcome.hovered;
            if outcome.cancel {
                // stop：只置 flag + 中止态，卡片留着等 abort 确认（见 C），不进退场
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

    pub fn show_center(&mut self, ctx: &egui::Context) {
        if !self.enabled {
            return;
        }
        // 关闭时：历史独有项做退场飞出（入场的严格反向），播完再停
        let closing = !self.center_open;
        if closing {
            let Some(closed_at) = self.center_closed_at else {
                return;
            };
            if Instant::now().duration_since(closed_at) > Duration::from_millis(350) {
                return;
            }
            if self.items.is_empty() {
                return;
            }
        } else if self.items.is_empty() {
            return;
        }
        const CARD_W: f32 = 360.0;
        const GAP: f32 = 8.0;
        const BOTTOM_PAD: f32 = 48.0;
        const RIGHT_PAD: f32 = 32.0;
        const EST_H: f32 = 110.0;

        let viewport = ctx.viewport_rect();
        // 同 show_toasts：最大 y 偏移与滚动上限同源，滚到顶才不会被多切
        let max_y = BOTTOM_PAD + visible_h(viewport.height(), BOTTOM_PAD);

        // 列表开着时先画背景捕获（同层先画在下）：吞缝隙点击 + 滚轮，顺带刷新滚动上限。
        if self.center_open {
            self.show_center_bg(ctx, viewport, CARD_W, GAP, BOTTOM_PAD, RIGHT_PAD, EST_H);
        }

        // 预计算历史目标 y（最新在底部，按每张卡实测高度累加）
        let history_y_map = Self::history_ys(&self.items, &self.card_h, BOTTOM_PAD, GAP, EST_H);

        // 仅渲染不在浮动层的条目（浮动中的由 show_toasts 负责重排）
        for idx in (0..self.items.len()).rev() {
            if self.items[idx].on_screen {
                continue;
            }
            let tid = self.items[idx].id;
            let raw_y = history_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD);
            // 列表开着时与 show_toasts 统一减滚动；关闭退场不减
            let target_y = if self.center_open {
                raw_y - self.center_scroll
            } else {
                raw_y
            };
            let card_h = self.measured_h(tid, EST_H);
            let y_off = self.y_for(tid, target_y, Instant::now());
            // 同上：按显示位置判可见，避免快速滚动时提前裁掉正在滑出的卡
            if is_fully_outside(y_off, card_h, BOTTOM_PAD, max_y) {
                continue;
            }
            // 打开：无停顿直接从右侧飞入；关闭：入场的严格反向飞出
            let x_off = if closing {
                let closed_at = self.center_closed_at.unwrap_or_else(Instant::now);
                anim::exit_x(Instant::now().duration_since(closed_at).as_secs_f32())
            } else if let Some(opened_at) = self.center_opened_at {
                anim::enter_x(Instant::now().duration_since(opened_at).as_secs_f32())
            } else {
                0.0
            };
            let area_id = egui::Id::new(("yinhe_notif", tid));
            let area_resp = egui::Area::new(area_id)
                .anchor(
                    egui::Align2::RIGHT_BOTTOM,
                    egui::vec2(-RIGHT_PAD + x_off, -y_off),
                )
                .order(egui::Order::Tooltip)
                .movable(false)
                .interactable(true)
                // 同上：允许从视口外飞入
                .constrain(false)
                // 同上：关 egui Area 自带 fade-in
                .fade_in(false)
                .show(ctx, |ui| {
                    // 边缘裁剪滑出：收窄 clip 到可见带，半张卡被裁掉而非突然消失
                    let clip = ui.clip_rect();
                    ui.set_clip_rect(clip.intersect(notif_band(clip, viewport, BOTTOM_PAD, max_y)));
                    card::history_card(ui, &self.items[idx], CARD_W);
                    ui.min_rect().height()
                });
            self.card_h.insert(tid, area_resp.inner);
        }
        // 滚动条在卡片之后画所以在上（与背景同层，后画在上）。
        if self.center_open {
            self.show_center_scrollbar(ctx, viewport, CARD_W, GAP, BOTTOM_PAD, RIGHT_PAD, EST_H);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}
