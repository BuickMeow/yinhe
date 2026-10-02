//! 钢笔工具路径模型与几何。
//!
//! 一条 [`PenPath`] 由若干锚点构成，相邻锚点之间是三次贝塞尔段（可闭合）。
//! 每个锚点带可选的入/出方向手柄（相对锚点的偏移）。[`PenPath::note_points`]
//! 把曲线映射为钢琴卷帘音符的 `(key, tick)`：曲线每经过一个整数音高行就产生
//! 一个音符；非单调曲线同一行会经过多次，因此允许同一行出现多个音符。

use yinhe_types::MAX_KEY;

/// 逻辑坐标点：`(tick, key)`。
pub type Pt = (f64, f64);

/// 三次贝塞尔求值。
fn cubic(p0: Pt, c0: Pt, c1: Pt, p1: Pt, t: f64) -> Pt {
    let u = 1.0 - t;
    let (uu, tt) = (u * u, t * t);
    let a = uu * u;
    let b = 3.0 * uu * t;
    let c = 3.0 * u * tt;
    let d = tt * t;
    (
        a * p0.0 + b * c0.0 + c * c1.0 + d * p1.0,
        a * p0.1 + b * c0.1 + c * c1.1 + d * p1.1,
    )
}

/// 路径锚点。位置为逻辑坐标；手柄为**相对锚点**的偏移，`None` 表示该侧无手柄。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenAnchor {
    pub tick: f64,
    pub key: f64,
    pub in_handle: Option<Pt>,
    pub out_handle: Option<Pt>,
}

impl PenAnchor {
    pub fn new(tick: f64, key: f64) -> Self {
        Self {
            tick,
            key,
            in_handle: None,
            out_handle: None,
        }
    }

    pub fn pos(&self) -> Pt {
        (self.tick, self.key)
    }

    /// 出方向控制点（绝对坐标）。
    pub fn out_ctrl(&self) -> Pt {
        match self.out_handle {
            Some((dx, dy)) => (self.tick + dx, self.key + dy),
            None => self.pos(),
        }
    }

    /// 入方向控制点（绝对坐标）。
    pub fn in_ctrl(&self) -> Pt {
        match self.in_handle {
            Some((dx, dy)) => (self.tick + dx, self.key + dy),
            None => self.pos(),
        }
    }

    /// 两侧都有手柄时视为「平滑点」。
    pub fn is_smooth(&self) -> bool {
        self.in_handle.is_some() && self.out_handle.is_some()
    }

    /// 整体平移（锚点与手柄一起移动）。
    pub fn translate(&mut self, dtick: f64, dkey: f64) {
        self.tick += dtick;
        self.key += dkey;
    }
}

/// 多锚点贝塞尔路径。
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PenPath {
    pub anchors: Vec<PenAnchor>,
    pub closed: bool,
}

impl PenPath {
    pub fn is_empty(&self) -> bool {
        self.anchors.is_empty()
    }

    pub fn len(&self) -> usize {
        self.anchors.len()
    }

    /// 段数：闭合为 `n`，否则 `n - 1`（不足两点为 0）。
    pub fn segment_count(&self) -> usize {
        let n = self.anchors.len();
        if n < 2 {
            0
        } else if self.closed {
            n
        } else {
            n - 1
        }
    }

    /// 第 `i` 段的控制点 `(p0, c0, c1, p1)`。
    pub fn segment(&self, i: usize) -> Option<(Pt, Pt, Pt, Pt)> {
        if i >= self.segment_count() {
            return None;
        }
        let n = self.anchors.len();
        let a = self.anchors[i];
        let b = self.anchors[(i + 1) % n];
        Some((a.pos(), a.out_ctrl(), b.in_ctrl(), b.pos()))
    }

    /// 折线化。`step_ticks` 为期望的最大时间步长（越小越精细）；音高方向额外
    /// 保证每行至少 4 个采样。返回含首尾的连续点列。
    pub fn flatten(&self, step_ticks: f64) -> Vec<Pt> {
        let segs = self.segment_count();
        if segs == 0 {
            return self
                .anchors
                .first()
                .map(|a| vec![a.pos()])
                .unwrap_or_default();
        }
        let step = step_ticks.max(0.25);
        let mut pts: Vec<Pt> = Vec::new();
        for i in 0..segs {
            let (p0, c0, c1, p1) = self.segment(i).expect("segment in range");
            let dtick = (p1.0 - p0.0).abs();
            let dkey = (p1.1 - p0.1)
                .abs()
                .max((c0.1 - p0.1).abs())
                .max((c1.1 - p0.1).abs());
            let n = ((dtick / step).ceil() as usize)
                .max((dkey * 4.0).ceil() as usize)
                .clamp(8, 2048);
            for j in 0..n {
                pts.push(cubic(p0, c0, c1, p1, j as f64 / n as f64));
            }
        }
        let end = if self.closed {
            self.anchors[0].pos()
        } else {
            self.anchors.last().expect("non-empty").pos()
        };
        pts.push(end);
        pts
    }

    /// 曲线经过的整数音高行 → `(key, tick)`。曲线每与某行相交一次产生一个点，
    /// 非单调曲线同一行会出现多个点。
    pub fn note_points(&self, step_ticks: f64) -> Vec<(u8, f64)> {
        let pts = self.flatten(step_ticks);
        if pts.is_empty() {
            return Vec::new();
        }
        let mut k_lo = f64::INFINITY;
        let mut k_hi = f64::NEG_INFINITY;
        for p in &pts {
            k_lo = k_lo.min(p.1);
            k_hi = k_hi.max(p.1);
        }
        let lo = k_lo.floor().clamp(0.0, MAX_KEY as f64) as i32;
        let hi = k_hi.ceil().clamp(0.0, MAX_KEY as f64) as i32;
        let mut out: Vec<(u8, f64)> = Vec::new();
        for k in lo..=hi {
            let kf = k as f64;
            let mut prev_y: Option<f64> = None;
            for w in pts.windows(2) {
                let (x0, y0) = (w[0].0, w[0].1);
                let (x1, y1) = (w[1].0, w[1].1);
                let d0 = y0 - kf;
                let d1 = y1 - kf;
                if y0 == y1 {
                    // 水平段：只在进入该行时记录一次，避免整段重复。
                    if d0 == 0.0 && prev_y != Some(kf) {
                        push_point(&mut out, k as u8, x0);
                    }
                } else if d0 * d1 <= 0.0 {
                    // 穿越该行（含端点落在行上）；d0==0 → x0，d1==0 → x1。
                    let t = d0 / (d0 - d1);
                    push_point(&mut out, k as u8, x0 + (x1 - x0) * t);
                }
                prev_y = Some(y0);
            }
        }
        out
    }
}

/// 追加一个音符点；与上一个（同 key）点几乎重合时忽略。
fn push_point(out: &mut Vec<(u8, f64)>, key: u8, tick: f64) {
    if let Some(&(k, t)) = out.last()
        && k == key
        && (t - tick).abs() < 1e-6
    {
        return;
    }
    out.push((key, tick));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(pts: &[(f64, f64)]) -> PenPath {
        PenPath {
            anchors: pts.iter().map(|&(t, k)| PenAnchor::new(t, k)).collect(),
            closed: false,
        }
    }

    #[test]
    fn monotonic_straight_line_hits_every_row_once() {
        let p = path(&[(0.0, 60.0), (960.0, 70.0)]);
        let notes = p.note_points(120.0);
        assert_eq!(notes.len(), 11);
        assert_eq!(notes.first().unwrap().0, 60);
        assert_eq!(notes.last().unwrap().0, 70);
        // tick 线性：key 60 → 0，key 70 → 960。
        for &(k, t) in &notes {
            let expect = (k as f64 - 60.0) * 96.0;
            assert!((t - expect).abs() < 1.0, "key {k} tick {t} expect {expect}");
        }
    }

    #[test]
    fn horizontal_line_yields_single_note_at_start() {
        let p = path(&[(0.0, 60.0), (960.0, 60.0)]);
        let notes = p.note_points(120.0);
        assert_eq!(notes, vec![(60, 0.0)]);
    }

    #[test]
    fn smooth_curve_can_hit_same_row_twice() {
        // 一个上凸的平滑段：两端 key 60，中间鼓过 62，应命中 60 两次、62 一次。
        let mut p = path(&[(0.0, 60.0), (960.0, 60.0)]);
        p.anchors[0].out_handle = Some((320.0, 3.0));
        p.anchors[1].in_handle = Some((-320.0, 3.0));
        let notes = p.note_points(60.0);
        let row60: Vec<f64> = notes
            .iter()
            .filter(|(k, _)| *k == 60)
            .map(|(_, t)| *t)
            .collect();
        assert!(row60.len() >= 2, "row 60 hits: {row60:?}");
        assert!(notes.iter().any(|(k, _)| *k == 62));
    }

    #[test]
    fn closed_path_segments_include_closing_segment() {
        let mut p = path(&[(0.0, 60.0), (480.0, 60.0), (480.0, 64.0)]);
        assert_eq!(p.segment_count(), 2);
        p.closed = true;
        assert_eq!(p.segment_count(), 3);
        let pts = p.flatten(120.0);
        assert_eq!(*pts.last().unwrap(), (0.0, 60.0));
    }

    #[test]
    fn flatten_contains_endpoints() {
        let p = path(&[(0.0, 60.0), (240.0, 72.0)]);
        let pts = p.flatten(10.0);
        assert_eq!(pts[0], (0.0, 60.0));
        assert_eq!(*pts.last().unwrap(), (240.0, 72.0));
    }
}
