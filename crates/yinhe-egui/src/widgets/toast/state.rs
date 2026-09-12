use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui;

use super::anim::{YAnim, y_anim_value};
use super::model::Notification;

mod progress;
mod push;
mod render;

#[cfg(test)]
mod tests;

// ── 统一通知中心（单一数据源：浮动卡与列表是同一列表的两个视图）──
pub struct Notifications {
    next_id: u64,
    /// 唯一数据源：浮动卡与通知中心列表都是这个列表的视图。
    items: Vec<Notification>,
    /// 通知列表是否展开（由 mode_bar 铃铛切换）。
    pub center_open: bool,
    max_history: usize,
    center_opened_at: Option<Instant>,
    center_closed_at: Option<Instant>,
    prev_center_open: bool,
    /// 完成通知自动收起秒数（None=不自动收起；设置页同步，每帧覆盖）。
    collapse_secs: Option<u32>,
    /// 可操作通知自动收起秒数（None=不自动收起；设置页同步，每帧覆盖）。
    action_collapse_secs: Option<u32>,
    /// 是否开启通知（设置页同步，每帧覆盖；关闭时不再建卡、不再记入历史）。
    enabled: bool,
    /// 上次 tick 时刻（悬停暂停按帧间隔顺延 deadline 用）。
    last_tick: Option<Instant>,
    /// 每张卡上帧实测高度（`ui.min_rect().height()`），堆叠按真实高度累加。
    card_h: HashMap<u64, f32>,
    /// Y 轴自有 ease-out 动画状态（id → 起点/终点/起始时刻，时长 0.35s）。
    y_anim: HashMap<u64, YAnim>,
    /// 列表滚动偏移（开列表时两处目标 y 统一减去它）。
    center_scroll: f32,
    /// 上帧最大滚动（贴底跟随与 clamp 用，每帧更新）。
    center_scroll_max: f32,
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

impl Notifications {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            items: Vec::new(),
            center_open: false,
            max_history: 100,
            center_opened_at: None,
            center_closed_at: None,
            prev_center_open: false,
            collapse_secs: Some(5),
            action_collapse_secs: Some(60),
            enabled: true,
            last_tick: None,
            card_h: HashMap::new(),
            y_anim: HashMap::new(),
            center_scroll: 0.0,
            center_scroll_max: 0.0,
        }
    }

    /// 从设置同步自动收起时长（main_loop 每帧调一次，两次 u32 拷贝）。
    pub fn set_collapse_durations(
        &mut self,
        collapse_secs: Option<u32>,
        action_collapse_secs: Option<u32>,
    ) {
        self.collapse_secs = collapse_secs;
        self.action_collapse_secs = action_collapse_secs;
    }

    /// 从设置同步通知总开关（main_loop 每帧调一次）。
    /// 关闭时已有卡走正常退场动画（不再立即清空），历史保留。
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            // 防卡死：关闭总开关时自动 resume 已暂停任务，否则任务永远暂停且无 UI 可恢复。
            for n in &self.items {
                if let Some(p) = n.pause_flag() {
                    p.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let ids: Vec<u64> = self
                .items
                .iter()
                .filter(|n| n.on_screen && n.leaving_since.is_none())
                .map(|n| n.id)
                .collect();
            for id in ids {
                self.dismiss(id);
            }
        }
        self.enabled = enabled;
    }

    /// 通知总开关是否开启（关闭时 mode_bar 铃铛隐藏）。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// 分配下一个通知 id。
    pub(super) fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// 按 id 找条目。
    pub(super) fn get(&self, id: u64) -> Option<&Notification> {
        self.items.iter().find(|n| n.id == id)
    }

    fn collapse_deadline(&self, secs: Option<u32>) -> Option<Instant> {
        secs.map(|s| Instant::now() + Duration::from_secs(u64::from(s)))
    }

    /// 该卡上帧实测高度；无实测（首帧）回退固定估算。
    fn measured_h(&self, id: u64, fallback: f32) -> f32 {
        self.card_h.get(&id).copied().unwrap_or(fallback)
    }

    /// Y 轴 ease-out 显示值：无记录或目标变更时从当前显示值重起，保证不断裂。
    fn y_for(&mut self, id: u64, target: f32, now: Instant) -> f32 {
        let need_reset = match self.y_anim.get(&id) {
            None => true,
            Some(a) => a.to != target,
        };
        if need_reset {
            let cur = match self.y_anim.get(&id) {
                Some(a) => y_anim_value(a, now),
                None => target,
            };
            self.y_anim.insert(
                id,
                YAnim {
                    from: cur,
                    to: target,
                    t0: now,
                },
            );
            cur
        } else if let Some(a) = self.y_anim.get(&id) {
            y_anim_value(a, now)
        } else {
            target
        }
    }

    /// 列表内容总高：按创建顺序用实测高度累加（含 GAP）。
    fn center_total_h(&self, fallback: f32, gap: f32) -> f32 {
        if self.items.is_empty() {
            return 0.0;
        }
        let mut total = 0.0;
        for n in &self.items {
            total += self.card_h.get(&n.id).copied().unwrap_or(fallback);
        }
        total += gap * ((self.items.len() as f32) - 1.0).max(0.0);
        total
    }

    fn perform_action(action: &super::model::ToastAction) {
        match &action.kind {
            super::model::ToastActionKind::RevealInFolder(path) => {
                crate::platform::open_containing_folder(path);
            }
        }
    }

    /// 收起浮卡：标记退场（320ms 后翻 off-screen），不动列表数据。
    /// 进行中任务收起时自动 resume 暂停（防卡死）：卡片消失后若仍 paused
    /// 任务永停且无 UI 可恢复；翻 off-screen 后 ensure 不重建卡，防复活。
    pub fn dismiss(&mut self, id: u64) {
        let Some(n) = self.items.iter_mut().find(|n| n.id == id) else {
            return;
        };
        if !n.on_screen {
            return;
        }
        if let Some(p) = n.pause_flag() {
            p.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        if n.leaving_since.is_none() {
            n.leaving_since = Some(Instant::now());
        }
    }

    /// 历史上限：只裁已收起的旧条目，屏幕上的浮卡不动。
    pub(super) fn prune_overflow(&mut self) {
        let archived = self.items.iter().filter(|n| !n.on_screen).count();
        if archived <= self.max_history {
            return;
        }
        let mut excess = archived - self.max_history;
        self.items.retain(|n| {
            if excess > 0 && !n.on_screen {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }

    fn tick(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        if self.center_open != self.prev_center_open {
            let was_open = self.prev_center_open;
            if self.center_open {
                self.center_opened_at = Some(now);
                self.center_closed_at = None;
                // 开列表边沿：旧未读清零（列表开着时新条目直接已读，见 push 系列）。
                self.mark_all_read();
            } else {
                self.center_opened_at = None;
                self.center_closed_at = Some(now);
            }
            self.prev_center_open = self.center_open;
            ctx.request_repaint();
            // 列表关闭瞬间：屏幕上所有卡全部走退场动画，进行中顺带防复活+resume（见 dismiss）
            if was_open && !self.center_open {
                let ids: Vec<u64> = self
                    .items
                    .iter()
                    .filter(|n| n.on_screen && n.leaving_since.is_none())
                    .map(|n| n.id)
                    .collect();
                for id in ids {
                    self.dismiss(id);
                }
            }
        }
        let mut needs_repaint = false;
        // 悬停暂停：上帧悬停的卡，deadline 按本帧间隔顺延（精确暂停，不断计时）
        let dt = self
            .last_tick
            .map(|t| now.duration_since(t))
            .unwrap_or(Duration::ZERO)
            .min(Duration::from_secs(1));
        self.last_tick = Some(now);
        if dt > Duration::ZERO {
            for n in self.items.iter_mut() {
                if n.hovered
                    && n.leaving_since.is_none()
                    && let Some(at) = n.collapse_at
                {
                    n.collapse_at = Some(at + dt);
                }
            }
        }
        // 退场动画播完：翻 off-screen（条目保留在列表里）
        let mut retired = false;
        for n in self.items.iter_mut() {
            if n.leaving_since
                .is_some_and(|since| now.duration_since(since) >= Duration::from_millis(320))
            {
                n.leaving_since = None;
                n.on_screen = false;
                retired = true;
            }
        }
        if retired {
            self.prune_overflow();
            needs_repaint = true;
        }
        // 实测高度/y 动画只保留还存在的条目，防 map 无限涨（量级小，直接查）。
        self.card_h
            .retain(|id, _| self.items.iter().any(|n| n.id == *id));
        self.y_anim
            .retain(|id, _| self.items.iter().any(|n| n.id == *id));
        // 自动收起到期：走正常退场，只收浮动卡，列表数据保留；
        // 列表开着时跳过（deadline 自然过期不管它）
        let expired: Vec<u64> = if self.center_open {
            Vec::new()
        } else {
            self.items
                .iter()
                .filter(|n| {
                    n.on_screen
                        && n.leaving_since.is_none()
                        && n.collapse_at.is_some_and(|at| at <= now)
                })
                .map(|n| n.id)
                .collect()
        };
        for id in expired {
            self.dismiss(id);
            needs_repaint = true;
        }
        let has_anim = self.items.iter().any(|n| {
            n.leaving_since.is_some()
                || n.source.is_some()
                || now.duration_since(n.created) < Duration::from_millis(400)
                || n.progress.is_some_and(|p| p < 0.999)
        });
        if has_anim || needs_repaint {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if !self.items.is_empty() {
            // 常驻通知无动画时仍需偶尔重绘以响应 hover
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        // 有未到期的自动收起则准时唤醒（精度±500ms 内可接受，不另起 timer）
        if let Some(wait) = self
            .items
            .iter()
            .filter(|n| n.leaving_since.is_none())
            .filter_map(|n| n.collapse_at)
            .filter(|at| *at > now)
            .min()
            .and_then(|at| at.checked_duration_since(now))
        {
            ctx.request_repaint_after(wait.min(Duration::from_secs(3600)));
        }
        // history 展开时也需动画（兜底：toast 回退也需）以及重排动画
        if self.center_open && !self.items.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        // 列表关闭退场动画进行中也需重绘
        if let Some(closed) = self.center_closed_at
            && now.duration_since(closed) < Duration::from_millis(400)
        {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }
}
