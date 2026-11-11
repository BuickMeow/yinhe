//! 音频 Clip 的 warp 映射（M1 后端）。
//!
//! `WarpMap` 把 **clip 内 beat** 映射到 **素材内秒数**；marker 之间分段线性。
//! 自动生成的 identity map 只有两个端点，等价于整段 varispeed；后续 warp
//! marker UI 可以插入更多 marker，播放路径无需改结构。

use serde::{Deserialize, Serialize};

/// 一个 warp marker：clip 内 beat → 素材内秒数。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpMarker {
    /// clip 内 beat 位置（通常从 0 开始）。
    pub beat: f64,
    /// 素材内的采样偏移（秒）。
    pub sample_seconds: f64,
}

/// clip 内 beat ↔ 素材秒数的分段线性映射。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WarpMap {
    /// 按 `beat` 升序；至少两个 marker。
    pub markers: Vec<WarpMarker>,
    /// 由编辑器自动生成（只有两个端点）。用户手动拖过 marker 后为 false。
    #[serde(default)]
    pub auto: bool,
}

impl WarpMap {
    /// 两端点 identity 映射：`beat 0..beat_span` ↔ `source_start..source_start+source_span`。
    pub fn identity(beat_span: f64, source_start: f64, source_span: f64, auto: bool) -> Self {
        Self {
            markers: vec![
                WarpMarker {
                    beat: 0.0,
                    sample_seconds: source_start.max(0.0),
                },
                WarpMarker {
                    beat: beat_span.max(0.0),
                    sample_seconds: (source_start + source_span).max(0.0),
                },
            ],
            auto,
        }
    }

    /// clip 内 beat 总长度。
    pub fn beat_span(&self) -> f64 {
        match (self.markers.first(), self.markers.last()) {
            (Some(a), Some(b)) => (b.beat - a.beat).max(0.0),
            _ => 0.0,
        }
    }

    /// 素材内秒数总跨度。
    pub fn source_span(&self) -> f64 {
        match (self.markers.first(), self.markers.last()) {
            (Some(a), Some(b)) => (b.sample_seconds - a.sample_seconds).max(0.0),
            _ => 0.0,
        }
    }

    /// 映射 clip 内 beat → 素材秒数；越界夹到首尾，marker 之间线性插值。
    pub fn source_seconds_at(&self, beat: f64) -> f64 {
        let Some(first) = self.markers.first() else {
            return 0.0;
        };
        if beat <= first.beat {
            return first.sample_seconds;
        }
        for pair in self.markers.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if beat <= b.beat {
                let span = b.beat - a.beat;
                if span <= f64::EPSILON {
                    return b.sample_seconds;
                }
                let t = (beat - a.beat) / span;
                return a.sample_seconds + (b.sample_seconds - a.sample_seconds) * t;
            }
        }
        self.markers.last().map(|m| m.sample_seconds).unwrap_or(0.0)
    }

    /// 校验：至少两个 marker、beat 严格升序、秒数不越界。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.markers.len() < 2 {
            return Err("warp map needs at least two markers");
        }
        let mut prev = f64::NEG_INFINITY;
        for m in &self.markers {
            if !m.beat.is_finite() || !m.sample_seconds.is_finite() {
                return Err("warp marker is not finite");
            }
            if m.beat <= prev {
                return Err("warp markers must be strictly increasing");
            }
            if m.sample_seconds < 0.0 {
                return Err("warp source seconds must be non-negative");
            }
            prev = m.beat;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_endpoints_and_midpoint() {
        let w = WarpMap::identity(4.0, 1.0, 2.0, true);
        assert!((w.beat_span() - 4.0).abs() < 1e-9);
        assert!((w.source_span() - 2.0).abs() < 1e-9);
        assert!((w.source_seconds_at(0.0) - 1.0).abs() < 1e-9);
        assert!((w.source_seconds_at(2.0) - 2.0).abs() < 1e-9);
        assert!((w.source_seconds_at(4.0) - 3.0).abs() < 1e-9);
        // 越界夹到首尾。
        assert!((w.source_seconds_at(-1.0) - 1.0).abs() < 1e-9);
        assert!((w.source_seconds_at(99.0) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn piecewise_mapping_uses_markers() {
        let w = WarpMap {
            markers: vec![
                WarpMarker {
                    beat: 0.0,
                    sample_seconds: 0.0,
                },
                WarpMarker {
                    beat: 1.0,
                    sample_seconds: 2.0,
                },
                WarpMarker {
                    beat: 2.0,
                    sample_seconds: 3.0,
                },
            ],
            auto: false,
        };
        assert!((w.source_seconds_at(0.5) - 1.0).abs() < 1e-9);
        assert!((w.source_seconds_at(1.5) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn validate_rejects_bad_maps() {
        assert!(WarpMap::identity(4.0, 0.0, 2.0, true).validate().is_ok());
        let mut w = WarpMap::identity(4.0, 0.0, 2.0, true);
        w.markers[1].beat = 0.0;
        assert!(w.validate().is_err());
        w.markers.pop();
        assert!(w.validate().is_err());
    }
}
