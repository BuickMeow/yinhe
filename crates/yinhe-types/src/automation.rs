use serde::{Deserialize, Serialize};

/// How to interpolate from one automation event to the next.
///
/// Stored per-event on `AutomationEvent::shape`, describing the segment
/// that *starts* at this event. The last event's shape has no effect
/// (no segment after it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SegmentShape {
    /// 离散：保持当前值，直到下个事件才瞬间跳变。MIDI CC 的原生语义。
    Step,
    /// 三次贝塞尔曲线（CSS handle 风格，偏移量参数化）。
    ///
    /// 归一化空间：起点 P0=(0,0) 对应本事件 (tick, value)，终点 P3=(1,1) 对应下一事件。
    /// 存储值为控制点相对各自锚点的归一化偏移量，内部 `*4` 放大得到实际贝塞尔参数：
    ///
    /// - `(x1, y1)`：P1 相对 P0 的偏移，实际位置 P1 = P0 + (P3-P0)·(x1·4, y1·4)
    /// - `(x2, y2)`：P2 相对 P3 的偏移，实际位置 P2 = P3 + (P3-P0)·(x2·4, y2·4)
    ///
    /// 每个分量 `∈ [-0.5, 0.5]`（DragValue/拖拽范围），内部 `*4` 后实际参数范围 `[-2, 2]`。
    /// 直线（退化）：`(0, 0, 0, 0)` — 0 为中性，偏离 0 即弯曲。
    ///
    /// 两个 handle 从各自锚点指出（P1 从 P0，P2 从 P3），符合 CSS 动画编辑器的视觉直觉。
    Curve { x1: f32, y1: f32, x2: f32, y2: f32 },
}

impl Default for SegmentShape {
    /// MIDI 文件导入与未指定时的默认值。Step 与 MIDI CC 原生语义一致。
    fn default() -> Self {
        SegmentShape::Step
    }
}

impl SegmentShape {
    /// 偏移量参数化的放大系数：存储值 `[-0.5, 0.5]` × 4 = 实际参数 `[-2, 2]`。
    pub const SCALE: f32 = 4.0;

    /// 直线 Curve 的默认偏移量：全部为 0（中性）。
    pub const LINEAR_X1: f32 = 0.0;
    pub const LINEAR_Y1: f32 = 0.0;
    pub const LINEAR_X2: f32 = 0.0;
    pub const LINEAR_Y2: f32 = 0.0;

    /// 直线 Curve 的快捷构造。
    pub const fn linear_curve() -> Self {
        SegmentShape::Curve {
            x1: Self::LINEAR_X1,
            y1: Self::LINEAR_Y1,
            x2: Self::LINEAR_X2,
            y2: Self::LINEAR_Y2,
        }
    }

    /// 在归一化进度 `t ∈ [0, 1]` 上计算插值因子 `f ∈ [0, 1]`。
    /// `value_at = v1 + (v2 - v1) * f`。
    ///
    /// 对于 Curve，t 是 tick 进度。三次贝塞尔的参数 u 不等于 t，
    /// 需要从 x(u)=t 反解 u（数值法），再代入 y(u)。
    #[inline]
    pub fn interpolate(self, t: f32) -> f32 {
        debug_assert!((0.0..=1.0).contains(&t), "interpolate t out of range: {t}");
        let t = t.clamp(0.0, 1.0);
        match self {
            SegmentShape::Step => 0.0, // Step: hold v1 until next event; segment value = v1
            SegmentShape::Curve { x1, y1, x2, y2 } => {
                if Self::is_linear_impl(x1, y1, x2, y2) {
                    return t;
                }
                // 实际控制点（归一化空间，P0=(0,0), P3=(1,1)）：
                // P1 = (x1*4, y1*4), P2 = (1+x2*4, 1+y2*4)
                let u = solve_cubic_bezier_u_for_x(t, x1, x2);
                let u1 = 1.0 - u;
                let p1y = y1 * Self::SCALE;
                let p2y = 1.0 + y2 * Self::SCALE;
                3.0 * u1 * u1 * u * p1y + 3.0 * u1 * u * u * p2y + u * u * u
            }
        }
    }

    /// 是否为直线（Curve 且偏移量全部 ≈ 0）。
    #[inline]
    pub fn is_linear(self) -> bool {
        matches!(self, SegmentShape::Curve { x1, y1, x2, y2 }
            if Self::is_linear_impl(x1, y1, x2, y2))
    }

    #[inline]
    fn is_linear_impl(x1: f32, y1: f32, x2: f32, y2: f32) -> bool {
        x1.abs() < 1e-4
            && y1.abs() < 1e-4
            && x2.abs() < 1e-4
            && y2.abs() < 1e-4
    }
}

/// 解三次贝塞尔方程 B_x(u) = t 求 u（Newton 迭代）。
///
/// 偏移量参数化：P1.x = x1·4，P2.x = 1 + x2·4。
/// `B_x(u) = 3(1-u)²u·(x1·4) + 3(1-u)u²·(1+x2·4) + u³`
/// `B_x'(u) = 3(1-u)²·(x1·4) + 6(1-u)u·(1+x2·4 - x1·4) + 3u²·(1 - (1+x2·4))`
///
/// 初值用 u=t（直线时精确）。6 次迭代对 [0,1] 范围足够收敛。
#[inline]
fn solve_cubic_bezier_u_for_x(t: f32, x1: f32, x2: f32) -> f32 {
    let p1x = x1 * SegmentShape::SCALE;
    let p2x = 1.0 + x2 * SegmentShape::SCALE;
    let mut u = t.clamp(0.0, 1.0);
    for _ in 0..6 {
        let u1 = 1.0 - u;
        let f = 3.0 * u1 * u1 * u * p1x + 3.0 * u1 * u * u * p2x + u * u * u - t;
        let df = 3.0 * u1 * u1 * p1x + 6.0 * u1 * u * (p2x - p1x) + 3.0 * u * u * (1.0 - p2x);
        if df.abs() < 1e-6 {
            break;
        }
        u -= f / df;
        u = u.clamp(0.0, 1.0);
    }
    u
}

/// Identifies an automatable parameter.
///
/// This enum is the unified key for all automation data — CC, PitchBend,
/// RPN, NRPN, Tempo, and future VST parameters. Each variant maps to a lane
/// of `(tick, value)` events sorted by tick.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AutomationTarget {
    /// MIDI CC 0–127.
    CC { controller: u8 },
    /// MIDI Pitch Bend (0–16383, center 8192).
    PitchBend,
    /// RPN (Registered Parameter Number), 14-bit parameter address 0–16383.
    Rpn { parameter: u16 },
    /// NRPN (Non-Registered Parameter Number), 14-bit parameter address 0–16383.
    Nrpn { parameter: u16 },
    /// Tempo (BPM). 全局唯一一条 lane，存于 `ConductorData.tempo`。
    /// `value` 直接装 bpm（f32）。`max_value` 仅作 fallback，
    /// panel 层会按实际事件动态计算最大值。
    Tempo,
}

impl AutomationTarget {
    /// Whether this target uses the full 14-bit range (0–16383).
    ///
    /// RPN 0 (Pitch Bend Sensitivity) and RPN 2 (Coarse Tune) are 7-bit
    /// values (0–127). Only RPN 1 (Fine Tune) is 14-bit.
    pub fn is_14bit(&self) -> bool {
        matches!(
            self,
            AutomationTarget::PitchBend
                | AutomationTarget::Rpn { parameter: 1 }
                | AutomationTarget::Nrpn { .. }
        )
    }

    /// Maximum raw value for this target (used to normalize bar heights).
    ///
    /// `Tempo` 返回的 240.0 仅作 fallback；panel 层会按实际事件动态
    /// 计算最大值（Tempo 的实际范围由项目内的事件决定，可能 120 也可能 200）。
    pub fn max_value(&self) -> f32 {
        match self {
            AutomationTarget::CC { .. } => 127.0,
            AutomationTarget::PitchBend => 16383.0,
            AutomationTarget::Rpn { parameter } => match parameter {
                0 => 127.0,    // Pitch Bend Sensitivity (semitones)
                2 => 127.0,    // Coarse Tune (semitones, -64..+63 stored as 0..127)
                _ => 16383.0,  // Fine Tune (14-bit)
            },
            AutomationTarget::Nrpn { .. } => 16383.0,
            AutomationTarget::Tempo => 240.0,
        }
    }

    /// Default / center value (used to draw a reference line).
    pub fn default_value(&self) -> f32 {
        match self {
            AutomationTarget::CC { controller } => match controller {
                10 | 71 | 72 | 73 | 74 => 64.0,
                _ => 0.0,
            },
            AutomationTarget::PitchBend => 8192.0,
            AutomationTarget::Rpn { parameter } => match parameter {
                0 => 2.0,     // Pitch Bend Sensitivity (2 semitones)
                1 => 8192.0,  // Fine Tune (center of 14-bit range)
                _ => 0.0,
            },
            AutomationTarget::Nrpn { .. } => 0.0,
            AutomationTarget::Tempo => 120.0,
        }
    }

    /// Whether this target has a non-zero center (PitchBend, Fine Tune).
    pub fn has_center_line(&self) -> bool {
        matches!(
            self,
            AutomationTarget::PitchBend
                | AutomationTarget::Rpn { parameter: 1 }
                | AutomationTarget::CC { controller: 10 }
                | AutomationTarget::CC { controller: 71 }
                | AutomationTarget::CC { controller: 72 }
                | AutomationTarget::CC { controller: 73 }
                | AutomationTarget::CC { controller: 74 }
        )
    }

    /// 用户在编辑器里新建事件时，本目标默认采用的插值形状。
    ///
    /// - 开关类 CC（Sustain/Sostenuto/Soft/Legato/Portamento）默认 `Step`
    /// - 其他连续量（Volume/Pan/PB/FineTune/Tempo/...）默认 `Curve` 直线（cubic-bezier(0,0,1,1)）
    /// - MIDI 导入时一律使用 `Step`（保留 MIDI 原生语义），见 parser
    pub fn default_shape(&self) -> SegmentShape {
        match self {
            AutomationTarget::CC { controller } => match controller {
                64 | 65 | 66 | 67 | 68 => SegmentShape::Step,
                _ => SegmentShape::linear_curve(),
            },
            AutomationTarget::PitchBend => SegmentShape::linear_curve(),
            AutomationTarget::Rpn { parameter: _ } => SegmentShape::linear_curve(),
            AutomationTarget::Nrpn { parameter: _ } => SegmentShape::linear_curve(),
            AutomationTarget::Tempo => SegmentShape::linear_curve(),
        }
    }

    /// Human-readable display name for the dropdown.
    pub fn display_name(&self) -> String {
        match self {
            AutomationTarget::CC { controller } => {
                let name = cc_name(*controller);
                if name.is_empty() {
                    format!("CC {}", controller)
                } else {
                    format!("CC {} ({})", controller, name)
                }
            }
            AutomationTarget::PitchBend => "Pitch Bend".into(),
            AutomationTarget::Rpn { parameter } => {
                match parameter {
                    0 => "PB Sensitivity (RPN 0)".into(),
                    1 => "Fine Tune (RPN 1)".into(),
                    2 => "Coarse Tune (RPN 2)".into(),
                    _ => format!("RPN {}", parameter),
                }
            }
            AutomationTarget::Nrpn { parameter } => {
                format!("NRPN {}", parameter)
            }
            AutomationTarget::Tempo => "Tempo".into(),
        }
    }
}

/// Common MIDI CC names (standard GM/GS assignments).
fn cc_name(cc: u8) -> &'static str {
    match cc {
        0 => "Bank Select MSB",
        1 => "Mod Wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Portamento Time",
        6 => "Data Entry MSB",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        32 => "Bank Select LSB",
        38 => "Data Entry LSB",
        64 => "Sustain",
        65 => "Portamento",
        66 => "Sostenuto",
        67 => "Soft Pedal",
        68 => "Legato",
        71 => "Resonance",
        72 => "Release",
        73 => "Attack",
        74 => "Cutoff",
        84 => "Portamento Control",
        91 => "Reverb",
        92 => "Tremolo",
        93 => "Chorus",
        94 => "Detune",
        95 => "Phaser",
        100 => "RPN LSB",
        101 => "RPN MSB",
        _ => "",
    }
}

/// A single automation event: a value at a point in time.
///
/// Channel and track are not stored here — they are implied by the
/// owning `AutomationLane` (which mirrors `TrackData`'s per-track design).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AutomationEvent {
    pub tick: u32,
    /// 原始值的浮点表示。CC/PB/RPN 仍装原始整数值（如 CC 64.0 = CC=64），
    /// Tempo 装 bpm（如 120.0）。未来浮点自动化可直接存小数。
    pub value: f32,
    /// 描述"从本事件到下一事件"的插值形状。
    /// 默认 `Step`（保留 MIDI 原生语义），编辑器新建事件时由
    /// `AutomationTarget::default_shape()` 提供更合适的默认。
    #[serde(default)]
    pub shape: SegmentShape,
}

/// A sorted lane of automation events for one parameter on one track.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AutomationLane {
    pub target: AutomationTarget,
    /// Track index (matches `TrackData` position in `YinModel.tracks`).
    pub track: u16,
    /// Events sorted by `tick`.
    pub events: Vec<AutomationEvent>,
}

impl AutomationLane {
    /// Returns a slice of events whose `tick` falls in `[start_tick, end_tick)`.
    ///
    /// Uses binary search since `events` is sorted by tick.
    pub fn events_in_range(&self, start_tick: u32, end_tick: u32) -> &[AutomationEvent] {
        let lo = self.events.partition_point(|e| e.tick < start_tick);
        let hi = self.events.partition_point(|e| e.tick < end_tick);
        &self.events[lo..hi]
    }
}

/// 用户在 automation 面板上的编辑操作。
///
/// 由 automation 面板产生，由 `Document::apply_automation_edits` 应用。
///
/// `target` 字段在所有变体上都存在，让 `apply_automation_edits` 可以直接
/// 根据 target 分派到 `track.automation_lanes` 或 `conductor.tempo`，
/// 不依赖 `lane_idx` 来推断存储位置。
#[derive(Clone, Debug)]
pub enum AutomationEdit {
    /// 添加新事件。如果 lane 不存在会自动创建。
    Add {
        track_idx: u16,
        target: AutomationTarget,
        tick: u32,
        value: f32,
        shape: SegmentShape,
    },
    /// 移动已有事件。
    Move {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        old_tick: u32,
        new_tick: u32,
        new_value: f32,
    },
    /// 切换已有事件的 shape（双击）。
    CycleShape {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
    },
    /// 直接设置已有事件的 shape（用于控制点拖拽）。
    SetShape {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
        shape: SegmentShape,
    },
    /// 删除已有事件。
    Delete {
        track_idx: u16,
        lane_idx: usize,
        target: AutomationTarget,
        tick: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lane(target: AutomationTarget, ticks: &[u32]) -> AutomationLane {
        AutomationLane {
            target,
            track: 0,
            events: ticks
                .iter()
                .map(|&t| AutomationEvent {
                    tick: t,
                    value: 64.0,
                    shape: SegmentShape::Step,
                })
                .collect(),
        }
    }

    #[test]
    fn test_events_in_range() {
        let lane = make_lane(
            AutomationTarget::CC { controller: 7 },
            &[100, 200, 300, 400, 500],
        );
        let slice = lane.events_in_range(150, 450);
        assert_eq!(slice.len(), 3);
        assert_eq!(slice[0].tick, 200);
        assert_eq!(slice[2].tick, 400);
    }

    #[test]
    fn test_events_in_range_empty() {
        let lane = make_lane(AutomationTarget::CC { controller: 7 }, &[100, 200]);
        assert!(lane.events_in_range(300, 400).is_empty());
    }

    #[test]
    fn test_display_names() {
        assert_eq!(
            AutomationTarget::CC { controller: 7 }.display_name(),
            "CC 7 (Volume)"
        );
        assert_eq!(
            AutomationTarget::CC { controller: 99 }.display_name(),
            "CC 99"
        );
        assert_eq!(AutomationTarget::PitchBend.display_name(), "Pitch Bend");
        assert_eq!(
            AutomationTarget::Rpn { parameter: 0 }.display_name(),
            "PB Sensitivity (RPN 0)"
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 1 }.display_name(),
            "Fine Tune (RPN 1)"
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 2 }.display_name(),
            "Coarse Tune (RPN 2)"
        );
        assert_eq!(
            AutomationTarget::Rpn { parameter: 5 }.display_name(),
            "RPN 5"
        );
        assert_eq!(
            AutomationTarget::Nrpn { parameter: 10 }.display_name(),
            "NRPN 10"
        );
    }

    #[test]
    fn test_max_and_default_values() {
        assert_eq!(AutomationTarget::CC { controller: 0 }.max_value(), 127.0);
        assert_eq!(AutomationTarget::CC { controller: 0 }.default_value(), 0.0);
        assert_eq!(AutomationTarget::CC { controller: 10 }.default_value(), 64.0);
        assert_eq!(AutomationTarget::CC { controller: 71 }.default_value(), 64.0);
        assert_eq!(AutomationTarget::CC { controller: 72 }.default_value(), 64.0);
        assert_eq!(AutomationTarget::CC { controller: 73 }.default_value(), 64.0);
        assert_eq!(AutomationTarget::CC { controller: 74 }.default_value(), 64.0);
        assert_eq!(AutomationTarget::PitchBend.max_value(), 16383.0);
        assert_eq!(AutomationTarget::PitchBend.default_value(), 8192.0);
        assert!(AutomationTarget::PitchBend.has_center_line());
        assert!(!AutomationTarget::CC { controller: 0 }.has_center_line());
        assert!(AutomationTarget::CC { controller: 10 }.has_center_line());
        assert!(AutomationTarget::CC { controller: 71 }.has_center_line());
        assert!(!AutomationTarget::CC { controller: 7 }.has_center_line());
        assert_eq!(AutomationTarget::Rpn { parameter: 0 }.max_value(), 127.0);
        assert_eq!(AutomationTarget::Rpn { parameter: 0 }.default_value(), 2.0);
        assert_eq!(AutomationTarget::Rpn { parameter: 1 }.max_value(), 16383.0);
        assert_eq!(AutomationTarget::Rpn { parameter: 1 }.default_value(), 8192.0);
        assert!(AutomationTarget::Rpn { parameter: 1 }.has_center_line());
        assert_eq!(AutomationTarget::Rpn { parameter: 2 }.max_value(), 127.0);
        assert_eq!(AutomationTarget::Rpn { parameter: 2 }.default_value(), 0.0);
        assert!(!AutomationTarget::Rpn { parameter: 2 }.has_center_line());
        assert_eq!(AutomationTarget::Nrpn { parameter: 5 }.max_value(), 16383.0);
        assert_eq!(AutomationTarget::Nrpn { parameter: 5 }.default_value(), 0.0);
    }

    #[test]
    fn test_default_shape_per_target() {
        // 开关类 CC → Step
        for cc in [64u8, 65, 66, 67, 68] {
            assert_eq!(
                AutomationTarget::CC { controller: cc }.default_shape(),
                SegmentShape::Step,
                "CC {cc} should default to Step"
            );
        }
        // 连续量 CC → Curve 直线（cubic-bezier(0,0,1,1)）
        let linear = SegmentShape::linear_curve();
        for cc in [0u8, 1, 7, 10, 11, 71, 74] {
            assert_eq!(
                AutomationTarget::CC { controller: cc }.default_shape(),
                linear,
                "CC {cc} should default to linear Curve"
            );
        }
        // PB / RPN / NRPN → 直线 Curve
        assert_eq!(AutomationTarget::PitchBend.default_shape(), linear);
        assert_eq!(AutomationTarget::Rpn { parameter: 0 }.default_shape(), linear);
        assert_eq!(AutomationTarget::Rpn { parameter: 1 }.default_shape(), linear);
        assert_eq!(AutomationTarget::Nrpn { parameter: 5 }.default_shape(), linear);
    }

    #[test]
    fn test_segment_shape_interpolate_endpoints() {
        // Step 在区间内始终返回 0（值仍为 v1，由调用方处理）
        assert_eq!(SegmentShape::Step.interpolate(0.0), 0.0);
        assert_eq!(SegmentShape::Step.interpolate(0.5), 0.0);
        assert_eq!(SegmentShape::Step.interpolate(1.0), 0.0);

        // 直线 Curve（偏移量全 0）端点和中点
        let lin = SegmentShape::linear_curve();
        assert_eq!(lin.interpolate(0.0), 0.0);
        assert_eq!(lin.interpolate(1.0), 1.0);
        assert!((lin.interpolate(0.5) - 0.5).abs() < 1e-6);

        // 贝塞尔端点：无论控制点位置，端点始终为 0 和 1
        // 偏移量 (x1,y1,x2,y2)：P1=(x1*4,y1*4), P2=(1+x2*4, 1+y2*4)
        assert_eq!(SegmentShape::Curve { x1: 0.1, y1: 0.2, x2: -0.1, y2: -0.2 }.interpolate(0.0), 0.0);
        assert_eq!(SegmentShape::Curve { x1: 0.1, y1: 0.2, x2: -0.1, y2: -0.2 }.interpolate(1.0), 1.0);
        assert_eq!(SegmentShape::Curve { x1: 0.25, y1: -0.5, x2: -0.25, y2: 0.5 }.interpolate(0.0), 0.0);
        assert_eq!(SegmentShape::Curve { x1: 0.25, y1: -0.5, x2: -0.25, y2: 0.5 }.interpolate(1.0), 1.0);
    }

    #[test]
    fn test_segment_shape_bezier_midpoint() {
        // 直线（偏移量全 0）：B_y(0.5) = 0.5
        assert!((SegmentShape::linear_curve().interpolate(0.5) - 0.5).abs() < 1e-6);

        // ease-in-out 近似 CSS cubic-bezier(0.42, 0, 0.58, 1)
        // → 偏移量 (x1=0.42/4, y1=0, x2=(0.58-1)/4, y2=(1-1)/4) = (0.105, 0, -0.105, 0)
        // B_y(0.5) 接近 0.5
        let ease_io = SegmentShape::Curve { x1: 0.105, y1: 0.0, x2: -0.105, y2: 0.0 };
        let v = ease_io.interpolate(0.5);
        assert!((v - 0.5).abs() < 0.02, "ease-in-out mid expected ~0.5, got {v}");

        // 控制点全部偏到 v_end（实际 y1=y2=1）：
        // P1.y = y1*4 = 1 → y1 = 0.25；P2.y = 1 + y2*4 = 1 → y2 = 0
        // B_y(0.5) = 0.375*1 + 0.375*1 + 0.125 = 0.875
        let v_end = SegmentShape::Curve { x1: 0.075, y1: 0.25, x2: -0.075, y2: 0.0 }.interpolate(0.5);
        assert!((v_end - 0.875).abs() < 1e-6, "expected 0.875, got {v_end}");

        // 控制点全部偏到 v_start（实际 y1=y2=0）：
        // P1.y = y1*4 = 0 → y1 = 0；P2.y = 1 + y2*4 = 0 → y2 = -0.25
        // B_y(0.5) = 0.125
        let v_start = SegmentShape::Curve { x1: 0.075, y1: 0.0, x2: -0.075, y2: -0.25 }.interpolate(0.5);
        assert!((v_start - 0.125).abs() < 1e-6, "expected 0.125, got {v_start}");
    }

    #[test]
    fn test_segment_shape_is_linear() {
        assert!(SegmentShape::linear_curve().is_linear());
        assert!(!SegmentShape::Curve { x1: 0.0, y1: 0.1, x2: 0.0, y2: 0.0 }.is_linear());
        assert!(!SegmentShape::Curve { x1: 0.1, y1: 0.0, x2: 0.0, y2: 0.0 }.is_linear());
        assert!(!SegmentShape::Curve { x1: 0.0, y1: 0.0, x2: 0.1, y2: 0.0 }.is_linear());
        assert!(!SegmentShape::Step.is_linear());
    }
}
