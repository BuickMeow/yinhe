//! 音符横向拖动的“自由调节 / 粘性量化”状态。
//!
//! 规则（类似 Ableton Live）：
//! - 整个手势中横向位移从未达到 1 个量化间隔：不吸附，按原始 tick 跟手。
//! - 一旦越过 1 个量化间隔：本手势剩余时间都按量化吸附，即使再拖回起点附近。
//!
//! 垂直音高移动始终按 key 吸附，不参与这里的判断。

use eframe::egui;
use yinhe_editor_core::quantize::QuantizePreset;

#[derive(Clone, Copy, Debug)]
struct FreeQuant {
    press_raw_tick: f64,
    latched: bool,
}

/// 按下时记录原始指针 tick；`raw_tick` 必须来自同一次按下的音乐坐标。
pub(crate) fn begin(ui: &egui::Ui, key: &'static str, raw_tick: f64) {
    ui.data_mut(|d| {
        d.insert_temp(
            ui.scope_id().with(key),
            FreeQuant {
                press_raw_tick: raw_tick,
                latched: false,
            },
        )
    });
}

/// 计算本次移动的 tick 增量。
///
/// `press_snapped_tick` 是按下时吸附后的 tick，`snapped_tick` 是当前吸附 tick；
/// 状态缺失时用 `press_snapped_tick` 作为原始按下点兜底。
#[allow(clippy::too_many_arguments)]
pub(crate) fn delta(
    ui: &egui::Ui,
    key: &'static str,
    raw_tick: f64,
    press_snapped_tick: f64,
    snapped_tick: f64,
    quantize: QuantizePreset,
    ppq: u32,
) -> i64 {
    let id = ui.scope_id().with(key);
    let mut state: FreeQuant = ui.data_mut(|d| {
        d.get_temp(id).unwrap_or(FreeQuant {
            press_raw_tick: press_snapped_tick,
            latched: false,
        })
    });
    let interval = f64::from(quantize.tick_interval(ppq));
    if !state.latched && interval > 0.0 && (raw_tick - state.press_raw_tick).abs() >= interval {
        state.latched = true;
    }
    let dt = if state.latched {
        (snapped_tick - press_snapped_tick).round() as i64
    } else {
        (raw_tick - state.press_raw_tick).round() as i64
    };
    ui.data_mut(|d| d.insert_temp(id, state));
    dt
}

/// 手势结束/取消时清理。
pub(crate) fn clear(ui: &egui::Ui, key: &'static str) {
    ui.data_mut(|d| d.remove::<FreeQuant>(ui.scope_id().with(key)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_until_one_quantize_then_latched() {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(100.0, 100.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            begin(ui, "fq_test", 100.0);
            // 自由区：29 tick 内跟手。
            out.push(delta(
                ui,
                "fq_test",
                119.0,
                90.0,
                120.0,
                QuantizePreset::Fraction(1, 64),
                480,
            ));
            // 越过 30 tick：锁存量化的 snapped 增量。
            out.push(delta(
                ui,
                "fq_test",
                130.0,
                90.0,
                120.0,
                QuantizePreset::Fraction(1, 64),
                480,
            ));
            // 缩回自由区：仍保持量化。
            out.push(delta(
                ui,
                "fq_test",
                101.0,
                90.0,
                90.0,
                QuantizePreset::Fraction(1, 64),
                480,
            ));
        })
        .drop_without_applying_deltas();
        assert_eq!(out, vec![19, 30, 0]);
    }

    #[test]
    fn clear_resets_latch() {
        let ctx = egui::Context::default();
        let mut out = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(100.0, 100.0),
            )),
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            begin(ui, "fq_test", 100.0);
            let _ = delta(
                ui,
                "fq_test",
                130.0,
                90.0,
                120.0,
                QuantizePreset::Fraction(1, 64),
                480,
            );
            clear(ui, "fq_test");
            begin(ui, "fq_test", 100.0);
            out.push(delta(
                ui,
                "fq_test",
                110.0,
                90.0,
                90.0,
                QuantizePreset::Fraction(1, 64),
                480,
            ));
        })
        .drop_without_applying_deltas();
        assert_eq!(out, vec![10], "重新起手后应重新进入自由区");
    }
}
