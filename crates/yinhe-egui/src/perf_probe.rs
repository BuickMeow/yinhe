//! Frame-time decomposition probe.
//!
//! Activate by setting env var `YIN_PERF=1`. Aggregates per-phase Durations
//! over a rolling window (default 60 frames ≈ 1 second at 60fps) and emits a
//! single `tracing::info!` line summarising the average.
//!
//! To keep the log readable during idle stretches, output is throttled:
//! - A summary is emitted at most once per `WINDOW` frames OR every
//!   `MAX_FLUSH_INTERVAL` of wall-clock time (whichever comes first), so even
//!   low-fps states still print eventually.
//! - If the new summary's total frame time differs from the previous emitted
//!   one by less than `QUIET_THRESHOLD_RATIO`, it is suppressed.
//! - A summary is always emitted when `static_rebuild_count` changes from 0
//!   to non-zero (or vice versa) so transitions are visible.

use std::sync::OnceLock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const WINDOW: usize = 60;
const QUIET_THRESHOLD_RATIO: f64 = 0.05;
const MAX_FLUSH_INTERVAL: Duration = Duration::from_secs(2);
/// Always emit at least one summary every this many flushes, even if the
/// content looks unchanged. Prevents indefinite silence in steady state.
const FORCE_EMIT_EVERY_N_FLUSHES: u32 = 5;

#[derive(Default, Clone, Copy, Debug)]
pub struct FrameSample {
    pub input: Duration,
    pub prep_static: Duration,
    pub prep_cursor: Duration,
    pub upload: Duration,
    pub paint: Duration,
    pub misc: Duration,
    pub static_rebuilt: bool,
    pub instance_count: usize,
    pub follow_mode: &'static str,
    /// Total notes in the loaded MIDI (0 if none).
    pub total_notes: u64,
    /// Current zoom: pixels per tick.
    pub ppt: f32,
    /// Visible tick window width = visible_ticks (end - start).
    pub visible_ticks: f64,
}

pub fn enabled() -> bool {
    static E: OnceLock<bool> = OnceLock::new();
    *E.get_or_init(|| {
        let on = std::env::var("YIN_PERF").as_deref() == Ok("1");
        if on {
            eprintln!("[yin_perf] enabled (YIN_PERF=1) — summaries every {WINDOW} frames or {}s, whichever first", MAX_FLUSH_INTERVAL.as_secs());
        }
        on
    })
}

struct Aggregator {
    samples: Vec<FrameSample>,
    last_flush_at: Option<Instant>,
    /// Wall-clock instant of first sample in current window (for real fps).
    window_started_at: Option<Instant>,
    last_emitted_total_ms: f64,
    last_emitted_rebuild_active: bool,
    quiet_flushes_in_row: u32,
}

impl Aggregator {
    const fn new() -> Self {
        Self {
            samples: Vec::new(),
            last_flush_at: None,
            window_started_at: None,
            last_emitted_total_ms: 0.0,
            last_emitted_rebuild_active: false,
            quiet_flushes_in_row: 0,
        }
    }
}

fn agg() -> &'static Mutex<Aggregator> {
    static A: OnceLock<Mutex<Aggregator>> = OnceLock::new();
    A.get_or_init(|| Mutex::new(Aggregator::new()))
}

/// Submit a per-frame sample. Cheap: amortised O(1), holds Mutex briefly.
pub fn submit(sample: FrameSample) {
    if !enabled() {
        return;
    }
    static SUBMITTED: AtomicU64 = AtomicU64::new(0);
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    static LAST_HEARTBEAT: OnceLock<Mutex<Instant>> = OnceLock::new();

    let sub = SUBMITTED.fetch_add(1, Ordering::Relaxed) + 1;

    let Ok(mut a) = agg().try_lock() else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    };
    a.samples.push(sample);
    if a.window_started_at.is_none() {
        a.window_started_at = Some(Instant::now());
    }

    let now = Instant::now();
    // Heartbeat: every 2s tell stderr how many samples landed, so we know
    // submit() is actually being called even if flush is somehow gated.
    {
        let hb = LAST_HEARTBEAT.get_or_init(|| Mutex::new(now));
        if let Ok(mut last) = hb.try_lock()
            && now.duration_since(*last) >= Duration::from_secs(2)
        {
            *last = now;
            eprintln!(
                "[yin_perf] heartbeat: submitted={sub} dropped={} buffered={}",
                DROPPED.load(Ordering::Relaxed),
                a.samples.len()
            );
        }
    }

    let should_flush_count = a.samples.len() >= WINDOW;
    let should_flush_time = match a.last_flush_at {
        Some(t) => now.duration_since(t) >= MAX_FLUSH_INTERVAL,
        None => false,
    };
    if a.last_flush_at.is_none() {
        a.last_flush_at = Some(now);
    }
    if should_flush_count || (should_flush_time && !a.samples.is_empty()) {
        flush(&mut a);
        a.last_flush_at = Some(now);
    }
}

fn flush(a: &mut Aggregator) {
    let n = a.samples.len();
    if n == 0 {
        return;
    }
    let window_elapsed = a
        .window_started_at
        .map(|t| t.elapsed())
        .unwrap_or(Duration::ZERO);
    a.window_started_at = None;

    let mut sum_input = Duration::ZERO;
    let mut sum_ps = Duration::ZERO;
    let mut sum_pc = Duration::ZERO;
    let mut sum_up = Duration::ZERO;
    let mut sum_pt = Duration::ZERO;
    let mut sum_mi = Duration::ZERO;
    let mut rebuild_count = 0usize;
    let mut max_inst = 0usize;
    let mut follow = "";
    let mut total_notes = 0u64;
    let mut ppt = 0.0f32;
    let mut max_visible_ticks = 0.0f64;

    for s in &a.samples {
        sum_input += s.input;
        sum_ps += s.prep_static;
        sum_pc += s.prep_cursor;
        sum_up += s.upload;
        sum_pt += s.paint;
        sum_mi += s.misc;
        if s.static_rebuilt {
            rebuild_count += 1;
        }
        max_inst = max_inst.max(s.instance_count);
        follow = s.follow_mode;
        total_notes = total_notes.max(s.total_notes);
        ppt = s.ppt;
        if s.visible_ticks > max_visible_ticks {
            max_visible_ticks = s.visible_ticks;
        }
    }

    let nf = n as f64;
    let to_ms = |d: Duration| d.as_secs_f64() * 1000.0 / nf;
    let input = to_ms(sum_input);
    let ps = to_ms(sum_ps);
    let pc = to_ms(sum_pc);
    let up = to_ms(sum_up);
    let pt = to_ms(sum_pt);
    let mi = to_ms(sum_mi);
    let total = input + ps + pc + up + pt + mi;
    let cpu_fps = if total > 0.0 { 1000.0 / total } else { 0.0 };
    let wall_secs = window_elapsed.as_secs_f64();
    let real_fps = if wall_secs > 0.0 { nf / wall_secs } else { 0.0 };

    let rebuild_active = rebuild_count > 0;
    let total_delta = (total - a.last_emitted_total_ms).abs();
    let total_ref = a.last_emitted_total_ms.max(0.001);
    let quiet = total_delta / total_ref < QUIET_THRESHOLD_RATIO
        && rebuild_active == a.last_emitted_rebuild_active;

    let force = a.quiet_flushes_in_row >= FORCE_EMIT_EVERY_N_FLUSHES;

    if !quiet || force {
        let line = format!(
            "[yin_perf] frames={n} real_fps={real_fps:.1} cpu_fps={cpu_fps:.0} \
             cpu/frame={total:.2}ms wall={wall_secs:.2}s | input={input:.2} \
             prep_static={ps:.2}(rb {rebuild_count}/{n}) prep_cursor={pc:.3} \
             upload={up:.2} | paint={pt:.2} misc={mi:.2} \
             | inst_max={max_inst} notes={total_notes} ppt={ppt:.4} \
             vis_ticks={max_visible_ticks:.0} follow={follow}"
        );
        eprintln!("{line}");
        tracing::info!(target: "yin_perf", "{line}");
        a.last_emitted_total_ms = total;
        a.last_emitted_rebuild_active = rebuild_active;
        a.quiet_flushes_in_row = 0;
    } else {
        a.quiet_flushes_in_row += 1;
    }

    a.samples.clear();
}
