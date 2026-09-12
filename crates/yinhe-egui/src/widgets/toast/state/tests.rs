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

/// 在无头 pass 内跑一帧 tick（可指定窗口焦点），模拟真实帧输入。
fn tick_frame(n: &mut Notifications, ctx: &egui::Context, focused: bool) {
    let raw = egui::RawInput {
        focused,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        n.tick(ui.ctx());
    });
    out.textures_delta.clear();
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

/// 窗口失焦暂停自动收起计时（回来继续），与悬停同链路。
#[test]
fn window_blur_pauses_collapse_deadline() {
    let ctx = ctx();
    let mut n = Notifications::new();
    n.set_collapse_durations(Some(60), Some(60));
    let id = n.success("t", "m");
    n.tick(&ctx); // 首 tick 只记录 last_tick
    std::thread::sleep(Duration::from_millis(5));
    let before = find(&n, id).collapse_at;
    // 失焦帧：deadline 顺延
    tick_frame(&mut n, &ctx, false);
    let after = find(&n, id).collapse_at;
    assert!(after.unwrap() > before.unwrap());
    // 恢复聚焦后不再顺延
    std::thread::sleep(Duration::from_millis(2));
    tick_frame(&mut n, &ctx, true);
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
    let done = n.finish_progress(
        EXPORT_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done",
        "f",
        None,
    );
    let before = find(&n, done).collapse_at.unwrap();
    assert!(before < Instant::now() + Duration::from_secs(10));
    // 挂操作按钮 → 自动升为可操作档
    n.set_action_with_icon(
        done,
        "打开文件夹",
        super::super::model::ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
        None,
    );
    let after = find(&n, done).collapse_at.unwrap();
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
    // 任务结束：离屏条目转正弹出（独立 id），槽位腾空
    let done = n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done",
        "",
        None,
    );
    assert_ne!(done, LOADING_PROGRESS_ID);
    assert!(find(&n, done).on_screen);
    // 下个任务新建槽位，不再覆盖完成通知
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v3")),
    );
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

/// 固定 id 只是任务槽位：每次完成转正为独立通知，多次操作各自保留完成卡。
#[test]
fn fixed_id_finish_promotes_to_standalone_entry() {
    let mut n = Notifications::new();
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v1")),
    );
    assert_eq!(n.items.len(), 1);
    let done1 = n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done1",
        "a.mid",
        None,
    );
    assert_ne!(done1, LOADING_PROGRESS_ID);
    assert_eq!(n.items.len(), 1);
    // 槽位已腾空，完成通知以独立 id 继续存在
    assert!(!n.items.iter().any(|x| x.id == LOADING_PROGRESS_ID));
    // 第二轮：新建进行中条目，不覆盖完成卡
    n.ensure_progress(
        LOADING_PROGRESS_ID,
        ToastKind::Info,
        Arc::new(FakeSource::new("v2")),
    );
    assert_eq!(n.items.len(), 2);
    let done2 = n.finish_progress(
        LOADING_PROGRESS_ID,
        ProgressOutcome::Completed,
        "done2",
        "b.mid",
        None,
    );
    assert_ne!(done2, done1);
    assert_eq!(n.items.len(), 2);
    let x1 = find(&n, done1);
    let x2 = find(&n, done2);
    assert_eq!(x1.title, "done1");
    assert_eq!(x2.title, "done2");
    assert_eq!(x2.progress, Some(1.0));
    assert_eq!(x2.progress_label, "已完成");
    assert!(x1.source.is_none() && x2.source.is_none());
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
    assert_ne!(id, EXPORT_PROGRESS_ID);
    let x = find(&n, id);
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
    assert_ne!(id, EXPORT_PROGRESS_ID);
    assert!(find(&n, id).on_screen);
    assert_eq!(find(&n, id).progress_label, "已中止");
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

/// 回归：堆叠重排时按「显示位置」而非「目标位置」裁卡。
/// 目标已出带但 y 动画还在带内时，卡片必须继续渲染（否则未滑出即消失）。
#[test]
fn cull_uses_displayed_y_not_target() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut n = Notifications::new();
    let mut oldest = 0;
    for i in 0..10 {
        let id = n.success(format!("t{i}"), "m");
        if i == 0 {
            oldest = id;
        }
    }
    let ids: Vec<u64> = n.items.iter().map(|x| x.id).collect();
    for id in ids {
        n.card_h.insert(id, 110.0);
    }
    // 哨兵高度：被裁则保持原值，渲染则被实测覆盖
    const SENTINEL: f32 = 1234.5;
    n.card_h.insert(oldest, SENTINEL);
    // 最旧卡目标位置：10 张堆叠远超视口顶部（出带）
    // 显示位置：y 动画仍在带内（卡坐在底边上），动画尚未滑出
    n.y_anim.insert(
        oldest,
        YAnim {
            from: 48.0,
            to: -9999.0,
            t0: Instant::now(),
        },
    );
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    let raw = egui::RawInput {
        screen_rect: Some(viewport),
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        n.show_toasts(ui.ctx(), viewport);
    });
    out.textures_delta.clear();
    let h = n.card_h.get(&oldest).copied().unwrap_or(SENTINEL);
    assert!(
        (h - SENTINEL).abs() > f32::EPSILON,
        "目标出带但显示位置仍在带内时被提前裁掉（提前消失 bug）"
    );
}

/// 回归：打开通知中心时卡片必须实际渲染在视口内。
/// 曾因 Area 首帧默认尺寸过小，ScrollArea 被压成 1px 宽只剩滚动条。
#[test]
fn center_list_renders_cards_inside_viewport() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    let _ = n.success("title-zero", "m");
    let _ = n.success("title-one", "m");
    n.center_open = true;
    n.tick(&ctx);
    // 等整列滑入动画结束
    std::thread::sleep(Duration::from_millis(350));
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    // Area/ScrollArea 首帧只做 sizing，第二帧才落位绘制
    let mut found = 0;
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            n.show_center(ui.ctx(), viewport);
        });
        out.textures_delta.clear();
        found = 0;
        for cs in &out.shapes {
            if let egui::Shape::Text(t) = &cs.shape {
                let text = t.galley.text();
                if (text == "title-zero" || text == "title-one") && viewport.contains(t.pos) {
                    found += 1;
                }
            }
        }
    }
    assert_eq!(found, 2, "通知中心应把两张卡画在视口内，实际 {found}");
}

/// 回归：通知超出视口时滚动吸底，最新的几张必须渲染。
/// 曾因 draw_card 用 available_rect 做 clip，滚动后下半部分卡片拿到反向矩形被整卡跳过。
#[test]
fn center_list_shows_newest_when_overflow() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    for i in 0..12 {
        let _ = n.success(format!("title-{i:02}"), "m");
    }
    n.center_open = true;
    n.tick(&ctx);
    std::thread::sleep(Duration::from_millis(350));
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 900.0));
    let mut visible: Vec<String> = Vec::new();
    // Area/ScrollArea 首帧只做 sizing，第二帧才落位绘制
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            n.show_center(ui.ctx(), viewport);
        });
        out.textures_delta.clear();
        visible.clear();
        for cs in &out.shapes {
            if let egui::Shape::Text(t) = &cs.shape {
                let text = t.galley.text().to_string();
                if text.starts_with("title-") && viewport.contains(t.pos) {
                    visible.push(text);
                }
            }
        }
    }
    assert!(
        visible.iter().any(|t| t == "title-11"),
        "滚动到底后最新通知应可见，实际可见 {visible:?}"
    );
    assert!(
        !visible.iter().any(|t| t == "title-00"),
        "最早的应滚出视口，实际可见 {visible:?}"
    );
}

/// 回归：通知区底边跟随底部状态栏（中央区域底部），裁剪线不写死窗口底留白。
#[test]
fn center_bottom_follows_bottom_panel() {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    for i in 0..6 {
        let _ = n.success(format!("title-{i:02}"), "m");
    }
    n.center_open = true;
    n.tick(&ctx);
    std::thread::sleep(Duration::from_millis(350));
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let mut visible: Vec<(String, f32)> = Vec::new();
    // Area/ScrollArea 首帧只做 sizing，第二帧才落位绘制
    for _ in 0..2 {
        let raw = egui::RawInput {
            screen_rect: Some(viewport),
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            // 模拟底部状态栏
            egui::Panel::bottom("test_bar").show(ui, |ui| {
                ui.allocate_space(egui::vec2(100.0, 40.0));
            });
            n.show_center(ui.ctx(), ui.available_rect_before_wrap());
        });
        out.textures_delta.clear();
        visible.clear();
        for cs in &out.shapes {
            if let egui::Shape::Text(t) = &cs.shape {
                let s = t.galley.text().to_string();
                if s.starts_with("title-") {
                    visible.push((s, t.pos.y));
                }
            }
        }
    }
    // 最新卡片完整落在底栏上方（底栏顶 ≤ 560）
    assert!(
        visible.iter().any(|(t, y)| t == "title-05" && *y < 560.0),
        "最新卡应完整显示在底栏上方，实际 {visible:?}"
    );
}

#[test]
#[ignore]
fn debug_snapshot_center() {
    use egui_kittest::Harness;
    let mut n = Notifications::new();
    n.set_collapse_durations(None, None);
    for i in 0..10 {
        let _ = n.success(format!("title-{i:02}"), "EnchantedLove");
    }
    n.center_open = true;
    let mut first = true;
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .wgpu()
        .build_ui_state(
            move |ui, _| {
                if first {
                    first = false;
                    ui.ctx().add_font(egui_material_icons::font_insert());
                    return;
                }
                n.show_center(ui.ctx(), ui.available_rect_before_wrap());
            },
            (),
        );
    for _ in 0..4 {
        harness.step();
    }
    let img = harness.render().unwrap();
    let path = "/var/folders/gs/_prs479j56bdvpc9ggv9c9880000gn/T/opencode/center_debug.png";
    img.save(path).unwrap();
    println!("SAVED {path}");
}
