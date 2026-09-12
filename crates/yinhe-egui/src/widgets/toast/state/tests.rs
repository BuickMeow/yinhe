use std::sync::{Arc, atomic::AtomicBool};
use std::time::{Duration, Instant};

use super::super::kind::ToastKind;
use super::super::model::{ProgressOutcome, ProgressSource};
use super::*;

fn ctx() -> egui::Context {
    egui::Context::default()
}

/// 测试用进度源：固定标题与进度。
struct FakeSource {
    title: &'static str,
    fraction: f32,
}

impl FakeSource {
    fn new(title: &'static str) -> Self {
        Self {
            title,
            fraction: 0.5,
        }
    }

    fn at(title: &'static str, fraction: f32) -> Self {
        Self { title, fraction }
    }
}

impl ProgressSource for FakeSource {
    fn title(&self) -> String {
        self.title.into()
    }
    fn message(&self) -> String {
        String::new()
    }
    fn fraction(&self) -> f32 {
        self.fraction
    }
    fn detail(&self) -> String {
        String::new()
    }
    fn cancel(&self) -> Option<Arc<AtomicBool>> {
        None
    }
}

fn find(n: &Notifications, id: u64) -> &Notification {
    match n.items.iter().find(|x| x.id == id) {
        Some(x) => x,
        None => panic!("notification {id} missing"),
    }
}

#[test]
fn push_classifies_collapse_tier() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(5), Some(60));
    let ok = n.success("t", "m");
    let err = n.error("t", "m");
    let t_ok = find(&n, ok);
    let t_err = find(&n, err);
    assert!(t_ok.collapse_at.is_some());
    assert!(t_err.collapse_at.is_some());
    // 可操作档晚于完成档
    assert!(t_err.collapse_at.unwrap() > t_ok.collapse_at.unwrap());
}

#[test]
fn single_source_of_truth_has_no_duplicate_history() {
    let mut n = Notifications::new();
    let id = n.success("t", "m");
    // 浮动卡与列表是同一份数据：只有一条
    assert_eq!(n.items.len(), 1);
    assert_eq!(n.items[0].id, id);
    assert!(n.items[0].on_screen);
}

#[test]
fn never_means_sticky() {
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    let id = n.success("t", "m");
    assert!(find(&n, id).collapse_at.is_none());
    n.tick(&ctx());
    assert!(find(&n, id).leaving_since.is_none());
}

#[test]
fn expired_toast_starts_leaving_but_keeps_entry() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(0), Some(0));
    let id = n.success("t", "m");
    // 0 秒档下帧即到期
    std::thread::sleep(Duration::from_millis(2));
    n.tick(&ctx());
    assert!(find(&n, id).leaving_since.is_some());
    // 退场播完翻 off-screen，条目仍在列表
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(!find(&n, id).on_screen);
    assert_eq!(n.items.len(), 1);
}

#[test]
fn ensure_does_not_collapse_running_task() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(0), Some(0));
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("t")),
    );
    std::thread::sleep(Duration::from_millis(2));
    n.tick(&ctx());
    // 进行中不计时，不会离开
    assert!(find(&n, LOADING_PROGRESS_ID).leaving_since.is_none());
}

#[test]
fn hover_pauses_collapse_deadline() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(60), Some(60));
    let id = n.success("t", "m");
    let before = find(&n, id).collapse_at;
    n.tick(&ctx()); // 首 tick 只记录 last_tick，不顺延
    std::thread::sleep(Duration::from_millis(5));
    n.items.iter_mut().find(|x| x.id == id).unwrap().hovered = true;
    n.tick(&ctx());
    let after = find(&n, id).collapse_at;
    assert!(after.unwrap() > before.unwrap());
    // 取消悬停：deadline 冻结不再顺延
    n.items.iter_mut().find(|x| x.id == id).unwrap().hovered = false;
    std::thread::sleep(Duration::from_millis(2));
    n.tick(&ctx());
    assert_eq!(find(&n, id).collapse_at, after);
}

#[test]
fn hover_does_not_extend_leaving_toast() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(60), Some(60));
    let id = n.success("t", "m");
    n.dismiss(id);
    n.items.iter_mut().find(|x| x.id == id).unwrap().hovered = true;
    let before = find(&n, id).collapse_at;
    n.tick(&ctx());
    n.tick(&ctx());
    assert_eq!(find(&n, id).collapse_at, before);
}

/// 完成默认走完成档；导出类卡由 set_action 升为可操作档（不再硬编码任务身份）。
#[test]
fn action_button_escalates_tier() {
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(5), Some(60));
    n.ensure_progress(
        EXPORT_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("正在导出")),
    );
    n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done",
        "f",
        None,
    );
    let before = find(&n, EXPORT_PROGRESS_ID).collapse_at.unwrap();
    assert!(before < Instant::now() + Duration::from_secs(10));
    // 挂操作按钮 → 自动升为可操作档
    n.set_action_with_icon(
        EXPORT_PROGRESS_ID,
        "打开文件夹",
        super::super::model::ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
        None,
    );
    let after = find(&n, EXPORT_PROGRESS_ID).collapse_at.unwrap();
    assert!(
        after > Instant::now() + Duration::from_secs(50),
        "actionable card should use actionable tier"
    );
}

#[test]
fn disabled_push_does_not_create_entry() {
    let mut n = Notifications::new();
    n.set_enabled(false);
    let id = n.success("t", "m");
    assert_eq!(id, 0);
    assert!(n.items.is_empty());
    n.push(ToastKind::Info, "a", "b");
    n.push(ToastKind::Warning, "a", "b");
    n.push(ToastKind::Error, "a", "b");
    assert!(n.items.is_empty());
}

#[test]
fn disabled_ensure_does_not_create_progress_card() {
    let mut n = Notifications::new();
    n.set_enabled(false);
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("t")),
    );
    assert!(n.items.is_empty());
    // finish 回退也不建卡
    n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "d",
        "m",
        None,
    );
    n.finish_progress(SAVE_PROGRESS_ID, ProgressOutcome::Failed, "f", "m", None);
    assert!(n.items.is_empty());
}

#[test]
fn disabled_tick_clears_cards_but_keeps_entries() {
    let mut n = Notifications::new();
    let id = n.success("t", "m");
    assert!(find(&n, id).on_screen);
    n.set_enabled(false);
    // 关闭走正常退场：卡仍在但 leaving 已起算，条目保留
    assert!(find(&n, id).leaving_since.is_some());
    n.tick(&ctx());
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(!find(&n, id).on_screen);
    assert_eq!(n.items.len(), 1);
}

#[test]
fn reenable_restores_normal_push() {
    let mut n = Notifications::new();
    n.set_enabled(false);
    n.success("t", "m");
    assert!(n.items.is_empty());
    n.set_enabled(true);
    let id = n.success("t", "m");
    assert!(find(&n, id).on_screen);
    n.tick(&ctx());
    assert!(find(&n, id).on_screen);
}

#[test]
fn collapsed_ensure_updates_source_without_card() {
    let mut n = Notifications::new();
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v1")),
    );
    assert!(find(&n, LOADING_PROGRESS_ID).on_screen);
    // 模拟用户点 X 收起进行中任务
    n.dismiss(LOADING_PROGRESS_ID);
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(!find(&n, LOADING_PROGRESS_ID).on_screen);
    // 收起后 ensure 不重建卡，只更新数据
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v2")),
    );
    assert!(!find(&n, LOADING_PROGRESS_ID).on_screen);
    let x = find(&n, LOADING_PROGRESS_ID);
    assert!(!x.on_screen);
    let Some(src) = x.source.as_ref() else {
        panic!("live source missing");
    };
    assert_eq!(src.title(), "v2");
    // 任务结束：离屏条目重新弹出，下个任务恢复建卡
    n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done",
        "",
        None,
    );
    assert!(find(&n, LOADING_PROGRESS_ID).on_screen);
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v3")),
    );
    assert!(find(&n, LOADING_PROGRESS_ID).on_screen);
    assert_eq!(
        find(&n, LOADING_PROGRESS_ID)
            .source
            .as_ref()
            .unwrap()
            .title(),
        "v3"
    );
}

#[test]
fn prune_overflow_keeps_screen_cards() {
    let mut n = Notifications::new();
    n.max_history = 2;
    let keep = n.success("keep", "m");
    for i in 0..3 {
        let id = n.success(format!("a{i}"), "m");
        n.dismiss(id);
    }
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(find(&n, keep).on_screen);
    assert_eq!(n.items.iter().filter(|x| !x.on_screen).count(), 2);
}

/// 固定 id 复用同一槽位：连续两轮任务原地更新同一条目，不再产生第二条历史。
#[test]
fn fixed_id_reuses_single_entry_across_runs() {
    let mut n = Notifications::new();
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v1")),
    );
    assert_eq!(n.items.len(), 1);
    n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done1",
        "a.mid",
        None,
    );
    // 第二轮：复用同一槽位
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v2")),
    );
    assert_eq!(n.items.len(), 1);
    n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done2",
        "b.mid",
        None,
    );
    assert_eq!(n.items.len(), 1);
    let x = find(&n, LOADING_PROGRESS_ID);
    assert_eq!(x.title, "done2");
    assert_eq!(x.progress, Some(1.0));
    assert_eq!(x.progress_label, "已完成");
    assert!(x.source.is_none());
}

#[test]
fn finish_aborted_snapshots_fraction_in_place() {
    let mut n = Notifications::new();
    n.ensure_progress(
        EXPORT_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::at("正在导出", 0.64)),
    );
    // 模拟 stop：中止态
    n.items
        .iter_mut()
        .find(|x| x.id == EXPORT_PROGRESS_ID)
        .unwrap()
        .cancelling = true;
    let id = n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Aborted,
        "已中止",
        "out.wav",
        None,
    );
    assert_eq!(id, EXPORT_PROGRESS_ID);
    let x = find(&n, EXPORT_PROGRESS_ID);
    assert_eq!(x.kind, ToastKind::Warning);
    assert_eq!(x.progress, Some(0.64));
    assert_eq!(x.progress_label, "已中止");
    assert!(x.source.is_none());
    assert!(!x.cancelling);
    assert!(x.collapse_at.is_some());
}

#[test]
fn finish_resurfaces_dismissed_task_card() {
    let mut n = Notifications::new();
    n.ensure_progress(
        EXPORT_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::at("正在导出", 0.4)),
    );
    // 用户收起后卡退场，仅列表可见
    n.dismiss(EXPORT_PROGRESS_ID);
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(!find(&n, EXPORT_PROGRESS_ID).on_screen);
    let id = n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Aborted,
        "已中止",
        "out.wav",
        None,
    );
    assert_eq!(id, EXPORT_PROGRESS_ID);
    assert!(find(&n, EXPORT_PROGRESS_ID).on_screen);
    assert_eq!(find(&n, EXPORT_PROGRESS_ID).progress_label, "已中止");
}

#[test]
fn finish_falls_back_to_push_when_entry_missing() {
    let mut n = Notifications::new();
    let id = n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Aborted,
        "已中止",
        "out.wav",
        None,
    );
    assert_ne!(id, 0);
    assert_ne!(id, EXPORT_PROGRESS_ID);
    let x = find(&n, id);
    assert_eq!(x.kind, ToastKind::Warning);
    assert_eq!(x.title, "已中止");
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
    // 与 show_toasts 收起分支同链路
    n.dismiss(EXPORT_PROGRESS_ID);
    assert!(
        !pause_flag.load(Ordering::Relaxed),
        "collapse must resume paused task"
    );
    assert!(find(&n, EXPORT_PROGRESS_ID).leaving_since.is_some());
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
    assert!(find(&n, id).leaving_since.is_none());
    // 关着时同条件会收（对照）
    n.center_open = false;
    // 关闭边沿本身就会收全部，这里仅断言 leaving 已起算
    n.tick(&ctx());
    assert!(find(&n, id).leaving_since.is_some());
}

#[test]
fn center_close_edge_dismisses_all() {
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    let static_id = n.success("s", "m");
    n.ensure_progress(
        EXPORT_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("t")),
    );
    // 开列表：先 tick 一次把 prev 对齐为 true
    n.center_open = true;
    n.tick(&ctx());
    assert!(
        n.items.iter().all(|x| x.leaving_since.is_none()),
        "open list must not dismiss"
    );
    // 关列表边沿：全部 leaving（含进行中；收起自动 resume 防卡死）
    n.center_open = false;
    n.tick(&ctx());
    assert!(find(&n, static_id).leaving_since.is_some());
    assert!(find(&n, EXPORT_PROGRESS_ID).leaving_since.is_some());
    // 退场播完：全部 off-screen（条目保留）
    std::thread::sleep(Duration::from_millis(330));
    n.tick(&ctx());
    assert!(n.items.iter().all(|x| !x.on_screen));
}

#[test]
fn center_open_edge_clears_unread() {
    let mut n = Notifications::new();
    // 关着时 push 产生未读
    n.center_open = false;
    n.tick(&ctx());
    let _ = n.success("a", "b");
    assert!(n.has_unread());
    // 开列表边沿清零
    n.center_open = true;
    n.tick(&ctx());
    assert!(!n.has_unread());
}

#[test]
fn center_open_push_and_ensure_stay_read() {
    let mut n = Notifications::new();
    n.center_open = true;
    n.tick(&ctx());
    assert!(!n.has_unread());
    // 开着时 push 不产生未读
    let _ = n.success("c", "d");
    assert!(!n.has_unread());
    // 开着时 ensure 新任务不产生未读
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("t")),
    );
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
    let ids: Vec<u64> = n.items.iter().map(|x| x.id).collect();
    for id in ids {
        n.card_h.insert(id, 110.0);
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
    // 造 3 条，每条实测 100，GAP=8：total=100*3+8*2=316
    let _ = n.success("a", "1");
    let _ = n.success("b", "2");
    let _ = n.success("c", "3");
    let ids: Vec<u64> = n.items.iter().map(|x| x.id).collect();
    for id in ids {
        n.card_h.insert(id, 100.0);
    }
    // viewport 高 200：visible=200-48-24=128，max=316-128=188
    n.center_scroll = 188.0;
    n.center_scroll_max = 188.0;
    n.update_center_scroll(200.0, 48.0, 8.0, 110.0);
    assert!((n.center_scroll_max - 188.0).abs() < 1e-4);
    // 新增一条变高到 total=424，max=296，底部附近应吸到新 max
    let last = n.success("d", "4");
    n.card_h.insert(last, 100.0);
    n.update_center_scroll(200.0, 48.0, 8.0, 110.0);
    assert!((n.center_scroll_max - 296.0).abs() < 1e-4);
    assert!((n.center_scroll - 296.0).abs() < 1e-4);
}
