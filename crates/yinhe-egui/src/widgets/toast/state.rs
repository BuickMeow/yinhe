use std::collections::{HashMap, HashSet};
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
    /// 完成通知自动收起秒数（None=不自动收起；设置页同步，每帧覆盖）。
    collapse_secs: Option<u32>,
    /// 可操作通知自动收起秒数（None=不自动收起；设置页同步，每帧覆盖）。
    action_collapse_secs: Option<u32>,
    /// 是否开启通知（设置页同步，每帧覆盖；关闭时不再建卡、不再记入历史）。
    enabled: bool,
    /// 上次 tick 时刻（悬停暂停按帧间隔顺延 deadline 用）。
    last_tick: Option<Instant>,
    /// 用户手动收起的进行中任务 id：ensure 只更新历史、不重建卡。
    collapsed: HashSet<u64>,
    /// 固定任务 id → 本次运行的历史条目 id（浮动卡复用固定槽位，历史一任务一条）。
    live_hist: HashMap<u64, u64>,
    /// 每张卡上帧实测高度（`ui.min_rect().height()`），堆叠按真实高度累加。
    card_h: HashMap<u64, f32>,
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
            collapse_secs: Some(5),
            action_collapse_secs: Some(60),
            enabled: true,
            last_tick: None,
            collapsed: HashSet::new(),
            live_hist: HashMap::new(),
            card_h: HashMap::new(),
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
    /// 关闭时已有卡走正常 320ms 退场（不再立即清空），历史保留。
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            // 防卡死：关闭总开关时自动 resume 已暂停任务，否则任务永远暂停且无 UI 可恢复。
            for t in &self.toasts {
                if let Some(p) = super::model::resolve_pause_toast(t) {
                    p.store(false, std::sync::atomic::Ordering::Relaxed);
                }
            }
            let ids: Vec<u64> = self.toasts.iter().map(|t| t.id).collect();
            for id in ids {
                self.dismiss_toast(id);
            }
        }
        self.enabled = enabled;
    }

    /// 通知总开关是否开启（关闭时 mode_bar 铃铛隐藏）。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn collapse_deadline(&self, secs: Option<u32>) -> Option<Instant> {
        secs.map(|s| Instant::now() + Duration::from_secs(u64::from(s)))
    }

    /// 该卡上帧实测高度；无实测（首帧）回退固定估算。
    fn measured_h(&self, id: u64, fallback: f32) -> f32 {
        self.card_h.get(&id).copied().unwrap_or(fallback)
    }

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

    /// 确保进度卡存在：同一 key（如 "loading"）复用同一 id。
    /// 只建卡/换数据源（Arc 交换，无文案拷贝），进度文案渲染时 pull。
    /// 调用方每帧调也无妨，但文案不再每帧拷贝；用户点了 X（leaving）时不复活。
    pub fn ensure_progress(
        &mut self,
        key_id: u64,
        kind: ToastKind,
        source: Arc<dyn super::model::ProgressSource>,
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

    /// 历史条目换数据源（kind/source），无则新建；供 ensure 复用。
    fn sync_history_source(
        &mut self,
        id: u64,
        kind: ToastKind,
        source: Arc<dyn super::model::ProgressSource>,
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
    fn sync_live_history(
        &mut self,
        fixed_id: u64,
        kind: ToastKind,
        source: Arc<dyn super::model::ProgressSource>,
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

    /// 清掉本次运行的 live 历史条目并删映射（封存的不碰）；未映射回退按 id 清，做兼容。
    pub fn prune_history(&mut self, id: u64) {
        if let Some(hist_id) = self.live_hist.remove(&id) {
            self.history.retain(|h| h.id != hist_id);
        } else {
            self.history.retain(|h| h.id != id);
        }
        self.collapsed.remove(&id);
    }

    fn push_history(
        &mut self,
        id: u64,
        kind: ToastKind,
        source: Arc<dyn super::model::ProgressSource>,
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
            .and_then(super::model::resolve_cancel_toast)
    }

    /// 给浮动卡挂操作按钮（如“打开文件夹”）。
    /// 有按钮即可操作，计时自动升为可操作档。
    pub fn set_action(
        &mut self,
        id: u64,
        label: impl Into<String>,
        kind: super::model::ToastActionKind,
    ) {
        self.set_action_with_icon(id, label, kind, None);
    }

    /// 带图标的操作按钮（已中止卡用文件夹图标，无文字，hover 显示 label）。
    pub fn set_action_with_icon(
        &mut self,
        id: u64,
        label: impl Into<String>,
        kind: super::model::ToastActionKind,
        icon: Option<egui_material_icons::MaterialIcon>,
    ) {
        if !self.enabled {
            return;
        }
        let deadline = self.collapse_deadline(self.action_collapse_secs);
        if let Some(t) = self.toasts.iter_mut().find(|t| t.id == id) {
            t.action = Some(super::model::ToastAction {
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

    fn perform_action(action: &super::model::ToastAction) {
        match &action.kind {
            super::model::ToastActionKind::RevealInFolder(path) => {
                crate::platform::open_containing_folder(path);
            }
        }
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

    /// 收起进行中任务：记 collapsed 防复活，并自动 resume 已暂停任务。
    /// show_toasts 的 dismiss 分支调用（单测亦走此链路）。
    /// 防卡死原因：收起后卡面消失，若仍 paused 则任务永停且无 UI 可恢复。
    pub(crate) fn collapse_for_dismiss(&mut self, id: u64) {
        let is_running = self.toasts.iter().any(|t| t.id == id && t.source.is_some());
        if !is_running {
            return;
        }
        self.collapsed.insert(id);
        // 先取 flag 再清（与 show_toasts 内联逻辑同语义）
        let pause_flag = self
            .toasts
            .iter()
            .find(|t| t.id == id)
            .and_then(super::model::resolve_pause_toast);
        if let Some(p) = pause_flag {
            p.store(false, std::sync::atomic::Ordering::Relaxed);
        }
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
            // 列表关闭瞬间：屏幕上所有卡全部走退场动画；进行中顺带记 collapsed 防复活+resume
            if was_open && !self.center_open {
                let ids: Vec<u64> = self
                    .toasts
                    .iter()
                    .filter(|t| t.leaving_since.is_none())
                    .map(|t| t.id)
                    .collect();
                for id in ids {
                    self.collapse_for_dismiss(id);
                    self.dismiss_toast(id);
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
            for t in self.toasts.iter_mut() {
                if t.hovered
                    && t.leaving_since.is_none()
                    && let Some(at) = t.collapse_at
                {
                    t.collapse_at = Some(at + dt);
                }
            }
        }
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
        // 实测高度只保留还存在的卡，防 map 无限涨（量级小，直接查）。
        self.card_h.retain(|id, _| {
            self.toasts.iter().any(|t| t.id == *id) || self.history.iter().any(|h| h.id == *id)
        });
        // 自动收起到期：走正常离开动画，只收浮动卡，历史保留；
        // 列表开着时跳过（deadline 自然过期不管它）
        let expired: Vec<u64> = if self.center_open {
            Vec::new()
        } else {
            self.toasts
                .iter()
                .filter(|t| t.leaving_since.is_none() && t.collapse_at.is_some_and(|at| at <= now))
                .map(|t| t.id)
                .collect()
        };
        for id in expired {
            self.dismiss_toast(id);
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
        // 有未到期的自动收起则准时唤醒（精度±500ms 内可接受，不另起 timer）
        if let Some(wait) = self
            .toasts
            .iter()
            .filter(|t| t.leaving_since.is_none())
            .filter_map(|t| t.collapse_at)
            .filter(|at| *at > now)
            .min()
            .and_then(|at| at.checked_duration_since(now))
        {
            ctx.request_repaint_after(wait.min(Duration::from_secs(3600)));
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

        // 预计算目标 y：toast 堆叠 vs 历史堆叠（按每张卡实测高度累加）
        let mut toast_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let ids: Vec<u64> = self.toasts.iter().rev().map(|t| t.id).collect();
            for (id, y) in ids
                .iter()
                .zip(stack_ys(&self.card_h, &ids, BOTTOM_PAD, GAP, EST_H))
            {
                toast_y_map.insert(*id, y);
            }
        }
        let mut history_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let ids: Vec<u64> = self.history.iter().rev().map(|h| h.id).collect();
            for (id, y) in ids
                .iter()
                .zip(stack_ys(&self.card_h, &ids, BOTTOM_PAD, GAP, EST_H))
            {
                history_y_map.insert(*id, y);
            }
        }

        let mut to_dismiss: Vec<u64> = Vec::new();
        for idx in (0..self.toasts.len()).rev() {
            let tid = self.toasts[idx].id;
            let target_y = if self.center_open {
                // 重排至历史中的位置
                history_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD)
            } else {
                toast_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD)
            };
            if target_y > max_h + self.measured_h(tid, EST_H) {
                self.toasts[idx].hovered = false;
                continue;
            }
            // 非线性 y 插值：已有通知平滑重排
            let y_off =
                ctx.animate_value_with_time(egui::Id::new(("notif_y", tid)), target_y, 0.35);
            let is_leaving = self.toasts[idx].leaving_since.is_some();
            // 打开列表时已存在的 toast 不重新飞入，仅重排；离开时仍飞出
            let x_off = if self.center_open && !is_leaving {
                0.0
            } else {
                super::anim::fly_anim(&self.toasts[idx])
            };
            let cancel_flag = super::model::resolve_cancel_toast(&self.toasts[idx]);
            let action_opt = self.toasts[idx].action.clone();
            let mut outcome = super::model::CardOutcome::default();
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
                .show(ctx, |ui| {
                    outcome = super::card::toast_card(
                        ui,
                        &self.toasts[idx],
                        CARD_W,
                        0.0,
                        1.0,
                        !self.center_open,
                    );
                    ui.min_rect().height()
                });
            self.card_h.insert(tid, area_resp.inner);
            self.toasts[idx].hovered = outcome.hovered;
            if outcome.cancel {
                // stop：只置 flag + 中止态，卡片留着等 abort 确认（见 C），不进退场
                if let Some(c) = cancel_flag {
                    c.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.toasts[idx].cancelling = true;
            } else if outcome.dismiss {
                // 进行中任务收起：记 collapsed 防复活 + 自动 resume（防卡死，见方法注释）；
                // 静态卡直接退场（helper 内判 source.is_some() 后直接返回）。
                self.collapse_for_dismiss(tid);
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
                self.dismiss_toast(id);
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

        // 预计算历史目标 y（最新在底部，按每张卡实测高度累加）
        let mut history_y_map: HashMap<u64, f32> = HashMap::new();
        {
            let ids: Vec<u64> = self.history.iter().rev().map(|h| h.id).collect();
            for (id, y) in ids
                .iter()
                .zip(stack_ys(&self.card_h, &ids, BOTTOM_PAD, GAP, EST_H))
            {
                history_y_map.insert(*id, y);
            }
        }

        // 仅渲染历史中不在当前 toast 的那些（已在屏幕的由 show_toasts 负责重排）
        for idx in (0..self.history.len()).rev() {
            if self.toasts.iter().any(|t| t.id == self.history[idx].id) {
                continue;
            }
            let tid = self.history[idx].id;
            let target_y = history_y_map.get(&tid).copied().unwrap_or(BOTTOM_PAD);
            if target_y > max_h + self.measured_h(tid, EST_H) {
                continue;
            }
            let y_off =
                ctx.animate_value_with_time(egui::Id::new(("notif_y", tid)), target_y, 0.35);
            // 打开：无停顿直接从右侧飞入；关闭：入场的严格反向飞出
            let x_off = if closing {
                let closed_at = self.center_closed_at.unwrap_or_else(Instant::now);
                super::anim::exit_x(Instant::now().duration_since(closed_at).as_secs_f32())
            } else if let Some(opened_at) = self.center_opened_at {
                super::anim::enter_x(Instant::now().duration_since(opened_at).as_secs_f32())
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
                .show(ctx, |ui| {
                    super::card::history_card(ui, &self.history[idx], CARD_W);
                    ui.min_rect().height()
                });
            self.card_h.insert(tid, area_resp.inner);
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// 堆叠 y 累加：按 ids 顺序（调用方先排好，如最新在底则传 rev 后），逐项取实测高度，
/// 缺失 id 用 fallback。返回与 ids 等长对齐的 y。
fn stack_ys(
    heights: &HashMap<u64, f32>,
    ids: &[u64],
    bottom_pad: f32,
    gap: f32,
    fallback: f32,
) -> Vec<f32> {
    let mut cum: f32 = 0.0;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(bottom_pad + cum);
        cum += heights.get(id).copied().unwrap_or(fallback) + gap;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> egui::Context {
        egui::Context::default()
    }

    #[test]
    fn push_classifies_collapse_tier() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(5), Some(60));
        let ok = n.success("t", "m");
        let err = n.error("t", "m");
        let t_ok = n.toasts.iter().find(|t| t.id == ok).unwrap();
        let t_err = n.toasts.iter().find(|t| t.id == err).unwrap();
        assert!(t_ok.collapse_at.is_some());
        assert!(t_err.collapse_at.is_some());
        // 可操作档晚于完成档
        assert!(t_err.collapse_at.unwrap() > t_ok.collapse_at.unwrap());
    }

    #[test]
    fn never_means_sticky() {
        let mut n = Notifications::new();
        n.set_collapse_durations(None, None);
        let id = n.success("t", "m");
        assert!(
            n.toasts
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .collapse_at
                .is_none()
        );
        n.tick(&ctx());
        assert!(
            n.toasts
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .leaving_since
                .is_none()
        );
    }

    #[test]
    fn expired_toast_starts_leaving_but_keeps_history() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(0), Some(0));
        let id = n.success("t", "m");
        // 0 秒档下帧即到期
        std::thread::sleep(Duration::from_millis(2));
        n.tick(&ctx());
        let t = n.toasts.iter().find(|t| t.id == id).unwrap();
        assert!(t.leaving_since.is_some());
        assert!(n.history.iter().any(|h| h.id == id));
    }

    #[test]
    fn ensure_does_not_collapse_running_task() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "t".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(0), Some(0));
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        std::thread::sleep(Duration::from_millis(2));
        n.tick(&ctx());
        // 进行中不计时，不会离开
        assert!(
            n.toasts
                .iter()
                .find(|t| t.id == LOADING_PROGRESS_ID)
                .unwrap()
                .leaving_since
                .is_none()
        );
    }

    #[test]
    fn hover_pauses_collapse_deadline() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(60), Some(60));
        let id = n.success("t", "m");
        let before = n.toasts.iter().find(|t| t.id == id).unwrap().collapse_at;
        n.tick(&ctx()); // 首 tick 只记录 last_tick，不顺延
        std::thread::sleep(Duration::from_millis(5));
        n.toasts.iter_mut().find(|t| t.id == id).unwrap().hovered = true;
        n.tick(&ctx());
        let after = n.toasts.iter().find(|t| t.id == id).unwrap().collapse_at;
        assert!(after.unwrap() > before.unwrap());
        // 取消悬停：deadline 冻结不再顺延
        n.toasts.iter_mut().find(|t| t.id == id).unwrap().hovered = false;
        std::thread::sleep(Duration::from_millis(2));
        n.tick(&ctx());
        let still = n.toasts.iter().find(|t| t.id == id).unwrap().collapse_at;
        assert_eq!(still, after);
    }

    #[test]
    fn hover_does_not_extend_leaving_toast() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(60), Some(60));
        let id = n.success("t", "m");
        n.dismiss_toast(id);
        n.toasts.iter_mut().find(|t| t.id == id).unwrap().hovered = true;
        let before = n.toasts.iter().find(|t| t.id == id).unwrap().collapse_at;
        n.tick(&ctx());
        n.tick(&ctx());
        let after = n.toasts.iter().find(|t| t.id == id).unwrap().collapse_at;
        assert_eq!(after, before);
    }

    #[test]
    fn complete_export_uses_actionable_tier() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(5), Some(60));
        n.ensure_progress(
            EXPORT_PROGRESS_ID,
            ToastKind::Info,
            Arc::new(crate::file_loader::LoadToastSource {
                progress: yinhe_editor_core::progress::new_shared(),
                cancel: None,
            }),
        );
        n.complete_progress(EXPORT_PROGRESS_ID, ToastKind::Success, "done", "f");
        let t = n
            .toasts
            .iter()
            .find(|t| t.id == EXPORT_PROGRESS_ID)
            .unwrap();
        // 60 秒档：剩余远大于 5 秒档上限
        assert!(
            t.collapse_at.unwrap() > Instant::now() + Duration::from_secs(50),
            "export complete should use actionable tier"
        );
    }

    #[test]
    fn disabled_push_does_not_create_card_or_history() {
        let mut n = Notifications::new();
        n.set_enabled(false);
        let hist_before = n.history.len();
        let id = n.success("t", "m");
        assert_eq!(id, 0);
        assert!(n.toasts.is_empty());
        assert_eq!(n.history.len(), hist_before);
        n.info("a", "b");
        n.warning("a", "b");
        n.error("a", "b");
        assert!(n.toasts.is_empty());
        assert_eq!(n.history.len(), hist_before);
    }

    #[test]
    fn disabled_ensure_does_not_create_progress_card() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "t".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.set_enabled(false);
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        assert!(!n.has_progress(LOADING_PROGRESS_ID));
        assert!(n.toasts.is_empty());
        assert!(n.history.is_empty());
        // complete/fail 回退也不建卡
        n.complete_progress(LOADING_PROGRESS_ID, ToastKind::Success, "d", "m");
        n.fail_progress(SAVE_PROGRESS_ID, "f", "m");
        assert!(n.toasts.is_empty());
        assert!(n.history.is_empty());
    }

    #[test]
    fn disabled_tick_clears_toasts_but_keeps_history() {
        let mut n = Notifications::new();
        let id = n.success("t", "m");
        assert!(n.toasts.iter().any(|t| t.id == id));
        n.set_enabled(false);
        // 关闭走正常退场：卡仍在但 leaving 已起算，历史保留
        assert!(!n.toasts.is_empty());
        assert!(
            n.toasts
                .iter()
                .find(|t| t.id == id)
                .unwrap()
                .leaving_since
                .is_some()
        );
        n.tick(&ctx());
        assert!(!n.toasts.is_empty());
        assert!(n.history.iter().any(|h| h.id == id));
        // 退场动画播完后卡才移除
        std::thread::sleep(Duration::from_millis(330));
        n.tick(&ctx());
        assert!(n.toasts.is_empty());
        assert!(n.history.iter().any(|h| h.id == id));
    }

    #[test]
    fn reenable_restores_normal_push() {
        let mut n = Notifications::new();
        n.set_enabled(false);
        n.success("t", "m");
        assert!(n.toasts.is_empty());
        n.set_enabled(true);
        let id = n.success("t", "m");
        assert!(n.toasts.iter().any(|t| t.id == id));
        assert!(n.history.iter().any(|h| h.id == id));
        n.tick(&ctx());
        assert!(n.toasts.iter().any(|t| t.id == id));
    }

    #[test]
    fn collapsed_ensure_updates_history_without_card() {
        use std::sync::Arc;
        struct S(&'static str);
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                self.0.into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v1")));
        assert!(n.has_progress(LOADING_PROGRESS_ID));
        // 模拟用户点 X 收起进行中任务
        n.collapsed.insert(LOADING_PROGRESS_ID);
        n.dismiss_toast(LOADING_PROGRESS_ID);
        std::thread::sleep(Duration::from_millis(330));
        n.tick(&ctx());
        assert!(!n.has_progress(LOADING_PROGRESS_ID));
        // 收起后 ensure 不建卡，只更新历史
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v2")));
        assert!(!n.has_progress(LOADING_PROGRESS_ID));
        let Some(&hist_id) = n.live_hist.get(&LOADING_PROGRESS_ID) else {
            panic!("live mapping missing");
        };
        let Some(h) = n.history.iter().find(|h| h.id == hist_id) else {
            panic!("live history missing");
        };
        // 历史渲染走 live source，标题应为新任务
        let Some(src) = h.source.as_ref() else {
            panic!("live source missing");
        };
        assert_eq!(src.title(), "v2");
        // 任务结束清收起标记，下个任务恢复建卡
        n.prune_history(LOADING_PROGRESS_ID);
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v3")));
        assert!(n.has_progress(LOADING_PROGRESS_ID));
    }

    #[test]
    fn prune_history_clears_frozen_entry_and_collapsed() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "t".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        n.collapsed.insert(EXPORT_PROGRESS_ID);
        let Some(&hist_id) = n.live_hist.get(&EXPORT_PROGRESS_ID) else {
            panic!("live mapping missing");
        };
        assert!(n.history.iter().any(|h| h.id == hist_id));
        n.prune_history(EXPORT_PROGRESS_ID);
        assert!(!n.history.iter().any(|h| h.id == hist_id));
        assert!(!n.collapsed.contains(&EXPORT_PROGRESS_ID));
        assert!(!n.live_hist.contains_key(&EXPORT_PROGRESS_ID));
    }

    #[test]
    fn fixed_id_two_runs_keep_separate_done_history_and_single_float() {
        use std::sync::Arc;
        struct S(&'static str);
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                self.0.into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        // 第一轮
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v1")));
        assert_eq!(
            n.toasts
                .iter()
                .filter(|t| t.id == LOADING_PROGRESS_ID)
                .count(),
            1
        );
        n.complete_progress(LOADING_PROGRESS_ID, ToastKind::Success, "done1", "a.mid");
        assert_eq!(
            n.toasts
                .iter()
                .filter(|t| t.id == LOADING_PROGRESS_ID)
                .count(),
            1
        );
        assert!(!n.live_hist.contains_key(&LOADING_PROGRESS_ID));
        // 第二轮：浮动卡复用同一槽位，历史新建一条
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v2")));
        assert_eq!(
            n.toasts
                .iter()
                .filter(|t| t.id == LOADING_PROGRESS_ID)
                .count(),
            1
        );
        assert_eq!(n.toasts.len(), 1);
        n.complete_progress(LOADING_PROGRESS_ID, ToastKind::Success, "done2", "b.mid");
        // 浮动卡始终一张，历史两条独立 done
        assert_eq!(
            n.toasts
                .iter()
                .filter(|t| t.id == LOADING_PROGRESS_ID)
                .count(),
            1
        );
        assert_eq!(n.history.len(), 2);
        assert_ne!(n.history[0].id, n.history[1].id);
        assert_eq!(n.history[0].title, "done1");
        assert_eq!(n.history[1].title, "done2");
        for h in &n.history {
            assert_eq!(h.progress, Some(1.0));
            assert_eq!(h.progress_label, "已完成");
            assert!(h.source.is_none());
        }
        // prune 无 live 可清，不碰已封存
        n.prune_history(LOADING_PROGRESS_ID);
        assert_eq!(n.history.len(), 2);
        // 新一轮 live 可被 prune，只清 live
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S("v3")));
        assert_eq!(n.history.len(), 3);
        let Some(&live_id) = n.live_hist.get(&LOADING_PROGRESS_ID) else {
            panic!("live mapping missing");
        };
        n.prune_history(LOADING_PROGRESS_ID);
        assert_eq!(n.history.len(), 2);
        assert!(!n.history.iter().any(|h| h.id == live_id));
        assert_eq!(n.history[0].title, "done1");
        assert_eq!(n.history[1].title, "done2");
    }

    #[test]
    fn abort_progress_in_place_updates_toast_and_history() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "正在导出".into()
            }
            fn message(&self) -> String {
                "渲染中".into()
            }
            fn fraction(&self) -> f32 {
                0.64
            }
            fn detail(&self) -> String {
                "渲染中".into()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        // 模拟 stop：中止态
        n.toasts
            .iter_mut()
            .find(|t| t.id == EXPORT_PROGRESS_ID)
            .unwrap()
            .cancelling = true;
        let id = n.abort_progress(EXPORT_PROGRESS_ID, "已中止", "out.wav");
        assert_eq!(id, EXPORT_PROGRESS_ID);
        let t = n
            .toasts
            .iter()
            .find(|t| t.id == EXPORT_PROGRESS_ID)
            .unwrap();
        assert_eq!(t.kind, ToastKind::Warning);
        assert_eq!(t.progress, Some(0.64));
        assert_eq!(t.progress_label, "已中止");
        assert!(t.source.is_none());
        assert!(!t.cancelling);
        assert!(t.collapse_at.is_some());
        // 历史一任务一条：封存后映射移除，历史里仅一条已中止（小 id，非固定 id）
        assert!(!n.live_hist.contains_key(&EXPORT_PROGRESS_ID));
        assert_eq!(n.history.len(), 1);
        let h = &n.history[0];
        assert_eq!(h.progress_label, "已中止");
        assert!(h.source.is_none());
    }

    #[test]
    fn abort_progress_rebuilds_when_only_history_exists() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "正在导出".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.4
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        // 模拟收起后卡被清理，仅历史残留
        n.collapsed.insert(EXPORT_PROGRESS_ID);
        n.dismiss_toast(EXPORT_PROGRESS_ID);
        std::thread::sleep(Duration::from_millis(330));
        n.tick(&ctx());
        assert!(!n.has_progress(EXPORT_PROGRESS_ID));
        let id = n.abort_progress(EXPORT_PROGRESS_ID, "已中止", "out.wav");
        assert_eq!(id, EXPORT_PROGRESS_ID);
        assert!(n.has_progress(EXPORT_PROGRESS_ID));
        assert!(!n.collapsed.contains(&EXPORT_PROGRESS_ID));
        let t = n
            .toasts
            .iter()
            .find(|t| t.id == EXPORT_PROGRESS_ID)
            .unwrap();
        assert_eq!(t.progress_label, "已中止");
    }

    #[test]
    fn abort_progress_falls_back_to_push_when_nothing_exists() {
        let mut n = Notifications::new();
        let id = n.abort_progress(EXPORT_PROGRESS_ID, "已中止", "out.wav");
        assert_ne!(id, 0);
        assert_ne!(id, EXPORT_PROGRESS_ID);
        let t = n.toasts.iter().find(|t| t.id == id).unwrap();
        assert_eq!(t.kind, ToastKind::Warning);
        assert_eq!(t.title, "已中止");
    }

    /// 收起已暂停的卡必须自动 resume（ExportToastSource 级真 flag）。
    #[test]
    fn collapse_paused_task_auto_resumes() {
        use std::sync::atomic::Ordering;
        let pause_flag = Arc::new(AtomicBool::new(true));
        let src = Arc::new(crate::app::export_state::ExportToastSource {
            progress: yinhe_audio::export::ExportProgress::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::clone(&pause_flag),
        });
        let mut n = Notifications::new();
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, src);
        assert!(pause_flag.load(Ordering::Relaxed));
        // 走与 show_toasts 收起分支同一链路
        n.collapse_for_dismiss(EXPORT_PROGRESS_ID);
        assert!(
            !pause_flag.load(Ordering::Relaxed),
            "collapse must resume paused task"
        );
        assert!(n.collapsed.contains(&EXPORT_PROGRESS_ID));
    }

    /// 关闭通知总开关必须自动 resume 已暂停任务（否则永停且无 UI 可恢复）。
    #[test]
    fn disable_notifications_auto_resumes_paused() {
        use std::sync::atomic::Ordering;
        let pause_flag = Arc::new(AtomicBool::new(true));
        let src = Arc::new(crate::app::export_state::ExportToastSource {
            progress: yinhe_audio::export::ExportProgress::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            pause: Arc::clone(&pause_flag),
        });
        let mut n = Notifications::new();
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, src);
        assert!(pause_flag.load(Ordering::Relaxed));
        n.set_enabled(false);
        assert!(
            !pause_flag.load(Ordering::Relaxed),
            "set_enabled(false) must resume paused task"
        );
    }

    #[test]
    fn stack_ys_equal_heights() {
        let heights: HashMap<u64, f32> = [(1, 80.0), (2, 80.0)].into_iter().collect();
        assert_eq!(
            stack_ys(&heights, &[1, 2], 48.0, 8.0, 110.0),
            vec![48.0, 136.0]
        );
    }

    #[test]
    fn stack_ys_mixed_heights_with_fallback() {
        let heights: HashMap<u64, f32> = [(1, 60.0), (3, 90.0)].into_iter().collect();
        // id=2 缺失用 fallback=110：48, 48+60+8=116, 116+110+8=234
        assert_eq!(
            stack_ys(&heights, &[1, 2, 3], 48.0, 8.0, 110.0),
            vec![48.0, 116.0, 234.0]
        );
    }

    #[test]
    fn stack_ys_empty() {
        let heights: HashMap<u64, f32> = HashMap::new();
        assert!(stack_ys(&heights, &[], 48.0, 8.0, 110.0).is_empty());
    }

    #[test]
    fn center_open_skips_auto_collapse() {
        let mut n = Notifications::new();
        n.set_collapse_durations(Some(0), Some(0));
        let id = n.success("t", "m");
        std::thread::sleep(Duration::from_millis(2));
        // 列表开着时到期也不收
        n.center_open = true;
        n.tick(&ctx());
        let Some(t) = n.toasts.iter().find(|t| t.id == id) else {
            panic!("toast missing");
        };
        assert!(t.leaving_since.is_none());
        // 关着时同条件会收（对照）
        n.center_open = false;
        // 关闭边沿本身就会收全部，这里仅断言 leaving 已起算
        n.tick(&ctx());
        let Some(t) = n.toasts.iter().find(|t| t.id == id) else {
            panic!("toast missing");
        };
        assert!(t.leaving_since.is_some());
    }

    #[test]
    fn center_close_edge_dismisses_all() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "t".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.set_collapse_durations(None, None);
        let static_id = n.success("s", "m");
        n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        // 开列表：先 tick 一次把 prev 对齐为 true
        n.center_open = true;
        n.tick(&ctx());
        assert!(
            n.toasts.iter().all(|t| t.leaving_since.is_none()),
            "open list must not dismiss"
        );
        // 关列表边沿：全部 leaving（含进行中；collapsed 顺带标记防复活）
        n.center_open = false;
        n.tick(&ctx());
        for t in &n.toasts {
            assert!(
                t.leaving_since.is_some(),
                "close edge must dismiss id={}",
                t.id
            );
        }
        let Some(st) = n.toasts.iter().find(|t| t.id == static_id) else {
            panic!("static toast missing");
        };
        assert!(st.leaving_since.is_some());
        assert!(n.collapsed.contains(&EXPORT_PROGRESS_ID));
        // show_close 取反逻辑：渲染层重（需真 egui 上下文量按钮），只测状态机；
        // show_toasts 以 !center_open 传 show_close，手动验证：列表开无 X、可 stop，关后有 X。
    }

    #[test]
    fn center_open_edge_clears_unread() {
        let mut n = Notifications::new();
        // 关着时 push 产生未读
        n.center_open = false;
        n.tick(&ctx());
        let _ = n.success("a", "b");
        assert_eq!(n.unread_count(), 1);
        // 开列表边沿清零
        n.center_open = true;
        n.tick(&ctx());
        assert_eq!(n.unread_count(), 0);
        assert!(!n.has_unread());
    }

    #[test]
    fn center_open_push_and_ensure_stay_read() {
        use std::sync::Arc;
        struct S;
        impl super::super::model::ProgressSource for S {
            fn title(&self) -> String {
                "t".into()
            }
            fn message(&self) -> String {
                String::new()
            }
            fn fraction(&self) -> f32 {
                0.5
            }
            fn detail(&self) -> String {
                String::new()
            }
            fn cancel(&self) -> Option<Arc<AtomicBool>> {
                None
            }
        }
        let mut n = Notifications::new();
        n.center_open = true;
        n.tick(&ctx());
        assert_eq!(n.unread_count(), 0);
        // 开着时 push 不产生未读
        let before = n.unread_count();
        let _ = n.success("c", "d");
        assert_eq!(n.unread_count(), before);
        // 开着时 ensure 新任务不产生未读
        n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(S));
        assert_eq!(n.unread_count(), before);
        assert!(!n.has_unread());
    }
}
