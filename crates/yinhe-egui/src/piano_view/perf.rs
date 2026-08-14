//! Pianoroll 性能探针（仅 `YIN_PERF=1` 启用）。
//!
//! 在 `show` 各阶段记录时间戳，末尾提交一个 `FrameSample`。
//! 拆出本模块仅为缩短 piano_view.rs；调用点仍负责采集时间戳。

use std::time::Instant;

use yinhe_types::{NoteSource, PianoRollView};

use crate::view_interaction::FollowMode;

/// 性能探针所需的时间戳和上下文。
pub struct PerfCtx<'a> {
    pub t_show_start: Option<Instant>,
    pub t_input_end: Option<Instant>,
    pub t_prepare_end: Option<Instant>,
    pub t_paint_end: Option<Instant>,
    pub follow_mode: &'a FollowMode,
    pub midi: Option<&'a dyn NoteSource>,
    pub view: &'a PianoRollView,
    pub width: f32,
}

/// 提交本帧性能采样。任一时间戳缺失时直接返回。
pub fn submit(ctx: PerfCtx) {
    let PerfCtx {
        t_show_start,
        t_input_end,
        t_prepare_end,
        t_paint_end,
        follow_mode,
        midi,
        view,
        width,
    } = ctx;

    let (Some(t0), Some(t1), Some(t2), Some(t3)) =
        (t_show_start, t_input_end, t_prepare_end, t_paint_end)
    else {
        return;
    };

    let t_end = Instant::now();
    let input = t1.saturating_duration_since(t0);
    let prepare_total = t2.saturating_duration_since(t1);
    let paint = t3.saturating_duration_since(t2);
    let misc = t_end.saturating_duration_since(t3);

    let total_notes = midi
        .map(|m| {
            let mut sum = 0u64;
            for k in 0..128u8 {
                sum += m.key_notes(k).len() as u64;
            }
            sum
        })
        .unwrap_or(0);

    let (s, e) = view.visible_tick_range(width);

    yinhe_memtrace::perf_probe::submit(yinhe_memtrace::perf_probe::FrameSample {
        input,
        prep_static: prepare_total,
        paint,
        misc,
        instance_count: 0,
        follow_mode: match follow_mode {
            FollowMode::None => "None",
            FollowMode::Centered => "Centered",
            FollowMode::Page => "Page",
            FollowMode::Continuous => "Continuous",
        },
        total_notes,
        ppt: view.base.pixels_per_tick,
        visible_ticks: e - s,
    });
}
