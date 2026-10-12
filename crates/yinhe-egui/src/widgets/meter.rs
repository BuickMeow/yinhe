//! 自绘电平/力度表。
//!
//! - [`level_meter`]：混音台通道条用，向上 L/R 双条 + dB 数字刻度；
//! - [`compact_meter`]：走带栏时间码第四栏用，向上 L/R 双条、无刻度；
//! - [`velocity_meter`]：音轨面板每轨用，**向下**单条（归一化力度），与电平表
//!   以方向区分。
//!
//! 三者共用同一套 dB 映射/着色（[`db_frac`] / [`meter_color`]）。

use eframe::egui;

/// 电平表单条宽 + 条间距（px）。
pub(crate) const METER_W: f32 = 6.0;
pub(crate) const METER_GAP: f32 = 2.0;
/// 电平表左侧 dB 数字刻度栏宽（px）。
pub(crate) const SCALE_W: f32 = 20.0;
/// 推子/电平表 dB 范围。
pub(crate) const DB_MIN: f32 = -60.0;
pub(crate) const DB_MAX: f32 = 6.0;

/// 线性增益 → dB（0 以下按 -60 显示）。
pub(crate) fn gain_to_db(g: f32) -> f32 {
    if g <= 0.0001 { -60.0 } else { 20.0 * g.log10() }
}

/// dB → 线性增益（≤ -60 dB 视为静音）。
pub(crate) fn db_to_gain(db: f32) -> f32 {
    if db <= -59.9 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// dB 值 → 纵向占比（0 = DB_MIN，1 = DB_MAX）。
pub(crate) fn db_frac(db: f32) -> f32 {
    ((db - DB_MIN) / (DB_MAX - DB_MIN)).clamp(0.0, 1.0)
}

/// 电平表刻度：`(dB, 是否标注数字)`。行业惯用标注点。
const METER_TICKS: &[(f32, bool)] = &[
    (0.0, true),
    (-6.0, true),
    (-12.0, true),
    (-18.0, false),
    (-24.0, true),
    (-36.0, false),
    (-48.0, true),
];

/// 自绘电平表（向上 L/R 双条）：左侧 dB 数字刻度栏 + 双条 + 刻度 + 峰值保持线。
/// dB 映射 -60..+6；>0dB 金色、≥0dBFS 红色顶格。
pub(crate) fn level_meter(ui: &mut egui::Ui, level: (f32, f32), hold: (f32, f32), height: f32) {
    let bars_w = METER_W * 2.0 + METER_GAP;
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(SCALE_W + bars_w, height), egui::Sense::hover());
    let painter = ui.painter();
    let bars = egui::Rect::from_min_size(
        egui::pos2(rect.min.x + SCALE_W, rect.min.y),
        egui::vec2(bars_w, height),
    );
    painter.rect_filled(bars, 0.0, crate::theme::track_bg());

    // 刻度线 + 数字标注。
    let font = egui::FontId::proportional(crate::theme::SMALL_LABEL_FONT);
    for &(db, labeled) in METER_TICKS {
        let y = bars.max.y - db_frac(db) * bars.height();
        painter.hline(
            bars.x_range(),
            y,
            egui::Stroke::new(
                1.0,
                crate::theme::text_muted().gamma_multiply(if labeled { 0.7 } else { 0.35 }),
            ),
        );
        if labeled {
            painter.text(
                egui::pos2(bars.min.x - 3.0, y),
                egui::Align2::RIGHT_CENTER,
                format!("{}", db as i32),
                font.clone(),
                crate::theme::text_secondary(),
            );
        }
    }

    // L/R 填充条。
    for (i, &p) in [level.0, level.1].iter().enumerate() {
        let x0 = bars.min.x + i as f32 * (METER_W + METER_GAP);
        let bar = egui::Rect::from_min_size(
            egui::pos2(x0, bars.min.y),
            egui::vec2(METER_W, bars.height()),
        );
        let frac = db_frac(gain_to_db(p));
        if frac > 0.0 {
            let h = bar.height() * frac;
            let fill = egui::Rect::from_min_max(egui::pos2(bar.min.x, bar.max.y - h), bar.max);
            painter.rect_filled(fill, 0.0, meter_color(p));
        }
    }

    // 峰值保持线（比当前电平高时显示）。
    for (i, (&l, &h)) in [level.0, level.1]
        .iter()
        .zip([hold.0, hold.1].iter())
        .enumerate()
    {
        if h <= l + 1e-4 {
            continue;
        }
        let x0 = bars.min.x + i as f32 * (METER_W + METER_GAP);
        let y = bars.max.y - db_frac(gain_to_db(h)) * bars.height();
        painter.hline(
            egui::Rangef::new(x0, x0 + METER_W),
            y,
            egui::Stroke::new(1.0, meter_color(h)),
        );
    }
}

/// 无刻度的窄版电平表（走带栏时间码用）：L/R 双条铺满给定矩形，带峰值保持线。
pub(crate) fn compact_meter(
    painter: &egui::Painter,
    rect: egui::Rect,
    level: (f32, f32),
    hold: (f32, f32),
) {
    painter.rect_filled(rect, 0.0, crate::theme::track_bg());
    let bar_w = (rect.width() - METER_GAP) * 0.5;
    for (i, &p) in [level.0, level.1].iter().enumerate() {
        let x0 = rect.min.x + i as f32 * (bar_w + METER_GAP);
        let frac = db_frac(gain_to_db(p));
        if frac > 0.0 {
            let h = rect.height() * frac;
            let fill = egui::Rect::from_min_max(
                egui::pos2(x0, rect.max.y - h),
                egui::pos2(x0 + bar_w, rect.max.y),
            );
            painter.rect_filled(fill, 0.0, meter_color(p));
        }
    }
    for (i, (&l, &h)) in [level.0, level.1]
        .iter()
        .zip([hold.0, hold.1].iter())
        .enumerate()
    {
        if h <= l + 1e-4 {
            continue;
        }
        let x0 = rect.min.x + i as f32 * (bar_w + METER_GAP);
        let y = rect.max.y - db_frac(gain_to_db(h)) * rect.height();
        painter.hline(
            egui::Rangef::new(x0, x0 + bar_w),
            y,
            egui::Stroke::new(1.0, meter_color(h)),
        );
    }
}

/// 力度表（向下单条）：自顶边起向下填充，`level`/`hold` 为归一化力度 0..1。
/// 与电平表（向上、L/R 双条）以方向区分；仅播放时非零。
pub(crate) fn velocity_meter(painter: &egui::Painter, rect: egui::Rect, level: f32, hold: f32) {
    painter.rect_filled(rect, 0.0, crate::theme::track_bg());
    let frac = level.clamp(0.0, 1.0);
    if frac > 0.0 {
        let h = rect.height() * frac;
        painter.rect_filled(
            egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), h)),
            0.0,
            velocity_color(level),
        );
    }
    let hf = hold.clamp(0.0, 1.0);
    if hf > frac + 1e-4 {
        let y = rect.min.y + rect.height() * hf;
        painter.hline(
            rect.x_range(),
            y,
            egui::Stroke::new(1.0, velocity_color(hold)),
        );
    }
}

/// 电平着色：≥0dBFS 红、> -6dB 金、其余强调色。
fn meter_color(p: f32) -> egui::Color32 {
    if p >= 1.0 {
        crate::theme::danger_text()
    } else if gain_to_db(p) > -6.0 {
        crate::theme::warning_gold()
    } else {
        crate::theme::accent_active()
    }
}

/// 力度着色：接近满力度红、>0.75 金、其余强调色。
fn velocity_color(v: f32) -> egui::Color32 {
    if v >= 0.96 {
        crate::theme::danger_text()
    } else if v > 0.75 {
        crate::theme::warning_gold()
    } else {
        crate::theme::accent_active()
    }
}
