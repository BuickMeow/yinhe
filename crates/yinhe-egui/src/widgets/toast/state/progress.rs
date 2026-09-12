use std::sync::{Arc, atomic::AtomicBool};
use std::time::Instant;

use super::super::kind::ToastKind;
use super::super::model::{ProgressOutcome, ProgressSource};
use super::Notifications;

impl Notifications {
    /// 确保进度卡存在：同一 key（如 LOADING_PROGRESS_ID）复用同一 id。
    /// 只建卡/换数据源（Arc 交换，无文案拷贝），进度文案渲染时 pull。
    /// 调用方每帧调也无妨；用户收起（off-screen）或退场中时只更新数据、不弹出。
    pub fn ensure_progress(&mut self, id: u64, kind: ToastKind, source: Arc<dyn ProgressSource>) {
        if !self.enabled {
            return;
        }
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            if n.leaving_since.is_some() {
                return; // 退场中不复活
            }
            // 上一任务已结束（source 已清空）：重新入场并换上本次标题
            if n.source.is_none() {
                n.created = Instant::now();
                n.title = source.title();
            }
            n.kind = kind;
            n.source = Some(source);
            // 进行中不计时，完成时才起算
            n.collapse_at = None;
            n.cancelling = false;
            return;
        }
        let title = source.title();
        self.insert(id, kind, title, String::new(), Some(source), None);
    }

    /// 进度任务收尾：原地切换完成态（避免高度跳变）并换独立 id 转正，
    /// 腾出任务槽位——下次同类操作新建条目，多次操作各自保留完成通知。
    /// 已收起/离屏的重新弹出；失败/中止按可操作档计时，普通完成按完成档。
    /// `detail` 覆盖默认详情文案（如加载耗时）。返回完成通知的 id（供 set_action 用）。
    pub fn finish_progress(
        &mut self,
        id: u64,
        outcome: ProgressOutcome,
        title: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
    ) -> u64 {
        if !self.enabled {
            return id;
        }
        let title = title.into();
        let message = message.into();
        let (kind, default_detail, dur) = match outcome {
            ProgressOutcome::Completed => (ToastKind::Success, "已完成", self.collapse_secs),
            ProgressOutcome::Failed => (ToastKind::Error, "失败", self.action_collapse_secs),
            ProgressOutcome::Aborted => (ToastKind::Warning, "已中止", self.action_collapse_secs),
        };
        let deadline = self.collapse_deadline(dur);
        if !self.items.iter().any(|n| n.id == id) {
            // 条目不存在（被历史上限裁掉等）：回退普通通知
            let new_id = self.alloc_id();
            return self.insert(new_id, kind, title, message, None, deadline);
        }
        // 先分配转正 id，再进入条目可变借用（alloc_id 需要 &mut self）
        let promoted = self.alloc_id();
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            // 完成满格；失败无进度条；中止保留中断时刻快照
            let fraction = match outcome {
                ProgressOutcome::Completed => Some(1.0),
                ProgressOutcome::Failed => None,
                ProgressOutcome::Aborted => n.snapshot_fraction(),
            };
            n.kind = kind;
            n.title = title;
            n.message = message;
            n.progress = fraction;
            n.progress_label = detail.unwrap_or_else(|| default_detail.to_string());
            n.cancel = None;
            n.source = None;
            n.collapse_at = deadline;
            n.cancelling = false;
            if !n.on_screen {
                // 已收起/离屏：重新入场
                n.created = Instant::now();
                n.on_screen = true;
            }
            n.leaving_since = None;
            n.id = promoted;
        }
        // 动画/实测高度跟随换 id，避免完成瞬间位置跳变
        if let Some(h) = self.card_h.remove(&id) {
            self.card_h.insert(promoted, h);
        }
        if let Some(a) = self.y_anim.remove(&id) {
            self.y_anim.insert(promoted, a);
        }
        self.prune_overflow();
        promoted
    }

    /// 给浮动卡挂操作按钮（如“打开文件夹”）。
    /// 有按钮即可操作，计时自动升为可操作档。
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
        if let Some(n) = self.items.iter_mut().find(|n| n.id == id) {
            n.action = Some(super::super::model::ToastAction {
                label: label.into(),
                kind,
                icon,
            });
            n.collapse_at = deadline;
        }
    }

    pub fn is_leaving(&self, id: u64) -> bool {
        self.get(id).is_some_and(|n| n.leaving_since.is_some())
    }

    pub fn get_cancel_flag(&self, id: u64) -> Option<Arc<AtomicBool>> {
        self.get(id).and_then(|n| n.cancel_flag())
    }
}
