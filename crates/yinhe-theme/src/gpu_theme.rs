/// All GPU rendering colors for the application.
///
/// Each field corresponds to a previously hardcoded color constant.
/// `default()` returns the current dark theme values.
#[derive(Clone, Debug)]
pub struct GpuTheme {
    // ── Pianoroll ──
    pub pr_bg: (f32, f32, f32),
    pub pr_measure_line: (f32, f32, f32, f32),
    pub pr_beat_line: (f32, f32, f32, f32),
    pub pr_sub_beat_line: (f32, f32, f32, f32),
    pub pr_black_key_row: (f32, f32, f32),
    pub pr_white_key: (f32, f32, f32),
    pub pr_black_key: (f32, f32, f32),
    pub pr_playhead: (f32, f32, f32, f32),

    // ── Arrangement ──
    pub ar_bg: (f32, f32, f32),
    pub ar_lane_even: (f32, f32, f32),
    pub ar_lane_odd: (f32, f32, f32),
    pub ar_measure_line: (f32, f32, f32, f32),
    pub ar_beat_line: (f32, f32, f32, f32),
    pub ar_playhead: (f32, f32, f32, f32),

    // ── Automation ──
    pub center_line: (f32, f32, f32, f32),
}

impl Default for GpuTheme {
    fn default() -> Self {
        Self {
            // Pianoroll
            pr_bg: (0.12, 0.12, 0.14),
            pr_measure_line: (0.35, 0.35, 0.40, 1.0),
            pr_beat_line: (0.22, 0.22, 0.25, 1.0),
            pr_sub_beat_line: (0.16, 0.16, 0.18, 1.0),
            pr_black_key_row: (0.10, 0.10, 0.12),
            pr_white_key: (0.70, 0.70, 0.70),
            pr_black_key: (0.16, 0.16, 0.17),
            pr_playhead: (1.0, 1.0, 1.0, 0.8),

            // Arrangement
            ar_bg: (0.14, 0.14, 0.16),
            ar_lane_even: (0.16, 0.16, 0.18),
            ar_lane_odd: (0.13, 0.13, 0.15),
            ar_measure_line: (0.30, 0.30, 0.35, 1.0),
            ar_beat_line: (0.20, 0.20, 0.23, 1.0),
            ar_playhead: (1.0, 1.0, 1.0, 0.8),

            // Automation
            center_line: (0.30, 0.30, 0.35, 0.6),
        }
    }
}
