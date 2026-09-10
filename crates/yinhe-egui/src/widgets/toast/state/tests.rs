use std::sync::{Arc, atomic::AtomicBool};

use super::super::kind::ToastKind;
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

#[test]
fn y_for_retarget_keeps_continuity() {
    let mut n = Notifications::new();
    let t0 = Instant::now();
    // 首帧直接落到目标，无跳变
    let y0 = n.y_for(7, 100.0, t0);
    assert!((y0 - 100.0).abs() < 1e-6);
    // 中途（100ms，约 0.285 进度）取显示值
    let t1 = t0 + Duration::from_millis(100);
    let mid = n.y_for(7, 100.0, t1);
    assert!(mid >= 100.0 - 1e-6, "single target must stay, got {mid}");
    // 目标突变为 200：本帧返回旧显示值，不断裂
    let t2 = t0 + Duration::from_millis(150);
    let snapshot = match n.y_anim.get(&7).copied() {
        Some(a) => a,
        None => YAnim {
            from: 100.0,
            to: 100.0,
            t0,
        },
    };
    let before = y_anim_value(&snapshot, t2);
    let ret = n.y_for(7, 200.0, t2);
    assert!(
        (ret - before).abs() < 1e-4,
        "retarget must continue from display, ret={ret} before={before}"
    );
    // 新记录 from 即旧显示值
    let Some(a) = n.y_anim.get(&7) else {
        panic!("y_anim missing");
    };
    assert!((a.from - before).abs() < 1e-4);
    assert!((a.to - 200.0).abs() < 1e-6);
}

/// 回归：快速滚动时按「显示位置」而非「目标位置」裁卡。
/// 目标已出带但 y 动画还在带内时，卡片必须继续渲染（否则未滑出即消失）。
#[test]
fn cull_uses_displayed_y_not_target() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut n = Notifications::new();
    n.center_open = true;
    n.prev_center_open = true;
    let mut tid = 0;
    for i in 0..10 {
        tid = n.success(format!("t{i}"), "m");
    }
    for h in n.history.clone() {
        n.card_h.insert(h.id, 110.0);
    }
    // 哨兵高度：被裁则保持原值，渲染则被实测覆盖
    const SENTINEL: f32 = 1234.5;
    n.card_h.insert(tid, SENTINEL);
    // 目标位置：最大滚动后远出下边界
    n.center_scroll = 5000.0;
    // 显示位置：仍在可见带内（卡坐在底边上），动画尚未滑出
    n.y_anim.insert(
        tid,
        YAnim {
            from: 48.0,
            to: -9999.0,
            t0: Instant::now(),
        },
    );
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        n.show_toasts(ui.ctx());
    });
    out.textures_delta.clear();
    let h = n.card_h.get(&tid).copied().unwrap_or(SENTINEL);
    assert!(
        (h - SENTINEL).abs() > f32::EPSILON,
        "目标出带但显示位置仍在带内时被提前裁掉（提前消失 bug）"
    );
}

#[test]
fn scroll_clamp_bounds() {
    let max = 300.0;
    // 上下界 clamp（与滚轮/拖动同公式）
    assert!(((-50.0_f32).clamp(0.0, max) - 0.0).abs() < 1e-6);
    assert!(((400.0_f32).clamp(0.0, max) - 300.0).abs() < 1e-6);
    assert!(((150.0_f32).clamp(0.0, max) - 150.0).abs() < 1e-6);
    // 滚轮公式：center_scroll + dy（上滑 dy>0 看旧消息，scroll 增大）
    let scrolled = (100.0 + (-30.0_f32)).clamp(0.0, max);
    assert!((scrolled - 70.0).abs() < 1e-6);
    // 拖拽公式：center_scroll - drag_dy * max / travel（下拽看新消息，scroll 减小）
    let dragged = (100.0_f32 - 20.0 * 300.0 / 200.0).clamp(0.0, max);
    assert!((dragged - 70.0).abs() < 1e-6);
}

#[test]
fn update_center_scroll_sticks_and_updates_max() {
    let mut n = Notifications::new();
    // 造 3 条历史，每条实测 100，GAP=8：total=100*3+8*2=316
    let _ = n.success("a", "1");
    let _ = n.success("b", "2");
    let _ = n.success("c", "3");
    for h in n.history.clone() {
        n.card_h.insert(h.id, 100.0);
    }
    // viewport 高 200：visible=200-48-24=128，max=316-128=188
    n.center_scroll = 188.0;
    n.center_scroll_max = 188.0;
    n.update_center_scroll(200.0, 48.0, 8.0, 110.0);
    assert!((n.center_scroll_max - 188.0).abs() < 1e-4);
    // 新增一条变高到 total=424，max=296，底部附近应吸到新 max
    let _ = n.success("d", "4");
    let Some(last) = n.history.last().cloned() else {
        panic!("history missing");
    };
    n.card_h.insert(last.id, 100.0);
    n.update_center_scroll(200.0, 48.0, 8.0, 110.0);
    assert!((n.center_scroll_max - 296.0).abs() < 1e-4);
    assert!((n.center_scroll - 296.0).abs() < 1e-4);
}
