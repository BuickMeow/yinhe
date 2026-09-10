use super::super::model::{ToastAction, ToastActionKind};
use super::*;

/// 无头渲染一张卡并量高度（无 CJK 字体时为 tofu，但跨状态可比）。
fn card_height(
    width: f32,
    title: &str,
    message: &str,
    progress: Option<f32>,
    label: &str,
    action: Option<ToastAction>,
) -> f32 {
    card_height_full(
        width, title, message, progress, label, action, false, false, false,
    )
}

/// 带取消/暂停按钮的高度测量（暂停按钮与 stop 同条件显示，验证覆盖层没被撑大）。
#[allow(clippy::too_many_arguments)]
fn card_height_full(
    width: f32,
    title: &str,
    message: &str,
    progress: Option<f32>,
    label: &str,
    action: Option<ToastAction>,
    with_cancel: bool,
    with_pause: bool,
    cancelling: bool,
) -> f32 {
    let ctx = egui::Context::default();
    // 注册图标字体（app 启动时同款，否则图标 label 排版 panic）
    ctx.add_font(egui_material_icons::font_insert());
    let mut h = 0.0;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let cancel = if with_cancel {
        Some(Arc::new(AtomicBool::new(false)))
    } else {
        None
    };
    let pause_flag = if with_pause {
        Some(Arc::new(AtomicBool::new(false)))
    } else {
        None
    };
    let mut out = ctx.run_ui(raw, |ui| {
        draw_card(
            ui,
            width,
            0.0,
            ToastKind::Info,
            title,
            message,
            progress,
            label,
            true,
            cancel.clone(),
            pause_flag.clone(),
            action.as_ref(),
            cancelling,
            Instant::now(),
        );
        h = ui.min_rect().height();
    });
    // 无头测试不贴纹理，显式丢弃（否则 debug 下 panic）
    out.textures_delta.clear();
    h
}

/// 暂停中（flag=true）高度测量：与进行中同按钮数，detail 覆盖“已暂停”仍 1 行。
fn card_height_paused(width: f32, title: &str, message: &str, progress: f32) -> f32 {
    let ctx = egui::Context::default();
    ctx.add_font(egui_material_icons::font_insert());
    let mut h = 0.0;
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let cancel = Some(Arc::new(AtomicBool::new(false)));
    let pause_flag = Some(Arc::new(AtomicBool::new(true)));
    let mut out = ctx.run_ui(raw, |ui| {
        draw_card(
            ui,
            width,
            0.0,
            ToastKind::Info,
            title,
            message,
            Some(progress),
            "已暂停",
            true,
            cancel.clone(),
            pause_flag.clone(),
            None,
            false,
            Instant::now(),
        );
        h = ui.min_rect().height();
    });
    out.textures_delta.clear();
    h
}

fn assert_same_height(cases: &[f32]) {
    let first = cases[0];
    for (i, h) in cases.iter().enumerate() {
        assert!((h - first).abs() < 0.5, "case {i}: height {h} != {first}");
    }
}

/// 进度族（进行中/完成/失败/中止，文案空满长短，有无操作按钮）高度必须一致，否则加载卡上下跳。
#[test]
fn progress_card_height_stable() {
    fn text_action(label: &str) -> ToastAction {
        ToastAction {
            label: label.to_string(),
            kind: ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
            icon: None,
        }
    }
    fn icon_action(label: &str) -> ToastAction {
        ToastAction {
            label: label.to_string(),
            kind: ToastActionKind::RevealInFolder(std::path::PathBuf::new()),
            icon: Some(ICON_FOLDER_OPEN),
        }
    }
    let w = 360.0;
    let running_empty = card_height(w, "正在加载", "解析 MIDI 音轨", Some(0.3), "", None);
    let running_short = card_height(w, "正在加载", "解析 MIDI 音轨", Some(0.3), "3/16", None);
    let running_long = card_height(
        w,
        "正在加载",
        "解析 MIDI 音轨",
        Some(0.9),
        "余韵衰减中 (剩余 3 音色) 余韵衰减中 (剩余 3 音色) 余韵衰减中",
        None,
    );
    let running_long_msg = card_height(
        w,
        "正在加载",
        "这是一段非常非常长的阶段文案这是一段非常非常长的阶段文案这是一段非常非常长的阶段文案",
        Some(0.5),
        "3/16",
        None,
    );
    let done = card_height(w, "MIDI加载完成", "a.mid", Some(1.0), "已完成", None);
    let done_duration = card_height(
        w,
        "MIDI加载完成",
        "a.mid",
        Some(1.0),
        "加载时间：15秒321毫秒",
        None,
    );
    let failed = card_height(w, "打开失败", "err", Some(0.5), "失败", None);
    let done_action = card_height(
        w,
        "已完成",
        "out.wav (12.3s, 8.1x)",
        Some(1.0),
        "已完成",
        Some(text_action("打开文件夹")),
    );
    let aborted = card_height(w, "正在导出", "渲染中 64%", Some(0.64), "已中止", None);
    let aborted_action = card_height(
        w,
        "正在导出",
        "渲染中 64%",
        Some(0.64),
        "已中止",
        Some(icon_action("打开文件夹")),
    );
    // 暂停中：按钮数（收起+stop+pause）/行数与进行中一致，detail“已暂停”仍 1 行
    let running_with_pause = card_height_full(
        w,
        "正在导出",
        "渲染中 64%",
        Some(0.64),
        "已渲染00:12 · 当前8.1x",
        None,
        true,
        true,
        false,
    );
    let paused = card_height_paused(w, "正在导出", "渲染中 64%", 0.64);
    assert_same_height(&[
        running_empty,
        running_short,
        running_long,
        running_long_msg,
        done,
        done_duration,
        failed,
        done_action,
        aborted,
        aborted_action,
        running_with_pause,
        paused,
    ]);
}

/// 暂停 toggle 翻转 flag（resolve+toggle 链路；draw_card 内点击即同语义 store）。
#[test]
fn pause_toggle_flips_flag() {
    use super::super::kind::ToastKind as Kind;
    use super::super::model::Toast;
    use super::super::model::{resolve_pause_toast, resolve_toast};
    use std::time::Instant as StdInstant;
    let pause_flag = Arc::new(AtomicBool::new(false));
    // ExportToastSource 级真 flag（与线上同构）
    let progress = yinhe_audio::export::ExportProgress::new();
    let src = Arc::new(crate::app::export_state::ExportToastSource {
        progress,
        cancel: Arc::new(AtomicBool::new(false)),
        pause: Arc::clone(&pause_flag),
    });
    let toast = Toast {
        id: 1,
        kind: Kind::Info,
        title: String::new(),
        message: String::new(),
        created: StdInstant::now(),
        progress: None,
        progress_label: String::new(),
        cancel: None,
        leaving_since: None,
        source: Some(src),
        collapse_at: None,
        action: None,
        hovered: false,
        cancelling: false,
    };
    // 未暂停时 detail 非“已暂停”
    let resolved = resolve_pause_toast(&toast);
    assert!(resolved.is_some());
    if let Some(p) = resolved {
        assert!(!p.load(std::sync::atomic::Ordering::Relaxed));
        // 模拟暂停按钮点击 toggle
        let cur = p.load(std::sync::atomic::Ordering::Relaxed);
        p.store(!cur, std::sync::atomic::Ordering::Relaxed);
    }
    assert!(pause_flag.load(std::sync::atomic::Ordering::Relaxed));
    // 已暂停态 detail 覆盖“已暂停”
    let (_, _, _, detail) = resolve_toast(&toast);
    assert_eq!(detail, "已暂停");
    // 再 toggle 恢复
    if let Some(p) = resolve_pause_toast(&toast) {
        let cur = p.load(std::sync::atomic::Ordering::Relaxed);
        p.store(!cur, std::sync::atomic::Ordering::Relaxed);
    }
    assert!(!pause_flag.load(std::sync::atomic::Ordering::Relaxed));
}
