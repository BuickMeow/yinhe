use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use super::super::kind::ToastKind;
use super::super::model::{ProgressOutcome, ProgressSource};
use super::*;

struct Src {
    fraction: f32,
    cancel: Option<Arc<AtomicBool>>,
    pause: Option<Arc<AtomicBool>>,
}

impl Src {
    fn new(fraction: f32) -> Self {
        Self {
            fraction,
            cancel: Some(Arc::new(AtomicBool::new(false))),
            pause: None,
        }
    }

    fn with_pause(fraction: f32) -> Self {
        Self {
            fraction,
            cancel: Some(Arc::new(AtomicBool::new(false))),
            pause: Some(Arc::new(AtomicBool::new(false))),
        }
    }
}

impl ProgressSource for Src {
    fn title(&self) -> String {
        "任务".to_string()
    }
    fn fraction(&self) -> f32 {
        self.fraction
    }
    fn cancel(&self) -> Option<Arc<AtomicBool>> {
        self.cancel.clone()
    }
    fn pause(&self) -> Option<Arc<AtomicBool>> {
        self.pause.clone()
    }
}

fn src(fraction: f32) -> Arc<dyn ProgressSource> {
    Arc::new(Src::new(fraction))
}

fn find<'a>(n: &'a Notifications, id: u64) -> &'a Notification {
    n.items.iter().find(|x| x.id == id).expect("entry exists")
}

#[test]
fn push_replaces_previous_result() {
    let mut n = Notifications::new();
    let a = n.push(ToastKind::Success, "a", "1");
    let b = n.push(ToastKind::Info, "b", "2");
    assert_ne!(a, b);
    assert_eq!(n.items.len(), 1, "结果通知只保留最新一条");
    assert_eq!(find(&n, b).title, "b");
    assert_eq!(n.primary, Some(b));
}

#[test]
fn success_expires_after_ttl() {
    let mut n = Notifications::new();
    let id = n.success("a", "1");
    n.items[0].expires_at = Some(Instant::now() - Duration::from_secs(1));
    n.tick(Instant::now());
    assert!(n.items.is_empty(), "过期的结果通知应被清理");
    assert_eq!(n.primary, None);
    let _ = id;
}

#[test]
fn ensure_progress_reuses_id() {
    let mut n = Notifications::new();
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, src(0.2));
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, src(0.6));
    assert_eq!(n.items.len(), 1);
    assert_eq!(n.primary, Some(LOADING_PROGRESS_ID));
    assert!((find(&n, LOADING_PROGRESS_ID).snapshot_fraction().unwrap() - 0.6).abs() < 1e-6);
}

#[test]
fn finish_completed_promotes_to_result() {
    let mut n = Notifications::new();
    n.ensure_progress(SAVE_PROGRESS_ID, ToastKind::Info, src(0.5));
    let id = n.finish_progress(
        SAVE_PROGRESS_ID,
        ProgressOutcome::Completed,
        "保存完成",
        "a.yin",
        None,
    );
    assert_ne!(id, 0);
    assert_eq!(n.items.len(), 1);
    assert_eq!(n.primary, Some(id));
    let r = find(&n, id);
    assert_eq!(r.kind, ToastKind::Success);
    assert_eq!(r.progress, Some(1.0));
    assert!(r.source.is_none());
    assert!(r.expires_at.is_some());
}

#[test]
fn finish_failed_pushes_error_and_removes_task() {
    let mut n = Notifications::new();
    n.ensure_progress(SAVE_PROGRESS_ID, ToastKind::Info, src(0.5));
    let id = n.finish_progress(
        SAVE_PROGRESS_ID,
        ProgressOutcome::Failed,
        "保存失败",
        "disk full",
        None,
    );
    assert_eq!(id, 0);
    assert!(n.items.is_empty());
    let err = n.front_error().expect("error queued");
    assert_eq!(err.title, "保存失败");
    assert_eq!(err.message, "disk full");
}

#[test]
fn finish_aborted_keeps_fraction() {
    let mut n = Notifications::new();
    n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, src(0.42));
    let id = n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Aborted,
        "已中止",
        "mix.wav",
        None,
    );
    let r = find(&n, id);
    assert_eq!(r.kind, ToastKind::Warning);
    assert!((r.progress.unwrap() - 0.42).abs() < 1e-6);
}

#[test]
fn dismissed_task_not_primary_and_not_resurrected() {
    let mut n = Notifications::new();
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, src(0.1));
    n.dismiss(LOADING_PROGRESS_ID);
    assert!(n.is_dismissed(LOADING_PROGRESS_ID));
    assert_eq!(n.primary, None);
    // 后台仍在跑：ensure 只更新数据，不显示。
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, src(0.7));
    assert_eq!(n.primary, None);
    assert!((find(&n, LOADING_PROGRESS_ID).snapshot_fraction().unwrap() - 0.7).abs() < 1e-6);
    // 真正结束后移除。
    n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Aborted,
        "已中止",
        "",
        None,
    );
    assert!(n.get(LOADING_PROGRESS_ID).is_none());
}

#[test]
fn cancel_flag_resolves_live_source() {
    let mut n = Notifications::new();
    let s = Src::new(0.3);
    let flag = s.cancel.clone().unwrap();
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, Arc::new(s));
    let live = n.get_cancel_flag(LOADING_PROGRESS_ID).expect("live flag");
    assert!(!live.load(std::sync::atomic::Ordering::Relaxed));
    live.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(flag.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn pause_is_released_when_dismissed() {
    let mut n = Notifications::new();
    let s = Src::with_pause(0.3);
    let pause = s.pause.clone().unwrap();
    pause.store(true, std::sync::atomic::Ordering::Relaxed);
    n.ensure_progress(EXPORT_PROGRESS_ID, ToastKind::Info, Arc::new(s));
    assert_eq!(n.primary, Some(EXPORT_PROGRESS_ID));
    n.dismiss(EXPORT_PROGRESS_ID);
    assert!(!pause.load(std::sync::atomic::Ordering::Relaxed));
}

#[test]
fn report_error_dedups_consecutive() {
    let mut n = Notifications::new();
    n.report_error("打开失败", "not found");
    n.report_error("打开失败", "not found");
    assert_eq!(n.errors.len(), 1);
    n.report_error("打开失败", "permission denied");
    assert_eq!(n.errors.len(), 2);
}

#[test]
fn report_error_caps_queue() {
    let mut n = Notifications::new();
    for i in 0..(MAX_ERRORS + 5) {
        n.report_error(format!("e{i}"), "m");
    }
    assert_eq!(n.errors.len(), MAX_ERRORS);
    assert_eq!(n.errors.front().unwrap().title, "e5");
}

#[test]
fn set_action_attaches_to_result() {
    let mut n = Notifications::new();
    n.ensure_progress(MIDI_EXPORT_PROGRESS_ID, ToastKind::Info, src(0.9));
    let id = n.finish_progress(
        MIDI_EXPORT_PROGRESS_ID,
        ProgressOutcome::Completed,
        "完成",
        "a.mid",
        None,
    );
    n.set_action_with_icon(
        id,
        "打开文件夹",
        super::super::model::ToastActionKind::RevealInFolder(std::path::PathBuf::from(".")),
        None,
    );
    assert!(find(&n, id).action.is_some());
}

#[test]
fn primary_falls_back_to_active_task() {
    let mut n = Notifications::new();
    let result = n.success("完成", "");
    n.ensure_progress(LOADING_PROGRESS_ID, ToastKind::Info, src(0.1));
    assert_eq!(n.primary, Some(LOADING_PROGRESS_ID));
    n.dismiss(LOADING_PROGRESS_ID);
    assert_eq!(n.primary, None);
    n.tick(Instant::now());
    assert_eq!(n.primary, Some(result), "任务收起后回退到结果通知");
}

#[test]
fn failed_task_does_not_leave_result_toast() {
    let mut n = Notifications::new();
    n.ensure_progress(RESCALE_PROGRESS_ID, ToastKind::Info, src(0.5));
    n.finish_progress(
        RESCALE_PROGRESS_ID,
        ProgressOutcome::Failed,
        "缩放失败",
        "boom",
        None,
    );
    assert!(n.items.is_empty());
}
